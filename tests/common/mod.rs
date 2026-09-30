//! A scripted stand-in for the turn-provider-gateway `/sdk` endpoint.
//!
//! Every await that depends on the peer is bounded by [`WAIT`], so a broken
//! client fails the test instead of hanging it.
#![allow(dead_code)]

use std::future::Future;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tello::{Client, Config, Event, Events};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderMap;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

pub const API_KEY: &str = "tello_test_secret_key_1234";

/// Upper bound for anything the test expects to happen.
pub const WAIT: Duration = Duration::from_secs(5);

/// Window for asserting that something did not happen. Only used after an
/// ordering sentinel proved the frames before it were already processed.
pub const QUIET: Duration = Duration::from_millis(100);

pub async fn within<F: Future>(what: &str, future: F) -> F::Output {
    tokio::time::timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

pub struct FakeGateway {
    listener: TcpListener,
    url: String,
}

impl FakeGateway {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        Self {
            listener,
            url: format!("ws://{addr}/sdk"),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn config(&self) -> Config {
        Config::new(API_KEY)
            .with_url(self.url.clone())
            .with_open_timeout(WAIT)
            .with_close_timeout(WAIT)
    }

    /// Accepts one WebSocket and records its upgrade request.
    // The callback's error type is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    pub async fn accept(&self) -> ServerConn {
        let (tcp, _) = within("tcp accept", self.listener.accept())
            .await
            .expect("accept");
        let mut upgrade = None;
        let ws = within(
            "ws upgrade",
            tokio_tungstenite::accept_hdr_async(tcp, |request: &Request, response: Response| {
                upgrade = Some(Upgrade {
                    uri: request.uri().to_string(),
                    headers: request.headers().clone(),
                });
                Ok(response)
            }),
        )
        .await
        .expect("ws handshake");
        ServerConn {
            ws,
            upgrade: upgrade.expect("upgrade request recorded"),
        }
    }

    /// Accepts one WebSocket, checks it authenticates first, and confirms it.
    pub async fn accept_authed(&self) -> ServerConn {
        let mut conn = self.accept().await;
        let auth = conn.recv_json().await;
        assert_eq!(auth["event"], "auth", "first frame must be auth: {auth}");
        conn.send_json(json!({ "type": "auth.ok", "version": "1.0", "accountId": "acct-1" }))
            .await;
        conn
    }
}

/// Connects a client to `gateway` and completes the auth handshake.
pub async fn connect(gateway: &FakeGateway) -> (Client, Events, ServerConn) {
    let (client, conn) = tokio::join!(Client::connect(gateway.config()), gateway.accept_authed());
    let (client, events) = client.expect("connect");
    (client, events, conn)
}

pub struct Upgrade {
    pub uri: String,
    pub headers: HeaderMap,
}

pub struct ServerConn {
    pub ws: WebSocketStream<TcpStream>,
    pub upgrade: Upgrade,
}

impl ServerConn {
    /// The next text frame from the client, decoded.
    pub async fn recv_json(&mut self) -> Value {
        loop {
            let message = within("client frame", self.ws.next())
                .await
                .expect("client closed the socket")
                .expect("read client frame");
            match message {
                Message::Text(text) => {
                    return serde_json::from_str(text.as_str()).expect("client sent JSON")
                }
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("expected a text frame, got {other:?}"),
            }
        }
    }

    pub async fn send_json(&mut self, frame: Value) {
        self.ws
            .send(Message::text(frame.to_string()))
            .await
            .expect("send frame");
    }

    pub async fn send(&mut self, message: Message) {
        self.ws.send(message).await.expect("send message");
    }

    /// Sends a close frame without finishing the handshake: the connection
    /// stays open until this value is dropped.
    pub async fn send_close(&mut self, code: u16, reason: &str) {
        self.ws
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::from(code),
                reason: reason.to_owned().into(),
            })))
            .await
            .expect("send close");
    }

    /// Closes like the gateway does: close frame, wait for the client's reply,
    /// then drop the TCP connection. The client may already be gone, so a
    /// failed send is ignored.
    pub async fn close_with(mut self, code: u16, reason: &str) {
        let close = Message::Close(Some(CloseFrame {
            code: CloseCode::from(code),
            reason: reason.to_owned().into(),
        }));
        if self.ws.send(close).await.is_ok() {
            while let Some(Ok(_)) = within("client close reply", self.ws.next()).await {}
        }
    }

    /// Waits for the client's pong to the ping carrying `payload`.
    pub async fn expect_pong(&mut self, payload: &'static [u8]) {
        loop {
            let message = within("pong", self.ws.next())
                .await
                .expect("client closed the socket")
                .expect("read client frame");
            match message {
                Message::Pong(data) if data.as_ref() == payload => return,
                Message::Pong(_) => continue,
                other => panic!("expected a pong, got {other:?}"),
            }
        }
    }

    /// Waits for the client's close frame.
    pub async fn expect_close(&mut self) -> Option<CloseFrame> {
        loop {
            let message = within("close frame", self.ws.next())
                .await
                .expect("socket ended without a close frame")
                .expect("read client frame");
            if let Message::Close(frame) = message {
                return frame;
            }
        }
    }
}

/// The next event, failing the test if none arrives.
pub async fn next_event(events: &mut Events) -> Event {
    within("event", events.recv())
        .await
        .expect("event stream ended")
}

/// Reads events until one satisfies `matches`, returning the ones before it.
pub async fn events_until(events: &mut Events, matches: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = next_event(events).await;
        if matches(&event) {
            return seen;
        }
        seen.push(event);
    }
}

pub fn stream_frame(kind: &str, extra: Value) -> Value {
    let mut frame = json!({
        "type": kind,
        "version": "1.0",
        "sessionId": "sess-1",
        "callId": "call-1",
        "timestamp": "2026-07-08T04:00:00.000Z",
    });
    if let (Some(frame), Value::Object(extra)) = (frame.as_object_mut(), extra) {
        frame.extend(extra);
    }
    frame
}

pub fn error_frame(code: &str, request_id: Option<&str>) -> Value {
    let mut frame = json!({ "type": "error", "version": "1.0", "code": code, "message": format!("{code} message") });
    if let Some(id) = request_id {
        frame["requestId"] = json!(id);
    }
    frame
}
