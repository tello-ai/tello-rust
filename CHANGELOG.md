# Changelog

## 0.1.0 (unreleased)

First release. Implements Tello WS protocol `1.0` against the turn-provider-gateway
`/sdk` endpoint, with the same semantics as tello-go, tello-js, tello-python and
tello-java.

- `Client::connect` authenticates in-band: the first frame is `auth`, and it
  returns only after `auth.ok`, which is consumed. An `unauthenticated` error
  frame, close code 4401, or no `auth.ok` within the open timeout fail with
  `Error::Authentication`. No credentials go on the upgrade request, and the
  API key is redacted from `Debug` and absent from errors.
- `Config` takes the API key or `TELLO_API_KEY`, and the URL from `TELLO_URL` or
  `ws://localhost:3000/sdk`. Open timeout 10s, close timeout 5s.
- Commands `create_call`, `answer`, `send_dtmf`, `cancel` and `get_summary`.
  `create_call` always sends a `requestId`, generating a UUID v4 when none is
  given, and returns it.
- `Events`: a lossless, single-consumer event queue that is also a `Stream`,
  with a typed variant per frame type, `Disconnected`, and `Unknown` for types
  added later. Every event keeps its raw frame.
- `wait_closed` ends on a terminal event (after `cancel`, the gateway's
  `cancelled` status change), on an error echoing one of the current call's
  `createCall` requestIds, or when the connection closes mid-call. Errors of
  other commands arrive only as events. `noActiveCall` never ends it;
  `callAlreadyActive` ends it only when it answers the `createCall` that
  opened the call, which means the gateway is still finishing the previous
  call and the caller can retry shortly.
- A `create_call` sent during a live call only joins that call's requestIds; it
  no longer resets the pending wait. A `create_call` whose frame cannot be sent
  ends the call it opened with the send error, so no wait is left hanging.
- Behavior change: a wait in progress returns when its own call ends, even if
  the event consumer starts a follow-up call before the waiter runs.
  Previously it could miss that end and keep waiting into the follow-up call.
- `Error` maps all 19 codes of `errors.v1.json` and exposes the gateway code;
  `ErrorEvent::to_error` turns an error event into the same typed error.
- The read task never waits for the event consumer, so heartbeat pings are
  always answered.
- `wss://` via rustls with the ring provider and webpki roots; no OpenSSL.
- The WebSocket upgrade URL carries `sdk=rust`, `version=<crate version>` and
  `protocol=<PROTOCOL_VERSION>` so the gateway can log which client
  connected; values are percent-encoded (`+` as `%2B`). The URL's path and
  other query pairs are kept verbatim; pairs whose form-decoded key is one of
  those three are replaced. The server never rejects a connection over these
  values, and the API key never goes on the URL.
