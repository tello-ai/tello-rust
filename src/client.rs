use std::collections::HashSet;
use std::fmt;
use std::pin::{pin, Pin};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, Stream, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::command::{self, Answer, CreateCall, GetSummary, SendDtmf};
use crate::config::{Config, ENV_API_KEY};
use crate::error::{Error, TransportError, UNAUTHENTICATED};
use crate::event::{self, Event};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Writer = SplitSink<Socket, Message>;
type Reader = SplitStream<Socket>;

const CLOSE_UNAUTHENTICATED: u16 = 4401;
const CLOSE_SESSION_REPLACED: u16 = 4429;

/// Error codes that never end [`Client::wait_closed`], even when they echo a
/// `createCall` of the current call: `callAlreadyActive` means the running
/// call continues, and `noActiveCall` is benign.
const NON_ENDING_CODES: [&str; 2] = ["callAlreadyActive", "noActiveCall"];

/// A connection to the gateway `/sdk` endpoint.
///
/// Cloning is cheap and every clone drives the same connection, so one task
/// can consume [`Events`] while another awaits [`Client::wait_closed`]. When
/// the last clone is dropped the connection is torn down without a close
/// handshake; call [`Client::close`] to close it cleanly.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    writer: tokio::sync::Mutex<Writer>,
    reader: AbortHandle,
    url: String,
    close_timeout: Duration,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.reader.abort();
        self.shared.finish();
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("url", &self.inner.url)
            .field("closed", &self.inner.shared.state().closed)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Opens the WebSocket and authenticates.
    ///
    /// The first frame is `auth` carrying the API key; this returns only after
    /// the gateway answers `auth.ok`, which is consumed and not emitted. An
    /// `unauthenticated` error frame, close code 4401, or no `auth.ok` within
    /// the open timeout all fail with [`Error::Authentication`].
    ///
    /// The returned [`Events`] receives every later frame, in order.
    pub async fn connect(config: Config) -> Result<(Client, Events), Error> {
        if config.api_key().is_empty() {
            return Err(Error::Config(format!(
                "an API key is required: pass one to Config::new or set {ENV_API_KEY}"
            )));
        }
        let socket = open(&config).await?;
        let (mut writer, mut reader) = socket.split();

        let auth = tokio::time::timeout(
            config.open_timeout(),
            authenticate(&mut writer, &mut reader, config.api_key()),
        )
        .await
        .unwrap_or_else(|_| {
            Err(Error::Authentication {
                message: "timed out waiting for auth.ok".to_owned(),
            })
        });
        if let Err(error) = auth {
            // Best effort: the socket is being discarded either way.
            let _closed =
                tokio::time::timeout(config.close_timeout(), writer.send(Message::Close(None)))
                    .await;
            return Err(error);
        }

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::new(events_tx)),
            changed: Notify::new(),
        });
        let reader = tokio::spawn(read_loop(
            reader,
            Arc::clone(&shared),
            config.close_timeout(),
        ))
        .abort_handle();
        let client = Client {
            inner: Arc::new(Inner {
                shared,
                writer: tokio::sync::Mutex::new(writer),
                reader,
                url: config.url().to_owned(),
                close_timeout: config.close_timeout(),
            }),
        };
        Ok((client, Events { rx: events_rx }))
    }

    /// Starts a call and returns the `requestId` it was sent with.
    ///
    /// The frame always carries a `requestId`: the one set on `call` when
    /// non-empty, otherwise a generated UUID v4. The gateway echoes it on this
    /// command's error frames, which is how [`Client::wait_closed`] tells an
    /// error that ends the call from one that answers another command. Do not
    /// reuse it on other commands.
    pub async fn create_call(&self, mut call: CreateCall) -> Result<String, Error> {
        let request_id = call
            .take_request_id()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        {
            let mut state = self.inner.shared.state();
            state.ensure_open()?;
            // A createCall during a live call is refused with
            // callAlreadyActive and the live call continues, so its id joins
            // that call's set instead of starting a new one.
            if !state.call_active {
                state.call_request_ids.clear();
            }
            state.call_request_ids.insert(request_id.clone());
            state.call_active = true;
            state.call_done = false;
            state.call_error = None;
        }
        self.send(command::create_call_frame(call, &request_id))
            .await?;
        Ok(request_id)
    }

    /// Sends your reply to the current user turn. `answer.accepted` confirms
    /// the gateway took it; `agent.turn` confirms it was spoken.
    pub async fn answer(&self, answer: Answer) -> Result<(), Error> {
        self.send(command::answer_frame(answer)).await
    }

    /// Sends DTMF digits into the current call. `dtmf.accepted` confirms it.
    pub async fn send_dtmf(&self, dtmf: SendDtmf) -> Result<(), Error> {
        self.send(command::send_dtmf_frame(dtmf)).await
    }

    /// Cancels the active call. A no-op on the gateway when no call is active.
    /// The call then ends with a `cancelled` status change.
    pub async fn cancel(&self) -> Result<(), Error> {
        self.send(command::cancel_frame()).await
    }

    /// Requests the summary of a completed call. The reply is a
    /// [`Event::CallSummary`], or an [`Event::Error`] echoing its `requestId`.
    pub async fn get_summary(&self, summary: GetSummary) -> Result<(), Error> {
        self.send(command::get_summary_frame(summary)).await
    }

    /// Waits until the current call ends or the connection closes.
    ///
    /// Returns `Ok(())` when the call reaches `call.completed`,
    /// `call.noAnswer`, `call.failed` or a `cancelled` status. Returns the
    /// mapped error when an error frame echoes one of this call's
    /// `createCall` requestIds (a refusal before `call.created`, or a stream
    /// failure after it), except `callAlreadyActive` and `noActiveCall`.
    /// Errors of other commands never end the wait; they only arrive as
    /// [`Event::Error`]. Returns [`Error::ConnectionClosed`] when the socket
    /// closes while a call is active, and [`Error::SessionReplaced`] on close
    /// code 4429.
    ///
    /// Cancel-safe. Bound it with [`tokio::time::timeout`] if you need to.
    pub async fn wait_closed(&self) -> Result<(), Error> {
        let shared = &self.inner.shared;
        loop {
            let mut changed = pin!(shared.changed.notified());
            changed.as_mut().enable();
            {
                let mut state = shared.state();
                if state.closed || state.call_done {
                    if let Some(error) = &state.close_error {
                        return Err(error.clone());
                    }
                    return state.call_error.take().map_or(Ok(()), Err);
                }
            }
            changed.await;
        }
    }

    /// Sends close code 1000 and waits up to the close timeout for the
    /// gateway to finish the handshake. The connection is gone afterwards
    /// either way, and [`Events`] ends with [`Event::Disconnected`].
    pub async fn close(&self) {
        let inner = &self.inner;
        if inner.shared.state().closed {
            return;
        }
        let normal = CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        };
        let finished = tokio::time::timeout(inner.close_timeout, async {
            // A failed send means the socket is already going away; the
            // reader observes that and finishes.
            let _sent = inner
                .writer
                .lock()
                .await
                .send(Message::Close(Some(normal)))
                .await;
            inner.shared.closed().await;
        })
        .await;
        if finished.is_err() {
            inner.reader.abort();
            inner.shared.finish();
        }
    }

    async fn send(&self, frame: String) -> Result<(), Error> {
        let shared = &self.inner.shared;
        shared.state().ensure_open()?;
        let mut writer = self.inner.writer.lock().await;
        writer
            .send(Message::text(frame))
            .await
            .map_err(|error| shared.send_error(&error))
    }
}

