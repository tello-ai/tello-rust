# Tello SDK WebSocket 프로토콜 계약 v1

> version: 1.0
> 대상: turn-provider-gateway `/sdk` 엔드포인트
> 상태: **source of truth**. 프레임을 실제로 만드는 코드와 같은 저장소에 둔다.
> 구현: `src/infrastructure/ws/sdk-protocol.ts` · `sdk-events.ts` · `ws-call.gateway.ts`
> 함께 보기: [`../events/sdk-events.v1.schema.json`](../events/sdk-events.v1.schema.json) · [`../errors/errors.v1.json`](../errors/errors.v1.json)

모든 언어 SDK(`tello-python` / `tello-js` / `tello-go` / `tello-java`)는 이 문서를 기준으로 구현한다.
각 SDK 저장소의 `docs/` 사본은 이 문서에서 복사한 생성물이며 직접 편집하지 않는다.

SDK는 이 WebSocket 하나로 통화를 시작하고, 상대방 발화(turn)를 받아 응답한다. REST/webhook은 이 프로토콜에 포함되지 않는다.

## 1. 엔드포인트 / 연결

```text
ws(s)://<host>:<port>/sdk
```

- 기본 포트 3000. WebSocket 서브프로토콜 협상 없음.
- 한 연결당 활성 통화는 하나다.

### 1.1 클라이언트 식별 쿼리 (선택)

업그레이드 URL 에 클라이언트가 자신을 밝히는 쿼리 3개를 붙일 수 있다.

```text
wss://api.telloai.io/sdk?sdk=js&version=0.1.1&protocol=1.0
```

| 쿼리 | 뜻 | 공식 SDK 값 |
| --- | --- | --- |
| `sdk` | SDK 식별자 | `js` · `python` · `go` · `java` · `rust` |
| `version` | SDK 패키지 배포 버전 (`v` 접두사 없음) | 예: `0.1.1` |
| `protocol` | SDK 의 `PROTOCOL_VERSION` | `1.0` |

- 전부 선택 항목이며 진단 로그(`sdk.connection.established` · `sdk.connection.rejected` · `sdk.command.create_call` 의 `client` 필드)에만 쓴다. **연결 거절이나 동작 분기에 쓰지 않는다.**
- trim 후 `^[A-Za-z0-9._+-]{1,32}$` 에 맞지 않거나 없으면 무시하고 `"unknown"` 으로 기록한다.
- API 키는 URL 에 싣지 않는다. 인증은 §2 의 `auth` 프레임으로만 한다.

## 2. 인증 (애플리케이션 핸드셰이크)

인증은 HTTP upgrade 헤더나 쿼리 토큰이 아니라 **애플리케이션 프레임**으로 이뤄진다.
소켓은 미인증 상태로 연결되며, upgrade 요청에서 어떤 자격 증명도 읽지 않는다.
API key는 upgrade 요청, URL 쿼리, 로그, 예외 메시지 어디에도 노출되지 않는다.

1. 소켓이 열린 뒤 클라이언트가 보내는 **첫 프레임**은 반드시 `auth`다. raw API key는 `token` 필드로 보낸다.

   ```json
   { "event": "auth", "data": { "token": "<TELLO_API_KEY>", "requestId": "<optional>" } }
   ```

2. 서버가 `auth.ok`를 보내기 전에는 다른 어떤 명령도 보내면 안 된다. 그 전에 온 명령은
   전부 `unauthenticated` error 프레임으로 거부된다.

   ```json
   { "type": "auth.ok", "version": "1.0", "accountId": "<accountId>", "requestId": "<echoed when supplied>" }
   ```

3. `auth.ok` 이후에만 `createCall` / `answer` / `sendDtmf` / `cancel` / `getSummary`를 보낼 수 있다.

인증에 실패하면 서버는 `error` 프레임(`code: "unauthenticated"`)을 보내고 close code `4401`로
연결을 종료한다. **`auth` 프레임이 10초(`AUTH_TIMEOUT_MS`) 안에 오지 않아도 같은 코드로 닫는다.**

