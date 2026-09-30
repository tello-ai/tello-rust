use serde_json::Value;

use crate::error::Error;

/// A frame received from the gateway, or an SDK-local notice.
///
/// Every gateway-sent variant keeps the decoded frame in its `raw` field, also
/// reachable through [`Event::raw`]. `auth.ok` is consumed by
/// [`Client::connect`](crate::Client::connect) and never appears here.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// `call.created`: the call exists and is `queued`.
    CallCreated(CallCreated),
    /// `call.statusChanged`. A `cancelled` status ends the call.
    CallStatusChanged(CallStatusChanged),
    /// `user.turn`: the callee spoke; it is your turn to answer.
    UserTurn(Turn),
    /// `agent.turn`: an answer was actually spoken in the call.
    AgentTurn(Turn),
    /// `answer.accepted`: an `answer` was validated and forwarded. Not a
    /// delivery guarantee; the matching `agent.turn` confirms it was spoken.
    AnswerAccepted(AnswerAccepted),
    /// `dtmf.accepted`: a `sendDtmf` was validated and forwarded.
    DtmfAccepted(DtmfAccepted),
    /// `call.completed`: terminal.
    CallCompleted(CallEnded),
    /// `call.noAnswer`: terminal.
    CallNoAnswer(CallEnded),
    /// `call.failed`: terminal.
    CallFailed(CallEnded),
    /// `call.summary`: the reply to `getSummary`.
    CallSummary(CallSummary),
    /// `error`: a command failed. [`ErrorEvent::to_error`] gives the typed error.
    Error(ErrorEvent),
    /// SDK-local: the connection closed. Always the last event of a connection.
    Disconnected,
    /// A frame whose `type` this SDK version does not know. The event contract
    /// is additive, so newer gateways may send these.
    Unknown(Value),
}

static NO_FRAME: Value = Value::Null;

impl Event {
    /// The decoded frame this event came from. [`Value::Null`] for
    /// [`Event::Disconnected`], which has no frame.
    pub fn raw(&self) -> &Value {
        match self {
            Event::CallCreated(event) => &event.raw,
            Event::CallStatusChanged(event) => &event.raw,
            Event::UserTurn(event) | Event::AgentTurn(event) => &event.raw,
            Event::AnswerAccepted(event) => &event.raw,
            Event::DtmfAccepted(event) => &event.raw,
            Event::CallCompleted(event) | Event::CallNoAnswer(event) | Event::CallFailed(event) => {
                &event.raw
            }
            Event::CallSummary(event) => &event.raw,
            Event::Error(event) => &event.raw,
            Event::Unknown(raw) => raw,
            Event::Disconnected => &NO_FRAME,
        }
    }

    /// Whether this event ends the current call: `call.completed`,
    /// `call.noAnswer`, `call.failed`, or a `cancelled` status change.
    pub fn is_terminal(&self) -> bool {
        match self {
            Event::CallCompleted(_) | Event::CallNoAnswer(_) | Event::CallFailed(_) => true,
            Event::CallStatusChanged(changed) => changed.status == CallStatus::Cancelled,
            _ => false,
        }
    }
}

/// Call status vocabulary of protocol 1.0.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CallStatus {
    Queued,
    Dialing,
    Ringing,
    InProgress,
    Transferring,
    Completed,
    NoAnswer,
    Failed,
    Cancelled,
    /// A status this SDK version does not know, kept verbatim.
    Other(String),
}

impl CallStatus {
    /// The wire value, e.g. `inProgress`.
    pub fn as_str(&self) -> &str {
        match self {
            CallStatus::Queued => "queued",
            CallStatus::Dialing => "dialing",
            CallStatus::Ringing => "ringing",
            CallStatus::InProgress => "inProgress",
            CallStatus::Transferring => "transferring",
            CallStatus::Completed => "completed",
            CallStatus::NoAnswer => "noAnswer",
            CallStatus::Failed => "failed",
            CallStatus::Cancelled => "cancelled",
            CallStatus::Other(value) => value,
        }
    }

    fn parse(value: &str) -> CallStatus {
        match value {
            "queued" => CallStatus::Queued,
            "dialing" => CallStatus::Dialing,
            "ringing" => CallStatus::Ringing,
            "inProgress" => CallStatus::InProgress,
            "transferring" => CallStatus::Transferring,
            "completed" => CallStatus::Completed,
            "noAnswer" => CallStatus::NoAnswer,
            "failed" => CallStatus::Failed,
            "cancelled" => CallStatus::Cancelled,
            other => CallStatus::Other(other.to_owned()),
        }
    }
}

/// `call.created`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CallCreated {
    pub session_id: String,
    pub call_id: String,
    pub timestamp: String,
    pub raw: Value,
}

/// `call.statusChanged`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CallStatusChanged {
    pub session_id: String,
    pub call_id: String,
    pub timestamp: String,
    pub status: CallStatus,
    pub previous_status: CallStatus,
    pub raw: Value,
}

/// `user.turn` and `agent.turn`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Turn {
    pub session_id: String,
    pub call_id: String,
    pub timestamp: String,
    pub turn_index: u64,
    pub text: String,
    pub raw: Value,
}