/// The events of one connection, in arrival order.
///
/// Single consumer. Nothing is dropped: frames queue until you read them, so
/// keep reading while the connection is open. Ends (`None`) after
/// [`Event::Disconnected`]. Also a [`Stream`].
#[derive(Debug)]
pub struct Events {
    rx: mpsc::UnboundedReceiver<Event>,
}

impl Events {
    /// The next event, or `None` once the connection is gone.
    pub async fn recv(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}

impl Stream for Events {
    type Item = Event;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Event>> {
        self.rx.poll_recv(cx)
    }
}

struct Shared {
    state: Mutex<State>,
    /// Woken after every state change that can end a wait.
    changed: Notify,
}

struct State {
    call_active: bool,
    call_done: bool,
    /// requestIds of the createCall commands sent for the current call. Only
    /// an error echoing one of them ends the wait.
    call_request_ids: HashSet<String>,
    /// Ends the current wait once, then is cleared.
    call_error: Option<Error>,
    /// Sticky connection-level failure.
    close_error: Option<Error>,
    closed: bool,
    /// `None` once the connection is finished, which ends [`Events`].
    events: Option<mpsc::UnboundedSender<Event>>,
}

impl State {
    fn new(events: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            call_active: false,
            call_done: false,
            call_request_ids: HashSet::new(),
            call_error: None,
            close_error: None,
            closed: false,
            events: Some(events),
        }
    }