SDK는 세 경우를 모두 `AuthenticationError`(언어별 대응 클래스)로 올린다.

- `unauthenticated` error 프레임 수신
- close code `4401`
- 클라이언트 쪽 `auth.ok` 대기 타임아웃

## 3. 프레임 방향 비대칭 (중요)

- **아웃바운드(client → server)**: NestJS `@nestjs/platform-ws` 라우팅 봉투를 쓴다.

  ```json
  { "event": "<command>", "data": { "...": "..." } }
  ```

- **인바운드(server → client)**: 봉투 없이 **flat** 프레임이다. `type` 필드로 디스패치한다.

  ```json
  { "type": "<event>", "version": "1.0", "sessionId": "...", "callId": "...", "timestamp": "..." }
  ```

  `auth.ok` · `call.summary` · `error` 세 프레임은 통화 스트림 이벤트가 아니라 명령 응답이라
  `sessionId` / `timestamp`를 싣지 않는다. 자세한 필드는 스키마를 따른다.

## 4. 명령 (client → server)

`data` 안의 필드다. 모든 명령은 선택적 `requestId`를 가질 수 있고, 응답·에러 프레임의
`requestId`로 에코된다. **`requestId`는 명령과 응답을 짝지어 주는 값이며 멱등성 키가 아니다.**

### 4.1 `createCall`

```json
{ "event": "createCall", "data": {
  "to": "+821012345678",          // 필수. 전화할 대상 번호. 비면 error: toRequired
  "prompt": "예약 확인",           // 선택, 기본 ""
  "metadata": { "any": "json" },  // 선택
  "requestId": "req-1"            // 선택
}}
```

이미 활성 통화가 있으면 error `callAlreadyActive`. 직전 통화의 종단 이벤트를 보낸 뒤에도
게이트웨이가 그 통화의 정리를 마칠 때까지(짧은 구간) 세션을 붙잡고 있으므로, 그 사이에 온
`createCall`도 `callAlreadyActive`를 받는다. 이때는 `call.created`가 오지 않으니 잠시 뒤 다시 보낸다.

에이전트는 서버가 정한다. `agentId`는 명령에 없으며, 옛 클라이언트가 보내도 무시하고
Voice Gateway 요청에도 싣지 않는다.

발신 게이트가 통화를 거부하면 `call.created` 없이 error 프레임 하나만 오고 세션이 끝난다.
통화가 만들어지지 않았으므로 `callId`도 과금도 없다. 코드는 §6의 `insufficientCredit` 이하 8종이다.

### 4.2 `answer`

```json
{ "event": "answer", "data": {
  "text": "확인했습니다.",         // 선택, 기본 ""
  "messageId": "m1",             // 선택, 기본 서버 생성 UUID
  "requestId": "req-2"           // 선택
}}
```

활성 통화가 없으면 error `noActiveCall`. 성공하면 `requestId`(제공한 경우)와 유효 `messageId`를
담은 `answer.accepted`가 먼저 온다. 이건 **명령이 검증되어 Voice Gateway로 제출됐다는 ACK일 뿐**
실제 발화 보장이 아니다. 텍스트가 실제로 말해지면 그때 `agent.turn`이 온다. 어시스턴트
타임아웃으로 Voice Gateway가 그 턴을 버리면 `agent.turn`은 오지 않는다.

### 4.3 `sendDtmf`

```json
{ "event": "sendDtmf", "data": {
  "digits": "1234#",             // 필수. 보낼 DTMF 다이얼 문자열
  "messageId": "m1",             // 선택, 기본 서버 생성 UUID
  "requestId": "r1"              // 선택
}}
```

`answer`를 미러링하는 명령이며, `text` 대신 `digits`를 보낸다. `digits`는 키패드
문자 `0-9`, `*`, `#`만 허용한다. 활성 통화가 없으면 error `noActiveCall`, `digits`가
비면 `dtmfDigitsRequired`, 허용 문자 외가 섞이면 `dtmfDigitsInvalid`.

톤은 발화가 아니므로 `agent.turn`이 따라오지 않는다. `dtmf.accepted`로 확인한다.

