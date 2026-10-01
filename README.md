**English** | [한국어](README.ko.md)

# tello-rust

Rust WebSocket SDK for the Tello `/sdk` protocol. The SDK is the "conversation
brain": the gateway streams each caller turn from a live phone call, and your
reply is forwarded back into the call.

> repo: `tello-rust` · crate: `tello-ai-sdk` · library: `tello`
>
> Transport is WebSocket only. There is no REST or webhook surface.

## 1. Install

```bash
cargo add tello-ai-sdk
cargo add tokio --features macros,rt-multi-thread
```

The crate is published as `tello-ai-sdk` (the `tello` name on crates.io belongs
to an unrelated project) and imported as `tello`:

```rust
use tello::{Client, Config};
```

It runs on Tokio. `wss://` uses rustls with the webpki root certificates, so no
OpenSSL is needed. Built and tested with stable Rust 1.94, edition 2021.

## 2. API key

`Config::new("")` and `Config::from_env()` read `TELLO_API_KEY`. The URL is
`TELLO_URL` when set, otherwise `ws://localhost:3000/sdk`; `with_url` overrides
both. The open timeout is 10s and the close timeout 5s
(`with_open_timeout`, `with_close_timeout`). The upgrade request adds
`sdk=rust&version=<crate version>&protocol=<PROTOCOL_VERSION>` (values
percent-encoded, so a `+` goes as `%2B`) to the end of the URL's query so the
gateway can log which client connected; the server never rejects on these
values. The path and your other query pairs are kept exactly as written; only
empty pairs and pairs whose form-decoded key is `sdk`, `version` or
`protocol` (including `%73dk`) are dropped.

`Client::connect` authenticates the API key internally: after the socket opens
it sends an `auth` frame (`{"event":"auth","data":{"token":"<apiKey>"}}`) and
returns only once the server confirms with `auth.ok`. No `Authorization` header
or query-string token is used. The key never appears in errors, and `Debug` for
`Config` and `Client` prints it redacted. `connect` returns
`Error::Authentication` if the gateway refuses the key, closes with code `4401`,
or `auth.ok` does not arrive within the open timeout. No command can run before
authentication completes, because you only get a `Client` from a successful
`connect`.

tungstenite, the WebSocket library underneath, logs raw frame payloads at the
`trace` level. Keep `tungstenite=trace` logging off in production, or the auth
frame is logged with the rest.

## 3. Connect + start a call

```rust
use tello::{Answer, Client, Config, CreateCall, Event};

#[tokio::main]
async fn main() -> Result<(), tello::Error> {
    let (client, mut events) = Client::connect(Config::from_env()).await?;

    // Consume events on their own task; it never blocks the connection.
    let answerer = client.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Event::UserTurn(turn) = event {
                let reply = Answer::new(format!("heard: {}", turn.text));
                if answerer.answer(reply).await.is_err() {
                    break;
                }
            }
            // Send DTMF digits: answerer.send_dtmf(tello::SendDtmf::new("1234#")).await
        }
    });

    client
        .create_call(CreateCall::new("+821012345678").prompt("reservation check"))
        .await?;
    let outcome = client.wait_closed().await;
    client.close().await;
    outcome
}
```

`Client` is a cheap, cloneable handle: every clone drives the same connection.
`connect` returns the event stream with it, so no frame can arrive before you
hold the receiver.

## 4. Realtime turn events

`Events` is a single-consumer, lossless queue of everything the gateway sends,
in arrival order. Read it with `events.recv().await`, or as a
`futures::Stream`. The connection's read task never waits for you: frames queue
until you read them, and the WebSocket keeps answering the gateway's heartbeat
pings meanwhile. Keep reading while the connection is open, since nothing is
dropped.

Each frame becomes one `Event` variant. `event.raw()` returns the decoded frame
for every variant, and each payload struct also keeps it in its `raw` field.

