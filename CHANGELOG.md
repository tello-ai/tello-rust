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
- `wait_closed` ends on a terminal event, on an error echoing one of the current
  call's `createCall` requestIds (except `callAlreadyActive` and
  `noActiveCall`), or when the connection closes mid-call. Errors of other
  commands arrive only as events.
- `Error` maps all 19 codes of `errors.v1.json` and exposes the gateway code;
  `ErrorEvent::to_error` turns an error event into the same typed error.
- The read task never waits for the event consumer, so heartbeat pings are
  always answered.
- `wss://` via rustls with the ring provider and webpki roots; no OpenSSL.