### 4.4 `cancel`

```json
{ "event": "cancel", "data": {} }
```

활성 통화가 없으면 무시(no-op). 취소가 반영되면 `call.statusChanged`가 `status: "cancelled"`로 오고,
`previousStatus`는 취소 직전의 상태다. 이 프레임이 그 통화의 종단 이벤트다.

### 4.5 `getSummary`

```json
{ "event": "getSummary", "data": {
  "callId": "call-1",            // 필수. 비면 error: callIdRequired
  "requestId": "r2"              // 선택
}}
```

인증된 계정이 소유한 **완료된** 통화의 요약을 조회한다. 진행 중인 통화가 필요 없으므로
통화가 끝난 뒤에도 호출할 수 있다. 대상 통화가 없으면 error `callNotFound`, 아직
`completed`가 아니면 `callNotCompleted`. 성공하면 `call.summary` 프레임이 온다.

## 5. 이벤트 (server → client)

스키마: [`../events/sdk-events.v1.schema.json`](../events/sdk-events.v1.schema.json).
통화 스트림 이벤트의 공통 필드는 `type`, `version`("1.0"), `sessionId`, `callId`,
`timestamp`(ISO-8601). 필드는 camelCase.

| type | 추가 필드 | 의미 |
| --- | --- | --- |
| `auth.ok` | `accountId`, `requestId?` | 인증 성공. 공통 필드 없음. SDK가 `connect()` 안에서 소비하고 재방출하지 않는다 |
| `call.created` | — | createCall 직후 첫 프레임. 공통 `callId`로 통화 id를 즉시 전달. 초기 상태 `queued`를 뜻한다 |
| `call.statusChanged` | `status`, `previousStatus` | 통화 상태 전이. 취소도 이 이벤트(status `"cancelled"`)로 온다 |
| `user.turn` | `turnIndex`, `text` | 상대방 발화. SDK가 응답할 차례 |
| `answer.accepted` | `requestId?`, `messageId` | answer 명령이 검증되어 Voice Gateway로 제출됨. 실제 발화는 후속 `agent.turn`으로 확인 |
| `dtmf.accepted` | `requestId?`, `messageId`, `digits` | sendDtmf 명령이 검증되어 Voice Gateway로 제출됨 |
| `agent.turn` | `turnIndex`, `text` | SDK 답변이 실제로 통화에서 발화됨 |
| `call.completed` | `status` | 종단: 정상 완료 |
| `call.noAnswer` | `status`, `failureReason?` | 종단: 무응답 |
| `call.failed` | `status`, `failureReason?` | 종단: 실패 |
| `call.summary` | `requestId?`, `callId`, `status`, `durationSeconds`, `transcript`, `summary`, `creditCharged` | getSummary 응답. `sessionId`·`timestamp` 없음. 뒤쪽 네 필드는 null 가능 |
| `error` | `code`, `message`, `requestId?`, `question?` | 명령 실패. `sessionId`·`callId`·`timestamp` 없음 |

status 어휘: `queued`, `dialing`, `ringing`, `inProgress`, `transferring`, `completed`, `noAnswer`, `failed`, `cancelled`.

`call.created`가 초기 `queued`를 나타내므로, `call.statusChanged`는 상태가 `queued`에서
바뀐 뒤부터 온다. 진행 상태는 직전 발행 상태와 중복 제거되며, Voice Gateway가 볼 때 이미
응답된 통화(SIP 인바운드, WebRTC 테스트 통화)는 `inProgress`로 바로 건너뛴다.
**`dialing`·`ringing`이 반드시 온다고 가정하면 안 된다.**

종단 이벤트(`call.completed` / `call.noAnswer` / `call.failed`, 또는 `cancelled` statusChanged)
이후 서버는 해당 통화 스트림을 종료한다.

## 6. 에러 프레임

스키마: [`../errors/errors.v1.json`](../errors/errors.v1.json). 별도 봉투 없이 flat이다.

```json
{ "type": "error", "version": "1.0", "code": "noActiveCall", "message": "No active call", "requestId": "req-2" }
```