| variant | frame `type` | fields |
| --- | --- | --- |
| `Event::CallCreated(CallCreated)` | `call.created` | `session_id`, `call_id`, `timestamp` |
| `Event::CallStatusChanged(CallStatusChanged)` | `call.statusChanged` | `status`, `previous_status` |
| `Event::UserTurn(Turn)` | `user.turn` | `turn_index`, `text` |
| `Event::AgentTurn(Turn)` | `agent.turn` | `turn_index`, `text` |
| `Event::AnswerAccepted(AnswerAccepted)` | `answer.accepted` | `request_id`, `message_id` |
| `Event::DtmfAccepted(DtmfAccepted)` | `dtmf.accepted` | `request_id`, `message_id`, `digits` |
| `Event::CallCompleted(CallEnded)` | `call.completed` | `status` |
| `Event::CallNoAnswer(CallEnded)` | `call.noAnswer` | `status`, `failure_reason` |
| `Event::CallFailed(CallEnded)` | `call.failed` | `status`, `failure_reason` |
| `Event::CallSummary(CallSummary)` | `call.summary` | `request_id`, `call_id`, `status`, `duration_seconds`, `transcript`, `summary`, `credit_charged` |
| `Event::Error(ErrorEvent)` | `error` | `code`, `message`, `request_id`, `question` |
| `Event::Disconnected` | — | SDK-local; always the last event of a connection |
| `Event::Unknown(Value)` | anything else | the raw frame; the event contract is additive |

Call-stream payloads also carry `session_id`, `call_id` and `timestamp`.
Optional fields are `Option`s. `status` is a `CallStatus` (`Queued`, `Dialing`,
`Ringing`, `InProgress`, `Transferring`, `Completed`, `NoAnswer`, `Failed`,
`Cancelled`, or `Other(String)` for a value this version does not know).
`event.is_terminal()` tells whether an event ends the call.

`auth.ok` is consumed internally by `connect` and never emitted.

## 5. Commands

```rust
impl Client {
    pub async fn connect(config: Config) -> Result<(Client, Events), Error>;
    pub async fn create_call(&self, call: CreateCall) -> Result<String, Error>; // the requestId sent
    pub async fn answer(&self, answer: Answer) -> Result<(), Error>;
    pub async fn send_dtmf(&self, dtmf: SendDtmf) -> Result<(), Error>;
    pub async fn cancel(&self) -> Result<(), Error>;
    pub async fn get_summary(&self, summary: GetSummary) -> Result<(), Error>;
    pub async fn wait_closed(&self) -> Result<(), Error>;
    pub async fn close(&self);
}

CreateCall::new(to).prompt(p).metadata(json).request_id(id)
Answer::new(text).message_id(id).request_id(id)
SendDtmf::new(digits).message_id(id).request_id(id)
GetSummary::new(call_id).request_id(id)
```

Optional fields you do not set, or set to `""`, are left out of the frame.
`cancel` sends `{"event":"cancel","data":{}}`; the gateway then ends the call
with `call.statusChanged` status `cancelled`, which is the terminal event (no
`call.completed` follows). A `requestId` correlates a command with its response
frame; it is not an idempotency key.

`create_call` always sends a `requestId`: yours when non-empty, otherwise a
generated UUID v4. It returns the id it sent. The gateway echoes that id on the
error frames of this command, which is how `wait_closed` knows an error ends
the call. Give each command its own `requestId` and never reuse the
`create_call` one on `answer`, `send_dtmf` or `get_summary`: an error echoing it
ends the wait.

`client.wait_closed()` returns when the current call reaches a terminal state
(`call.completed` / `call.noAnswer` / `call.failed`, or a `cancelled` status),
when an error answers this call's `create_call`, or when the connection closes.
A wait in progress returns when its own call ends, even if your event loop has
already started a follow-up call by then; call `wait_closed` again to wait for
the follow-up. It is cancel-safe; bound it with `tokio::time::timeout` if you
need to.

`client.close()` sends close code 1000 and waits up to the close timeout for
the gateway to finish the handshake. Dropping the last `Client` clone tears the
connection down without a close handshake.

One connection carries one active call at a time. A second `create_call` during
a live call is refused with `callAlreadyActive` and the live call continues.
Right after a call ends the gateway may still be finishing it, so the next
`create_call` can also be refused with `callAlreadyActive`; that call never
started, `wait_closed` returns `Error::CallAlreadyActive`, and you can retry
shortly.

## 6. Error handling

Every failure is one `tello::Error` enum. Gateway error frames map to variants
by `code`:

| gateway `code` | `Error` variant |
| --- | --- |
| `unauthenticated` | `Authentication` (auth handshake; also close code 4401 and the `auth.ok` timeout) |
| `toRequired` | `Validation` |
| `callIdRequired` | `Validation` |
| `callNotFound` | `Validation` |
| `callNotCompleted` | `Validation` |
| `dtmfDigitsRequired` | `Validation` |
| `dtmfDigitsInvalid` | `Validation` |
| `callAlreadyActive` | `CallAlreadyActive` |
| `noActiveCall` | `NoActiveCall` |
| `callRejected` | `CallRejected` (with `question`) |
| `internalError` | `Server` |
| any other code | `Server`, keeping the code |

