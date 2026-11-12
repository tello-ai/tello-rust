[English](README.md) | **한국어**

# tello-rust

Tello용 Rust SDK. `Client`는 `/sdk` WebSocket 프로토콜로 대화의 두뇌를 맡습니다.
API 게이트웨이(api-gateway)는 진행 중인 통화에서 상대방이 말한 턴을 실시간으로
넘겨주고, 여러분이 만든 답변은 다시 통화로 전달됩니다. `Api`는 공개 REST
API(`/v1`)용 별도 클라이언트로, 도구·에이전트·크루를 관리하고 목표로 크루를
만들고 통화 초안을 다룹니다([§7](#7-rest-자원-api)).

> 저장소: `tello-rust` · 크레이트: `tello-ai-sdk` · 라이브러리: `tello`

## 1. 설치

```bash
cargo add tello-ai-sdk
cargo add tokio --features macros,rt-multi-thread
```

크레이트 이름은 `tello-ai-sdk`입니다(crates.io의 `tello`는 관계없는 다른
프로젝트가 쓰고 있습니다). 코드에서는 `tello`로 가져옵니다:

```rust
use tello::{Client, Config};
```

Tokio 위에서 동작합니다. `wss://`와 REST API는 ring 암호 공급자와 webpki 루트
인증서를 쓰는 rustls로 동작하므로 OpenSSL도, 프로세스 전역 암호 공급자 설치도
필요 없습니다. stable Rust 1.94, edition 2021로 빌드·테스트했습니다.

## 2. API 키

`Config::new("")`와 `Config::from_env()`는 `TELLO_API_KEY`에서 키를 읽습니다.
URL은 `TELLO_URL`이 있으면 그 값을, 없으면 `ws://localhost:3000/sdk`를 씁니다.
`with_url`로 직접 지정할 수도 있습니다. 여는 타임아웃은 10초, 닫는 타임아웃은
5초입니다(`with_open_timeout`, `with_close_timeout`).

키 인증은 `Client::connect`가 내부에서 끝냅니다. 소켓이 열리면 `auth` 프레임
(`{"event":"auth","data":{"token":"<apiKey>"}}`)을 보내고, 서버가 `auth.ok`로
응답한 뒤에야 반환합니다. `Authorization` 헤더나 query string 토큰은 쓰지
않습니다. 키는 오류 메시지에 들어가지 않고, `Config`와 `Client`의 `Debug`
출력에서도 가려집니다. 게이트웨이가 키를 거부하거나, `4401`로 연결을 닫거나,
여는 타임아웃 안에 `auth.ok`가 오지 않으면 `connect`가 `Error::Authentication`을
반환합니다. 키에 허용 IP 목록이 있고 이 연결의 주소가 그 밖이면 게이트웨이는
`ipNotAllowed`로 답합니다(close `4401`, reason `ip_not_allowed`). 이때 오류의
`code`는 `ipNotAllowed`, `ip`는 게이트웨이가 확인한 주소(확인하지 못했으면
`None`)이고, 같은 곳에서 다시 접속해도 같은 답이 옵니다. 게이트웨이가 키를 아예
확인하지 못하면 `internalError`와 close `1011`로 답하고, `connect`는 이를
`Error::Server`로 반환합니다. 키가 거부된 것이 아니므로 잠시 뒤 다시 접속하면
됩니다. `Client`는 `connect`가 성공해야만 손에 들어오므로, 인증이 끝나기 전에는
어떤 명령도 나갈 수 없습니다.

내부 WebSocket 라이브러리인 tungstenite는 `trace` 레벨에서 프레임 원문을
로그로 남깁니다. 운영 환경에서는 `tungstenite=trace` 로그를 켜지 마세요. 켜면
`auth` 프레임도 함께 기록됩니다.

## 3. 연결 + 통화 시작

```rust
use tello::{Answer, Client, Config, CreateCall, Event};

#[tokio::main]
async fn main() -> Result<(), tello::Error> {
    let (client, mut events) = Client::connect(Config::from_env()).await?;

    // 이벤트는 별도 태스크에서 읽습니다. 연결을 막지 않습니다.
    let answerer = client.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Event::UserTurn(turn) = event {
                let reply = Answer::new(format!("heard: {}", turn.text));
                if answerer.answer(reply).await.is_err() {
                    break;
                }
            }
            // DTMF 전송: answerer.send_dtmf(tello::SendDtmf::new("1234#")).await
        }
    });

    client
        .create_call(CreateCall::new("+821012345678").prompt("예약 확인"))
        .await?;
    let outcome = client.wait_closed().await;
    client.close().await;
    outcome
}
```

`Client`는 복제 비용이 싼 핸들입니다. 어느 복제본이든 같은 연결을 씁니다.
이벤트 스트림은 `connect`가 함께 돌려주므로, 수신 쪽을 손에 쥐기 전에 도착하는
프레임은 없습니다.

## 4. 실시간 턴 이벤트

`Events`는 게이트웨이가 보낸 모든 프레임을 도착 순서대로 담는, 소비자가 하나인
무손실 큐입니다. `events.recv().await`로 읽거나 `futures::Stream`으로 쓰면 됩니다.
연결의 읽기 태스크는 여러분을 기다리지 않습니다. 프레임은 읽을 때까지 쌓이고,
그동안에도 WebSocket은 게이트웨이의 heartbeat ping에 계속 응답합니다. 버리는
프레임이 없으니 연결이 열려 있는 동안에는 계속 읽으세요.

프레임 하나는 `Event` variant 하나가 됩니다. 모든 variant에서 `event.raw()`로
디코딩된 원본 프레임을 얻을 수 있고, 각 payload 구조체의 `raw` 필드에도 들어
있습니다.

| variant | 프레임 `type` | 필드 |
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
| `Event::Error(ErrorEvent)` | `error` | `code`, `message`, `request_id`, `question`, `required_scope`, `ip` |
| `Event::Disconnected` | — | SDK 자체 이벤트. 항상 연결의 마지막 이벤트 |
| `Event::Unknown(Value)` | 그 밖의 값 | 원본 프레임. 이벤트 계약은 추가만 하는 방식입니다 |

통화 스트림 payload에는 `session_id`, `call_id`, `timestamp`도 들어 있습니다.
선택 필드는 `Option`입니다. `status`는 `CallStatus`(`Queued`, `Dialing`,
`Ringing`, `InProgress`, `Transferring`, `Completed`, `NoAnswer`, `Failed`,
`Cancelled`, 이 버전이 모르는 값이면 `Other(String)`)입니다.
`event.is_terminal()`은 그 이벤트가 통화를 끝내는지 알려 줍니다.

`auth.ok`는 `connect`가 내부에서 소비하며 밖으로 내보내지 않습니다.

## 5. 명령

```rust
impl Client {
    pub async fn connect(config: Config) -> Result<(Client, Events), Error>;
    pub async fn create_call(&self, call: CreateCall) -> Result<String, Error>; // 보낸 requestId
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

지정하지 않았거나 `""`로 지정한 선택 필드는 프레임에서 빠집니다. `cancel`은
`{"event":"cancel","data":{}}`를 보냅니다. 그러면 게이트웨이는 `status`가
`cancelled`인 `call.statusChanged`로 통화를 끝내며, 이것이 종료 이벤트입니다
(`call.completed`는 뒤따르지 않습니다). `requestId`는 명령과 응답 프레임을
짝지어 주는 값이며, 멱등성 키가 아닙니다.

`create_call`은 항상 `requestId`를 보냅니다. 넘긴 값이 비어 있지 않으면 그 값을,
아니면 생성한 UUID v4를 쓰고, 보낸 값을 반환합니다. 게이트웨이는 이 명령의 오류
프레임에 그 값을 되돌려 싣고, `wait_closed`는 그것으로 오류가 통화를 끝내는지
판단합니다. 명령마다 `requestId`를 따로 쓰고, `create_call`의 값을 `answer`,
`send_dtmf`, `get_summary`에 다시 쓰지 마세요. 그 값을 되돌려 실은 오류는
대기를 끝냅니다.

`client.wait_closed()`는 현재 통화가 종료 상태(`call.completed` /
`call.noAnswer` / `call.failed`, 또는 `cancelled` 상태)에 이르거나, 이 통화의
`create_call`에 대한 오류가 오거나, 연결이 닫히면 반환합니다. 진행 중인 대기는
그 사이 이벤트 루프가 후속 통화를 시작했더라도 자기 통화가 끝나면 반환합니다.
후속 통화를 기다리려면 `wait_closed`를 다시 호출하세요. cancel-safe이며,
무한정 기다리지 않으려면 `tokio::time::timeout`으로 상한을 거세요.

`client.close()`는 close code 1000을 보내고, 게이트웨이가 핸드셰이크를 마칠
때까지 닫는 타임아웃만큼 기다립니다. 마지막 `Client` 복제본을 drop 하면 close
핸드셰이크 없이 연결이 끊깁니다.

연결 하나에는 활성 통화가 하나뿐입니다. 통화 중에 `create_call`을 또 보내면
`callAlreadyActive`로 거부되고 진행 중인 통화는 계속됩니다. 통화가 막 끝난
직후에는 게이트웨이가 아직 그 통화를 정리하고 있어 다음 `create_call`도
`callAlreadyActive`로 거부될 수 있습니다. 이 통화는 시작되지 않았으므로
`wait_closed`는 `Error::CallAlreadyActive`를 반환하고, 잠시 뒤 재시도하면 됩니다.

## 6. 오류 처리

모든 실패는 `tello::Error` enum 하나로 옵니다. 게이트웨이 오류 프레임은 `code`에
따라 variant로 대응됩니다:

| 게이트웨이 `code` | `Error` variant |
| --- | --- |
| `unauthenticated` | `Authentication` (인증 핸드셰이크. 4401 종료와 `auth.ok` 타임아웃 포함) |
| `ipNotAllowed` | `Authentication` (`code`는 `ipNotAllowed`, `ip` 포함. 인증 핸드셰이크, 4401 종료) |
| `insufficientScope` | `PermissionDenied` (`required_scope` 포함): 키에 `calls:write`(`createCall`·`answer`·`sendDtmf`·`cancel`)나 `calls:read`(`getSummary`) 권한이 없습니다. 그 명령만 거부되고 연결은 유지됩니다 |
| `toRequired` | `Validation` |
| `callIdRequired` | `Validation` |
| `callNotFound` | `Validation` |
| `callNotCompleted` | `Validation` |
| `dtmfDigitsRequired` | `Validation` |
| `dtmfDigitsInvalid` | `Validation` |
| `callAlreadyActive` | `CallAlreadyActive` |
| `noActiveCall` | `NoActiveCall` |
| `callRejected` | `CallRejected` (`question` 포함) |
| `internalError` | `Server` (`auth` 응답이면 close 1011 동반: 잠시 뒤 다시 접속) |
| 그 밖의 코드 | `Server` (코드는 그대로 보존) |

게이트웨이에서 온 오류는 모두 `error.code()`로 코드를 돌려줍니다. **분기는
코드로 하고 메시지로는 하지 마세요.** 메시지는 게이트웨이가 다시 쓸 수 있는
표시용 문자열입니다. SDK가 직접 만든 오류의 `code()`는 `None`입니다.

`create_call`은 통화가 만들어지기 전에 거부될 수도 있습니다. 이 경우
`call.created`도 `callId`도 과금도 없습니다. 게이트웨이는 재시도하지 않으므로
재시도 정책은 호출자 몫입니다.

| 게이트웨이 `code` | `Error` variant | 대응 |
| --- | --- | --- |
| `insufficientCredit` | `CallRefused` | 충전을 안내합니다. 재전송해도 소용없습니다 |
| `concurrentLimitExceeded` | `CallRefused` | 자기 통화가 하나 끝나기를 기다렸다가 재시도합니다 |
| `callerNotVerified` | `CallRefused` | 번호 인증을 안내합니다. 재전송해도 소용없습니다 |
| `noRepresentativeNumber` | `CallRefused` | 발신 번호 설정을 안내합니다. 재전송해도 소용없습니다 |
| `callProviderUnauthorized` | `CallProvider` | 서비스 장애로 보고합니다. 재전송은 도움이 안 됩니다 |
| `callProviderDraining` | `CallProvider` | 나중에 재시도합니다 |
| `callProviderUnavailable` | `CallProvider` | 나중에 재시도합니다 |
| `callSetupFailed` | `CallProvider` | 실패로 보고합니다 |

SDK가 직접 만드는 오류는 네 가지입니다:

| variant | 발생 시점 |
| --- | --- |
| `ConnectionClosed` | 통화 도중 소켓이 닫힘, 또는 닫힌 연결로 명령을 보냄 |
| `SessionReplaced` | 게이트웨이가 4429로 닫음 |
| `Transport` | WebSocket을 열지 못함(DNS, TCP, TLS, HTTP upgrade, 여는 타임아웃) |
| `Config` | API 키를 넘기지 않았고 `TELLO_API_KEY`도 없음 |

명령 오류는 소켓을 닫지 않고 `Event::Error`로 전달됩니다. `wait_closed`를
끝내는 오류는 이 통화의 `create_call` requestId 중 하나를 되돌려 실은 오류뿐입니다.
그래서 거부된 `create_call`(예: `toRequired`, `callRejected`,
`insufficientCredit`)이 멈춘 채 남지 않고, `call.created` 이후 스트림이 실패한
통화도 마찬가지입니다(게이트웨이가 그 실패도 `create_call`에 대한 오류로 보냅니다).
`callAlreadyActive`는 통화를 연 `create_call`에 대한 응답일 때만 대기를 끝냅니다
(게이트웨이가 아직 이전 통화를 정리하는 중이니 잠시 뒤 재시도하세요). 통화 중에
보낸 `create_call`에 대한 응답이면 이벤트로만 전달됩니다. `noActiveCall`은 id가
일치해도 절대 끝내지 않습니다. `answer`, `send_dtmf`, `get_summary`, `cancel`이
실패해도 통화는 끝나지 않으므로, 그 오류는 `Event::Error`로만 전달되고
`wait_closed`는 계속 기다립니다. `wait_closed`가 반환하는 오류:

- 통화 시작 거부, 또는 `call.created` 이후 통화 스트림 실패 → 위 표의 대응 variant
- 통화 도중 연결 끊김 → `Error::ConnectionClosed`
- 다른 연결에 세션을 빼앗김(4429 종료) → `Error::SessionReplaced`

인증 실패는 `connect`가 직접 반환합니다.

오류 이벤트를 처리할 때는 `ErrorEvent::to_error`로 같은 타입의 오류로 바꿔 쓰면
됩니다:

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

WS 수준 ping heartbeat는 게이트웨이가 10초마다 보내며, pong을 놓친 클라이언트는
끊깁니다. SDK의 읽기 태스크는 아무도 `Events`를 읽지 않을 때도 ping에 스스로
응답합니다. 재연결이나 세션 재개 프로토콜은 없습니다. 비정상 종료가 나면
재연결이 필요한 상황으로 보고 통화를 처음부터 다시 시작하세요.

## 7. REST 자원 API

`tello::Api`는 WebSocket `Client`와 별개인 공개 REST API 클라이언트입니다. 같은
서버용 API 키를 `Authorization: Bearer <key>`로 보내고, `Accept:
application/json`, `User-Agent: tello-rust-sdk/<크레이트 버전>`, 본문이 있으면
`Content-Type: application/json`을 붙입니다.

```rust
use std::time::Duration;

use serde_json::json;
use tello::{Api, ApiConfig, ListTools};

#[tokio::main]
async fn main() -> Result<(), tello::Error> {
    // TELLO_API_KEY, 그리고 TELLO_API_URL 또는 https://api.telloai.io/v1 을 읽습니다.
    let api = Api::from_env()?;
    // 직접 지정할 수도 있습니다:
    let _api = Api::new(
        ApiConfig::new("tello_live_xxx")
            .with_api_url("https://api.telloai.io/v1")
            .with_timeout(Duration::from_secs(10)),
    )?;

    let page = api.tools().list(ListTools::new().limit(20)).await?;
    println!("{}", page["data"]);

    let body = json!({"type": "endCall", "name": "통화 종료", "functionName": "hang_up"});
    let tool = api.tools().create(&body, Some("tool-hang-up-1")).await?;
    println!("만든 도구 {}", tool["id"]);
    Ok(())
}
```

`ApiConfig::new(key)`는 `key`를, 그 값이 `""`이면 `TELLO_API_KEY`를 씁니다. 키가
없으면 `Api::new`가 `Error::Config`로 실패합니다. base URL은 `with_api_url`,
없으면 `TELLO_API_URL`, 없으면 `https://api.telloai.io/v1`입니다. `https`여야 하고
(`http`는 `127.0.0.1`·`localhost`·`::1`만), 끝의 `/`는 무시합니다. 조회(list·get)는
30초(`with_timeout`), 모든 쓰기(생성·수정·삭제, 크루 생성, 통화 초안)는
120초(`with_long_timeout`) 뒤 타임아웃됩니다. 키는 오류에도 `Debug`에도 나오지
않습니다. `Api`는 복제 비용이 싼 핸들입니다.

본문과 결과는 `serde_json::Value` JSON 객체라서 이 버전이 모르는 칸도 그대로
남습니다. 경로 id는 퍼센트 인코딩하고, 빈 id는 아무것도 보내지 않고
`Error::InvalidArgument`로 거부합니다.

| 메서드 | HTTP | 성공 | 반환 |
| --- | --- | --- | --- |
| `tools().list(ListTools::new().limit(n).cursor(c))` | `GET /tools?limit&cursor` | 200 | `{"data": [...], "nextCursor": …}` |
| `tools().get(id)` | `GET /tools/{id}` | 200 | 도구 |
| `tools().create(&body, key)` | `POST /tools` | 201 | 도구(`revision` 포함) |
| `tools().update(id, &patch)` | `PATCH /tools/{id}` | 200 | 도구 |
| `tools().delete(id)` | `DELETE /tools/{id}` | 204 | `()` |
| `agents().list()` / `crews().list()` | `GET /agents` / `GET /crews` | 200 | `{"data": [...]}` |
| `agents()` / `crews()` `.get` `.create` `.update` `.delete` | `/agents[/{id}]` / `/crews[/{id}]` | 200 / 201 / 200 / 204 | 에이전트·크루 / `()` |
| `crews().generate(&input, key)` | `POST /crews/generate` | 202 (같은 목표의 작업이면 200) | 작업 `{"id", "status": "running", …}` |
| `crews().generate_and_wait(&input, GenerateAndWait::new())` | generate 후 `GET /crew-generations/{id}` | | 끝난 작업 |
| `crew_generations().get(id)` | `GET /crew-generations/{id}` | 200 | 작업 |
| `call_drafts().create(&body, key)` | `POST /call-drafts` | 201 | 초안 |
| `call_drafts().revise(draft_id, request, key)` | `POST /call-drafts/{id}/revise`, 본문 `{"request": …}` | 200 | **새 id**의 초안 |
| `call_drafts().confirm(draft_id, key)` | `POST /call-drafts/{id}/confirm`, 본문 없음 | 201 | 통화 |

`key`는 선택 `Idempotency-Key`(`Option<&str>`)이며 넘겼을 때만 보냅니다. 같은
요청을 같은 키로 다시 보내면 첫 응답이 재생됩니다. 키는 쉼표 없는 출력 가능한
ASCII 1~255자여야 하고, 아니면 아무것도 보내지 않고 `Error::InvalidArgument`로
실패합니다. `confirm`은 항상 키를 보냅니다. `None`이면 UUID v4를 만듭니다.

표와 다른 성공 상태나 JSON 객체가 아닌 본문은 `invalid_response` 오류입니다.
초안의 모든 상태(`pendingConfirmation`, `needsClarification`, `refused`,
`confirmed`)는 성공이니 `status`로 분기하세요.

`generate_and_wait`는 작업이 `running`이 아닐 때까지 2초(`poll_interval`)마다
조회하고 그 작업을 돌려줍니다. `failed` 작업도 오류가 아니라 그대로 돌려줍니다.
600초(`timeout`)가 지나면 `Error::CrewGenerationTimeout { id, .. }`로 실패하며,
작업은 서버에서 계속 진행됩니다.

> **쓰기는 즉시 운영에 반영됩니다.** 도구·에이전트·크루의 생성·수정·삭제는 한
> 요청에서 저장·배포·운영 반영까지 하고, 다음 통화부터 바로 쓰입니다.
> **`call_drafts().confirm`은 실제 과금되는 전화를 겁니다.** 에이전트에 준비된
> 시나리오가 있으면 `call_drafts().create`도 바로 겁니다(status `confirmed`).
> 통제된 테스트 번호로만 시험하세요.

SDK는 재시도하지 않습니다. 오류 응답은 `Error::Api(ApiError)`이며 `status`(HTTP),
`code`(서버 `error` 원문 snake_case, 예: `tool_not_found`), `message`(표시용),
`data`(봉투의 `data`, 없으면 `None`), `idempotency_key`(보낸 키, 없으면 `None`)를
담습니다. 프록시 페이지처럼 오류 봉투가 없는 응답의 `code`는 `http_<status>`입니다.
응답을 아예 받지 못하면(DNS, TCP, TLS, 연결 끊김, 타임아웃) `Error::ApiConnection`
이고, 여기에도 `idempotency_key()`가 있습니다. **이렇게 실패한 쓰기는 이미
반영됐을 수 있습니다.** get이나 list로 확인한 뒤 같은 키로 다시 보내세요.

```rust
async fn confirm(api: &tello::Api, draft_id: &str) -> Result<(), tello::Error> {
    match api.call_drafts().confirm(draft_id, None).await {
        Ok(call) => println!("발신 중: {}", call["id"]),
        Err(tello::Error::Api(error)) if error.code == "draft_not_found" => {
            eprintln!("없는 초안: {}", error.message);
        }
        Err(tello::Error::Api(error)) => {
            eprintln!("{} {}: {:?}", error.status, error.code, error.data);
        }
        Err(tello::Error::ApiConnection(error)) => {
            // 통화가 이미 걸렸을 수 있습니다. 새 키가 아니라 이 키로 다시 보냅니다.
            let key = error.idempotency_key().unwrap_or_default().to_owned();
            api.call_drafts().confirm(draft_id, Some(&key)).await?;
        }
        Err(other) => return Err(other),
    }
    Ok(())
}
```

대표 코드(전체 목록은 Tello 개발자 문서의 REST 오류 페이지에 있습니다):

| 상태 | `code` | 대응 |
| --- | --- | --- |
| 401 | `unauthenticated` | API 키를 확인합니다 |
| 403 | `insufficient_scope` | 키에 `data.requiredScope` 권한을 줍니다 |
| 403 | `ip_not_allowed` | 키의 허용 IP 밖에서 보낸 요청입니다(`data.ip`) |
| 400 | `invalid_request` / `invalid_tool` / `invalid_agent` / `invalid_crew` | `data.detail`을 보고 본문을 고칩니다 |
| 404 | `tool_not_found` / `agent_not_found` / `crew_not_found` | 이 계정의 id를 씁니다 |
| 422 | `tool_not_publishable` / `agent_not_publishable` / `crew_not_publishable` | 아무것도 저장되지 않았습니다. `data.reason`을 봅니다 |
| 409 | `crew_generation_in_progress` | 다른 목표를 만드는 중입니다. `data.id`가 그 작업입니다 |
| 404 / 409 / 410 | `draft_not_found` / `draft_superseded` / `draft_expired` | 최신 초안을 쓰거나 처음부터 다시 만듭니다 |
| 409 | `idempotency_in_progress` | 같은 키의 요청이 아직 처리 중입니다. 잠시 뒤 같은 키로 확인합니다 |
| 502 | `write_outcome_unknown` | 이미 반영됐을 수 있습니다. 확인하기 전에는 새 키로 다시 만들지 않습니다 |
| 503 | `upstream_unavailable` | 반영되지 않았습니다. 잠시 뒤 다시 보냅니다 |

## 8. 예제

```bash
TELLO_API_KEY=tello_live_xxx TELLO_URL=ws://localhost:3000/sdk \
    cargo run --example basic_call   # 연결, 통화 1건, 각 턴에 응답
```

실제 통화를 겁니다. 먼저 [`examples/basic_call.rs`](examples/basic_call.rs)의
placeholder 수신 번호 `+821012345678`을 통제된 테스트 번호로 바꾸세요. 라이브
자격 증명은 `examples/.env`에 두세요. 이 파일은 git에서 무시되고 패키지에도
들어가지 않습니다.

## 9. 버전 호환성

`tello-ai-sdk 0.1.x`는 Tello WS 프로토콜 `1.0`을 구현합니다
(`tello::PROTOCOL_VERSION`).

전체 프레임 계약은 [`docs/protocol/sdk-ws.v1.md`](docs/protocol/sdk-ws.v1.md)에
있고, [`docs/events/sdk-events.v1.schema.json`](docs/events/sdk-events.v1.schema.json)과
[`docs/errors/errors.v1.json`](docs/errors/errors.v1.json)이 함께 따라옵니다. 이
세 파일은 API 게이트웨이(api-gateway) 구현 옆에 있는 정본 계약에서 복사한
생성물입니다. 읽기는 여기서, 수정은 정본에서 하세요.