`message`는 TPG가 코드마다 갖고 있는 영어 문장이다. **분기는 항상 `code`로 하고 `message`는
표시용으로만 쓴다.** Voice Gateway의 한국어 거부 문구는 의도적으로 전달하지 않는다. 다른
서비스가 소유한 문자열이라 예고 없이 바뀔 수 있고, 수신 번호 같은 요청 값을 되돌려 싣는다.

### 6.1 명령 검증 / 세션 상태

| code | 기본 message | SDK 예외 | 비고 |
| --- | --- | --- | --- |
| `unauthenticated` | Authentication required | `AuthenticationError` | close 4401 동반. 10초 안에 `auth` 미도착도 포함 |
| `callAlreadyActive` | A call is already active | `CallAlreadyActiveError` | |
| `toRequired` | to is required | `ValidationError` | |
| `callIdRequired` | callId is required | `ValidationError` | `getSummary`에 `callId` 누락 |
| `callNotFound` | Call not found | `ValidationError` | `getSummary` 대상 통화 없음 |
| `callNotCompleted` | Call is not completed | `ValidationError` | `getSummary` 통화가 아직 미완료 |
| `noActiveCall` | No active call | `NoActiveCallError` | |
| `dtmfDigitsRequired` | digits is required | `ValidationError` | `sendDtmf`에 `digits` 누락 |
| `dtmfDigitsInvalid` | digits must contain only 0-9, *, # | `ValidationError` | `sendDtmf` `digits`에 허용 외 문자 |
| `callRejected` | Call rejected | `CallRejectedError` | `question` 필드 동반 가능 |
| `internalError` | Internal error | `TelloServerError` | TPG 자체 결함에만 쓴다 |

### 6.2 `createCall` 거부 — 계정 정책

SDK 예외는 전부 `CallRefusedError`. 세부 분기는 `code`로 한다. 앞의 세 가지는 계정 소유자가
조치할 수 있다.

| code | 기본 message | 발생 조건 | 클라이언트 대응 |
| --- | --- | --- | --- |
| `insufficientCredit` | The account has no call credit remaining | 계정에 통화 크레딧이 없음 | 충전을 안내한다. 재전송해도 소용없다 |
| `concurrentLimitExceeded` | The account is already using all of its concurrent outbound call lines | 계정의 동시 발신 회선을 모두 쓰는 중 | 자기 통화가 하나 끝나기를 기다렸다가 새 `createCall` |
| `callerNotVerified` | The callee number is not a verified number for this account | 수신 번호가 이 계정의 인증 번호가 아님 | 번호 인증을 먼저 안내한다. 재전송해도 소용없다 |
| `noRepresentativeNumber` | No outbound caller number is configured for this account | 계정에 발신 번호가 설정돼 있지 않음 | 발신 번호 설정을 안내한다. 재전송해도 소용없다 |

`insufficientCredit`은 발생 지점이 하나 더 있다. TPG가 통화 요청 전에 계정 크레딧을 자체
검사한다(한 번도 충전한 적 없는 계정을 제공자 게이트가 통과시키기 때문). 프레임은 양쪽이
동일하므로 클라이언트가 구분할 필요는 없다.

### 6.3 `createCall` 거부 — 서비스측 상태

SDK 예외는 전부 `CallProviderError`. 클라이언트가 원인을 만들지 않았고 고칠 수도 없다.

| code | 기본 message | 발생 조건 | 클라이언트 대응 |
| --- | --- | --- | --- |
| `callProviderUnauthorized` | The call provider rejected this service's credentials | 통화 제공자가 TPG 자체 자격 증명을 거부 | 서비스 장애로 보고한다. 재전송은 도움이 안 된다 |
| `callProviderDraining` | The call provider is not accepting new calls | 제공자가 신규 통화를 받지 않음(배포·드레인) | 클라이언트 판단으로 나중에 재시도 |
| `callProviderUnavailable` | The call provider could not create the media room for this call | 제공자가 미디어 룸을 만들지 못함 | 클라이언트 판단으로 나중에 재시도 |
| `callSetupFailed` | The call provider declined to start the call | 제공자가 거부했고 TPG가 사유를 분류하지 못함 | 실패로 보고한다 |

