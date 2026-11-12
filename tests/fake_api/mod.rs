//! A scripted stand-in for the public REST API (`/v1`).
//!
//! Each connection carries one request: the server records it, answers with
//! the next scripted [`Reply`] and closes. Every await that depends on the
//! client is bounded by [`WAIT`].
#![allow(dead_code)]

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tello::{Api, ApiConfig};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

pub const API_KEY: &str = "tello_test_secret_key_1234";

/// Upper bound for anything the test expects to happen.
pub const WAIT: Duration = Duration::from_secs(5);

pub async fn within<F: Future>(what: &str, future: F) -> F::Output {
    tokio::time::timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

/// How the server answers one request.
pub enum Reply {
    /// `status` with a JSON body.
    Json(u16, Value),
    /// `status` with a body of the given content type.
    Body(u16, &'static str, String),
    /// `status` with no body.
    Empty(u16),
    /// The reply, sent after a delay.
    Delayed(Duration, Box<Reply>),
    /// Close the connection without answering.
    Drop,
}

/// One request as the server received it.
#[derive(Debug)]
pub struct Recorded {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    /// Lowercased names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }
}

pub struct FakeApi {
    base: String,
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: mpsc::UnboundedReceiver<Recorded>,
}

impl FakeApi {
    pub async fn start() -> FakeApi {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}/v1", listener.local_addr().expect("addr"));
        let replies = Arc::new(Mutex::new(VecDeque::new()));
        let (sender, requests) = mpsc::unbounded_channel();
        let script = Arc::clone(&replies);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let script = Arc::clone(&script);
                let sender = sender.clone();
                tokio::spawn(serve(stream, script, sender));
            }
        });
        FakeApi {
            base,
            replies,
            requests,
        }
    }

    /// `http://127.0.0.1:<port>/v1`.
    pub fn url(&self) -> &str {
        &self.base
    }

    /// A client of this server with the test key.
    pub fn api(&self) -> Api {
        self.api_with(|config| config)
    }

    pub fn api_with(&self, configure: impl FnOnce(ApiConfig) -> ApiConfig) -> Api {
        Api::new(configure(ApiConfig::new(API_KEY).with_api_url(self.url()))).expect("api")
    }

    /// Queues the answer to the next unanswered request.
    pub fn reply(&self, reply: Reply) {
        self.replies.lock().unwrap().push_back(reply);
    }

    /// The next request the server received.
    pub async fn request(&mut self) -> Recorded {
        within("a request", self.requests.recv())
            .await
            .expect("server stopped")
    }

    /// Fails if the server has received a request not yet taken.
    pub fn assert_no_request(&mut self) {
        if let Ok(request) = self.requests.try_recv() {
            panic!("unexpected request {} {}", request.method, request.target);
        }
    }
}

async fn serve(
    stream: TcpStream,
    script: Arc<Mutex<VecDeque<Reply>>>,
    sender: mpsc::UnboundedSender<Recorded>,
) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).await.is_err() {
        return;
    }
    let _ = sender.send(Recorded {
        method,
        target,
        headers,
        body,
    });
    let reply = script.lock().unwrap().pop_front();
    let mut reply = reply.unwrap_or(Reply::Body(
        599,
        "text/plain",
        "no scripted reply".to_owned(),
    ));
    let mut stream = reader.into_inner();
    loop {
        reply = match reply {
            Reply::Delayed(delay, next) => {
                tokio::time::sleep(delay).await;
                *next
            }
            Reply::Drop => return,
            Reply::Json(status, value) => {
                Reply::Body(status, "application/json", value.to_string())
            }
            Reply::Empty(status) => {
                write(&mut stream, status, None).await;
                return;
            }
            Reply::Body(status, content_type, text) => {
                write(&mut stream, status, Some((content_type, text))).await;
                return;
            }
        };
    }
}

async fn write(stream: &mut TcpStream, status: u16, body: Option<(&str, String)>) {
    let mut response = format!("HTTP/1.1 {status} Status\r\nconnection: close\r\n");
    match &body {
        Some((content_type, text)) => {
            response.push_str(&format!(
                "content-type: {content_type}\r\ncontent-length: {}\r\n\r\n{text}",
                text.len()
            ));
        }
        None => response.push_str("\r\n"),
    }
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