    fn ensure_open(&self) -> Result<(), Error> {
        if !self.closed {
            return Ok(());
        }
        Err(self
            .close_error
            .clone()
            .unwrap_or_else(|| Error::connection_closed("the client is not connected")))
    }

    fn end_call(&mut self) {
        self.call_active = false;
        self.call_done = true;
    }

    fn emit(&self, event: Event) {
        if let Some(events) = &self.events {
            // The consumer may have dropped Events; keep reading regardless so
            // pongs and wait_closed keep working.
            let _unread = events.send(event);
        }
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // No code path panics while holding the lock, so a poisoned state is
        // still consistent.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn dispatch(&self, raw: Value) {
        if raw.get("type").and_then(Value::as_str) == Some("auth.ok") {
            return;
        }
        let event = event::parse(raw);
        let mut state = self.state();
        match &event {
            Event::Error(error) => {
                if error.code == UNAUTHENTICATED {
                    state.close_error = Some(error.to_error());
                } else if state.call_active
                    && error
                        .request_id
                        .as_ref()
                        .is_some_and(|id| state.call_request_ids.contains(id))
                    && !NON_ENDING_CODES.contains(&error.code.as_str())
                {
                    // This call's createCall failed: refused before
                    // call.created, or its stream failed after it. No terminal
                    // frame follows, so end the wait with the mapped error.
                    state.call_error = Some(error.to_error());
                    state.end_call();
                }
            }
            event if event.is_terminal() => state.end_call(),
            _ => {}
        }
        // State first, then the event: a consumer that sees a terminal event
        // can immediately start the next call.
        state.emit(event);
        drop(state);
        self.changed.notify_waiters();
    }

    fn note_close(&self, frame: Option<&CloseFrame>) {
        let Some(frame) = frame else { return };
        let mut state = self.state();
        if state.close_error.is_some() {
            return;
        }
        let reason = frame.reason.as_str();
        state.close_error = match u16::from(frame.code) {
            CLOSE_UNAUTHENTICATED => Some(Error::Authentication {
                message: or_default(reason, UNAUTHENTICATED),
            }),
            CLOSE_SESSION_REPLACED => Some(Error::SessionReplaced {
                message: or_default(reason, "session replaced"),
            }),
            _ => None,
        };
    }

    /// Marks the connection gone. Idempotent.
    fn finish(&self) {
        let mut state = self.state();
        if state.closed {
            return;
        }
        state.closed = true;
        if state.call_active && state.close_error.is_none() {
            state.close_error = Some(Error::connection_closed(
                "the connection closed before the call ended",
            ));
        }
        state.call_active = false;
        state.call_done = true;
        state.emit(Event::Disconnected);
        state.events = None;
        drop(state);
        self.changed.notify_waiters();
    }

    async fn closed(&self) {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            if self.state().closed {
                return;
            }
            changed.await;
        }
    }

    fn send_error(&self, error: &tungstenite::Error) -> Error {
        self.state()
            .close_error
            .clone()
            .unwrap_or_else(|| Error::connection_closed(error.to_string()))
    }
}

fn or_default(value: &str, default: &str) -> String {
    if value.is_empty() { default } else { value }.to_owned()
}

