# Changelog

## Unreleased

- `tello::Api`: a client of the public REST API (`/v1`), separate from the
  WebSocket `Client`. `tools()`, `agents()` and `crews()` list, get, create,
  update and delete; `crews().generate()` and `crew_generations().get()`
  start and read crew generation jobs, and `crews().generate_and_wait()`
  polls until the job is no longer running; `call_drafts()` creates,
  revises and confirms call drafts. Bodies and results are
  `serde_json::Value` objects.
- `ApiConfig` takes the API key or `TELLO_API_KEY`, and the base URL from
  `TELLO_API_URL` or `https://api.telloai.io/v1` (`https` only; `http` for
  loopback hosts). Reads time out after 30s and writes after 120s. Requests
  carry `User-Agent: tello-rust-sdk/<version>` and are never retried.
- New errors: `Error::Api(ApiError)` with the HTTP status, the server's
  snake_case `code`, `message`, `data` and the idempotency key sent
  (`http_<status>` for non-envelope answers, `invalid_response` for an
  undocumented success status or a body that is not a JSON object);
  `Error::ApiConnection` for transport failures and timeouts, also carrying
  the key; `Error::InvalidArgument` for an empty id or an idempotency key
  that is not 1-255 printable ASCII characters without a comma;
  `Error::CrewGenerationTimeout`. `call_drafts().confirm` generates a UUID
  v4 key when none is given.
- The REST client uses reqwest with the same ring-based rustls config as the
  WebSocket, so no process-wide crypto provider is needed.
- WebSocket: `ipNotAllowed` answering `auth` is `Error::Authentication`, which
  now carries `code` (`unauthenticated` or `ipNotAllowed`) and `ip`
  alongside `message`. `insufficientScope` is the new
  `Error::PermissionDenied` with `required_scope`; `ErrorEvent` gains
  `required_scope` and `ip`. `internalError` answering `auth`, or close
  `1011` during auth, is `Error::Server` instead of `Error::Authentication`.
- The protocol contract copies under `docs/` are synced from the gateway,
  now named the API gateway (api-gateway).

## 0.1.0 (unreleased)

First release. Implements Tello WS protocol `1.0` against the API gateway
(api-gateway, formerly turn-provider-gateway) `/sdk` endpoint, with the same
semantics as tello-go, tello-js, tello-python and tello-java.

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