/// `answer.accepted`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct AnswerAccepted {
    pub session_id: String,
    pub call_id: String,
    pub timestamp: String,
    pub request_id: Option<String>,
    pub message_id: String,
    pub raw: Value,
}

/// `dtmf.accepted`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DtmfAccepted {
    pub session_id: String,
    pub call_id: String,
    pub timestamp: String,
    pub request_id: Option<String>,
    pub message_id: String,
    pub digits: String,
    pub raw: Value,
}

/// `call.completed`, `call.noAnswer` and `call.failed`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CallEnded {
    pub session_id: String,
    pub call_id: String,
    pub timestamp: String,
    pub status: CallStatus,
    pub failure_reason: Option<String>,
    pub raw: Value,
}

/// `call.summary`. A command reply: it carries no session id or timestamp.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CallSummary {
    pub request_id: Option<String>,
    pub call_id: String,
    pub status: CallStatus,
    pub duration_seconds: Option<i64>,
    pub transcript: Option<String>,
    pub summary: Option<String>,
    pub credit_charged: Option<f64>,
    pub raw: Value,
}

/// `error`: a command failed. The connection stays open.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ErrorEvent {
    /// Branch on this, never on `message`.
    pub code: String,
    pub message: String,
    /// The `requestId` of the failed command, when it had one.
    pub request_id: Option<String>,
    /// Present for `callRejected`.
    pub question: Option<String>,
    pub raw: Value,
}

impl ErrorEvent {
    /// The typed error for this frame, mapped by `code` as declared in
    /// `docs/errors/errors.v1.json`. Unknown codes map to [`Error::Server`].
    pub fn to_error(&self) -> Error {
        Error::from_code(&self.code, &self.message, self.question.as_deref())
    }
}

/// Decodes one gateway frame. Never fails: a missing string is empty, a
/// missing optional is `None`, and an unknown `type` becomes [`Event::Unknown`].
pub(crate) fn parse(raw: Value) -> Event {
    let session_id = text(&raw, "sessionId");
    let call_id = text(&raw, "callId");
    let timestamp = text(&raw, "timestamp");
    match raw.get("type").and_then(Value::as_str).unwrap_or_default() {
        "call.created" => Event::CallCreated(CallCreated {
            session_id,
            call_id,
            timestamp,
            raw,
        }),
        "call.statusChanged" => Event::CallStatusChanged(CallStatusChanged {
            session_id,
            call_id,
            timestamp,
            status: status(&raw, "status"),
            previous_status: status(&raw, "previousStatus"),
            raw,
        }),
        kind @ ("user.turn" | "agent.turn") => {
            let wrap = if kind == "user.turn" {
                Event::UserTurn
            } else {
                Event::AgentTurn
            };
            wrap(Turn {
                session_id,
                call_id,
                timestamp,
                turn_index: raw
                    .get("turnIndex")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                text: text(&raw, "text"),
                raw,
            })
        }
        "answer.accepted" => Event::AnswerAccepted(AnswerAccepted {
            session_id,
            call_id,
            timestamp,
            request_id: optional_text(&raw, "requestId"),
            message_id: text(&raw, "messageId"),
            raw,
        }),
        "dtmf.accepted" => Event::DtmfAccepted(DtmfAccepted {
            session_id,
            call_id,
            timestamp,
            request_id: optional_text(&raw, "requestId"),
            message_id: text(&raw, "messageId"),
            digits: text(&raw, "digits"),
            raw,
        }),
        kind @ ("call.completed" | "call.noAnswer" | "call.failed") => {
            let wrap = match kind {
                "call.completed" => Event::CallCompleted,
                "call.noAnswer" => Event::CallNoAnswer,
                _ => Event::CallFailed,
            };
            wrap(CallEnded {
                session_id,
                call_id,
                timestamp,
                status: status(&raw, "status"),
                failure_reason: optional_text(&raw, "failureReason"),
                raw,
            })
        }
        "call.summary" => Event::CallSummary(CallSummary {
            request_id: optional_text(&raw, "requestId"),
            call_id,
            status: status(&raw, "status"),
            duration_seconds: raw.get("durationSeconds").and_then(Value::as_i64),
            transcript: optional_text(&raw, "transcript"),
            summary: optional_text(&raw, "summary"),
            credit_charged: raw.get("creditCharged").and_then(Value::as_f64),
            raw,
        }),
        "error" => Event::Error(ErrorEvent {
            code: text(&raw, "code"),
            message: text(&raw, "message"),
            request_id: optional_text(&raw, "requestId"),
            question: optional_text(&raw, "question"),
            raw,
        }),
        _ => Event::Unknown(raw),
    }
}

fn optional_text(raw: &Value, key: &str) -> Option<String> {
    raw.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn text(raw: &Value, key: &str) -> String {
    optional_text(raw, key).unwrap_or_default()
}

fn status(raw: &Value, key: &str) -> CallStatus {
    CallStatus::parse(raw.get(key).and_then(Value::as_str).unwrap_or_default())
}