`callProviderUnauthorized`는 `unauthenticated`와 다르다. `unauthenticated`는 클라이언트
자신의 API key가 거부됐고 재인증하라는 뜻이고, 이쪽은 클라이언트가 갖고 있지도 않은 자격
증명에 대한 것이다. `callSetupFailed`를 `internalError`와 분리해 두는 이유도 같다. 제공자가
요청을 거부했다는 정보가 남아 있어 통화가 만들어지지 않았음이 확정되기 때문이다.

### 6.4 재시도

**§6.2·§6.3의 거부는 게이트웨이가 재시도하지 않는다.** 요청을 다시 보내지도, 통화를
큐잉하지도, 소켓을 붙잡고 기다리지도 않는다. 거부는 한 번 전달되고 연결은 즉시 새
`createCall`을 낼 수 있는 상태가 된다. 재시도 정책은 클라이언트 몫이며, 나중에 성공할 수
있는 것은 `concurrentLimitExceeded` · `callProviderDraining` · `callProviderUnavailable`
세 가지뿐이다.

명령 실패 error는 연결을 닫지 않는다.

## 7. Close code

| code | 이름 | 사용 |
| --- | --- | --- |
| 1000 | Normal | 정상 종료 |
| 1001 | GoingAway | 서버 종료(`shutting_down`) |
| 4401 | Unauthenticated | 연결 인증 실패, 또는 10초 안에 `auth` 프레임 미도착 |
| 4429 | SessionReplaced | 프로토콜에 정의만 되어 있고 **현재 방출되지 않는다** |

`1002` / `1008` / `1009` / `1011`도 상수로 정의돼 있으나 현재 게이트웨이 코드에서 쓰이지 않는다.

heartbeat 타임아웃으로 인한 종료는 close 프레임 없이 소켓이 끊긴다(`terminate()`).

## 8. 하트비트

서버가 **10초**(`HEARTBEAT_INTERVAL_MS`)마다 WebSocket 프로토콜 ping을 보낸다. 클라이언트는
표준 pong으로 응답해야 하며, 직전 ping에 pong이 없으면 다음 sweep에서 연결이 종료된다.
간격은 흔한 로드밸런서 idle 타임아웃(약 60초)보다 낮게 잡아, 살아 있지만 조용한 세션이
중간 장비에 끊기지 않게 한다.

앱 레벨 JSON 하트비트가 아니므로 표준 WS 라이브러리(Python `websockets`, Node `ws` 등)가
자동으로 처리한다.

통화 중에 클라이언트가 사라지면 게이트웨이가 해당 세션을 취소한다. 그렇게 하지 않으면
턴에 답할 사람 없이 Voice Gateway 레그가 계속 돌아간다.

## 9. 비목표

- 재연결 / 세션 resume 프로토콜 없음. 비정상 종료는 통화를 처음부터 다시 시작해야 하는 상황으로 본다.
- `answer.accepted`는 명령 제출 ACK일 뿐 실제 발화 전달 보장은 아니다.

## 10. 제거된 계약 (breaking)

옛 클라이언트가 참조할 수 있어 남겨 둔다. 전부 현재 게이트웨이에 없다.

| 제거된 것 | 대체 |
| --- | --- |
| `sendSms` 명령, `sms.sent` 이벤트, `smsToRequired` / `smsMessageRequired` / `smsFailed` 에러 코드 | 없음. TPG가 SMS 게이트웨이를 통째로 뺐다 |
| `listAgents` 명령, `agents.listed` 이벤트 | 없음 |
| `createCall`의 `agentId` | 에이전트는 서버가 정한다 |

deprecation shim이 없다. `sendSms` 프레임은 어떤 핸들러에도 매칭되지 않고 조용히 버려지므로,
옛 클라이언트는 아무 응답도 받지 못한 채 자기 타임아웃까지 블록된다. 그 외에는 이벤트 계약이
additive-only다.