/// Reads until the socket ends. Never waits on the consumer: events go into
/// an unbounded queue, so tungstenite keeps flushing its queued pongs, which
/// it does at the start of every read.
async fn read_loop(mut reader: Reader, shared: Arc<Shared>, close_timeout: Duration) {
    while let Some(message) = reader.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if let Ok(raw) = serde_json::from_str(text.as_str()) {
                    shared.dispatch(raw);
                }
            }
            Ok(Message::Binary(bytes)) => {
                if let Ok(raw) = serde_json::from_slice(&bytes) {
                    shared.dispatch(raw);
                }
            }
            Ok(Message::Close(frame)) => {
                shared.note_close(frame.as_ref());
                // One more read flushes tungstenite's close reply and waits for
                // the server to drop TCP, bounded in case it never does.
                let _ended = tokio::time::timeout(close_timeout, reader.next()).await;
                break;
            }
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Err(_) => break,
        }
    }
    shared.finish();
}

async fn open(config: &Config) -> Result<Socket, Error> {
    let transport = |error| Error::Transport(TransportError::websocket(error));
    let request = config.url().into_client_request().map_err(transport)?;
    let connector = if request.uri().scheme_str() == Some("wss") {
        Connector::Rustls(tls_config()?)
    } else {
        Connector::Plain
    };
    let connecting =
        tokio_tungstenite::connect_async_tls_with_config(request, None, true, Some(connector));
    match tokio::time::timeout(config.open_timeout(), connecting).await {
        Ok(Ok((socket, _response))) => Ok(socket),
        Ok(Err(error)) => Err(transport(error)),
        Err(_) => Err(Error::Transport(TransportError::open_timeout())),
    }
}

/// rustls with the ring provider and the webpki root set, so `wss://` works
/// without OpenSSL or a process-wide default provider.
fn tls_config() -> Result<Arc<rustls::ClientConfig>, Error> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| Error::Transport(TransportError::tls(error)))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Sends the `auth` frame and waits for the reply. The key only ever goes into
/// that frame; no error built here contains it.
async fn authenticate(
    writer: &mut Writer,
    reader: &mut Reader,
    api_key: &str,
) -> Result<(), Error> {
    writer
        .send(Message::text(command::auth_frame(api_key)))
        .await
        .map_err(|_| Error::connection_closed("failed to send the auth frame"))?;
    loop {
        let reply: Value = match reader.next().await {
            Some(Ok(Message::Text(text))) => {
                serde_json::from_str(text.as_str()).unwrap_or(Value::Null)
            }
            Some(Ok(Message::Binary(bytes))) => {
                serde_json::from_slice(&bytes).unwrap_or(Value::Null)
            }
            Some(Ok(Message::Close(frame))) => return Err(auth_close_error(frame.as_ref())),
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
            Some(Err(_)) | None => {
                return Err(Error::connection_closed(
                    "the connection closed during authentication",
                ))
            }
        };
        return match reply.get("type").and_then(Value::as_str) {
            Some("auth.ok") => Ok(()),
            Some("error") if reply.get("code").and_then(Value::as_str) == Some(UNAUTHENTICATED) => {
                Err(Error::Authentication {
                    message: or_default(
                        reply.get("message").and_then(Value::as_str).unwrap_or(""),
                        UNAUTHENTICATED,
                    ),
                })
            }
            _ => Err(Error::Authentication {
                message: "unexpected authentication response".to_owned(),
            }),
        };
    }
}

fn auth_close_error(frame: Option<&CloseFrame>) -> Error {
    match frame.map(|frame| (u16::from(frame.code), frame.reason.as_str())) {
        Some((CLOSE_UNAUTHENTICATED, reason)) => Error::Authentication {
            message: or_default(reason, UNAUTHENTICATED),
        },
        Some((CLOSE_SESSION_REPLACED, reason)) => Error::SessionReplaced {
            message: or_default(reason, "session replaced"),
        },
        _ => Error::connection_closed("the connection closed during authentication"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_without_an_api_key_is_a_config_error() {
        let config = Config::resolve(String::new(), |_| None);
        let error = Client::connect(config).await.expect_err("no key");
        assert!(matches!(error, Error::Config(_)), "got {error:?}");
    }
}