Every gateway-derived error exposes its code through `error.code()` — branch
on that, never on the message, which is display text the gateway may reword.
`code()` is `None` for errors the SDK raises itself.

`create_call` can also be refused before any call exists — no `call.created`,
no `callId`, no charge. The gateway never retries these; any retry policy is
yours.

| gateway `code` | `Error` variant | what to do |
| --- | --- | --- |
| `insufficientCredit` | `CallRefused` | tell the user to top up; do not resend |
| `concurrentLimitExceeded` | `CallRefused` | wait for one of your own calls to end, then retry |
| `callerNotVerified` | `CallRefused` | tell the user to verify the number; do not resend |
| `noRepresentativeNumber` | `CallRefused` | tell the user to configure a caller number; do not resend |
| `callProviderUnauthorized` | `CallProvider` | service fault; report it, resending never helps |
| `callProviderDraining` | `CallProvider` | retry later at your own pace |
| `callProviderUnavailable` | `CallProvider` | retry later at your own pace |
| `callSetupFailed` | `CallProvider` | surface as a failure and report it |

The SDK raises four more:

| variant | when |
| --- | --- |
| `ConnectionClosed` | the socket closed while a call was active, or a command was sent on a closed connection |
| `SessionReplaced` | the gateway closed with 4429 |
| `Transport` | the WebSocket could not be opened (DNS, TCP, TLS, HTTP upgrade, open timeout) |
| `Config` | no API key was given and `TELLO_API_KEY` is unset |

Command errors never close the socket; they arrive as `Event::Error`. Only an
error that echoes one of this call's `create_call` requestIds ends
`wait_closed`, so a refused `create_call` (e.g. `toRequired`, `callRejected`,
`insufficientCredit`) does not hang, and neither does a call whose stream fails
after `call.created` (the gateway reports that against `create_call` too).
`callAlreadyActive` ends it only when it answers the `create_call` that opened
the call (the gateway is still finishing the previous call, so retry shortly);
answering a `create_call` sent during a live call it is only an event.
`noActiveCall` never ends it, even with a matching id. A failed `answer`,
`send_dtmf`, `get_summary` or `cancel` does not end the call: its error is
delivered only as an `Event::Error` and `wait_closed` keeps waiting.
`wait_closed` returns:

- a call-start refusal, or a failure of the call's stream after `call.created` → its mapped variant above
- the connection dropping mid-call → `Error::ConnectionClosed`
- the session being displaced (close 4429) → `Error::SessionReplaced`

Authentication failures are returned by `connect` itself.

To act on an error event, turn it into the same typed error with
`ErrorEvent::to_error`:

```rust
if let Event::Error(error_event) = event {
    match error_event.to_error() {
        tello::Error::CallRejected { message, question, .. } => {
            eprintln!("call rejected: {message} ({question:?})");
        }
        other => eprintln!("{} failed: {other}", error_event.request_id.unwrap_or_default()),
    }
}
```

The gateway drives a WS-level ping heartbeat every 10s and drops a client that
misses a pong; the SDK's read task answers pings on its own, even while nobody
reads `Events`. There is no reconnect/resume — treat an abnormal close as
reconnect-worthy and restart the call.

## 7. Examples

```bash
TELLO_API_KEY=tello_live_xxx TELLO_URL=ws://localhost:3000/sdk \
    cargo run --example basic_call   # connect, one call, answer each turn
```

It places a real call. Replace the placeholder recipient `+821012345678` in
[`examples/basic_call.rs`](examples/basic_call.rs) with a controlled test
number first. Keep live credentials in `examples/.env`, which is git-ignored and
not packaged.

## 8. Version compatibility

`tello-ai-sdk 0.1.x` implements Tello WS protocol `1.0` (`tello::PROTOCOL_VERSION`).

The full frame contract is in [`docs/protocol/sdk-ws.v1.md`](docs/protocol/sdk-ws.v1.md),
with [`docs/events/sdk-events.v1.schema.json`](docs/events/sdk-events.v1.schema.json)
and [`docs/errors/errors.v1.json`](docs/errors/errors.v1.json). Those three files
are generated copies of the canonical contract that lives beside the gateway
implementation — read them here, edit them there.
