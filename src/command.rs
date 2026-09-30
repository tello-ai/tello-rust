use serde_json::{json, Map, Value};

/// Parameters of [`Client::create_call`](crate::Client::create_call).
///
/// ```
/// let call = tello::CreateCall::new("+821012345678")
///     .prompt("Confirm the reservation")
///     .metadata(serde_json::json!({ "orderId": "o-1" }));
/// ```
#[derive(Debug, Clone)]
pub struct CreateCall {
    to: String,
    prompt: String,
    metadata: Option<Value>,
    request_id: Option<String>,
}

impl CreateCall {
    /// A call to `to`, the number to dial.
    pub fn new(to: impl Into<String>) -> Self {
        Self {
            to: to.into(),
            prompt: String::new(),
            metadata: None,
            request_id: None,
        }
    }

    /// Instructions for the call. Sent as `""` when not set.
    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    /// Arbitrary JSON attached to the call.
    pub fn metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// The `requestId` to send. An empty value counts as unset, and an unset
    /// one is replaced by a generated UUID v4. Use a value that no other
    /// command of this connection carries.
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub(crate) fn take_request_id(&mut self) -> Option<String> {
        self.request_id.take().filter(|id| !id.is_empty())
    }
}

/// Parameters of [`Client::answer`](crate::Client::answer).
#[derive(Debug, Clone)]
pub struct Answer {
    text: String,
    message_id: Option<String>,
    request_id: Option<String>,
}

impl Answer {
    /// Reply with `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            message_id: None,
            request_id: None,
        }
    }

    /// The message id to use. The gateway generates one when unset.
    pub fn message_id(mut self, message_id: impl Into<String>) -> Self {
        self.message_id = Some(message_id.into());
        self
    }

    /// Correlates the command with its `answer.accepted` or `error` frame.
    /// Not an idempotency key.
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

/// Parameters of [`Client::send_dtmf`](crate::Client::send_dtmf).
#[derive(Debug, Clone)]
pub struct SendDtmf {
    digits: String,
    message_id: Option<String>,
    request_id: Option<String>,
}

impl SendDtmf {
    /// Send `digits`: keypad characters `0-9`, `*` and `#`.
    pub fn new(digits: impl Into<String>) -> Self {
        Self {
            digits: digits.into(),
            message_id: None,
            request_id: None,
        }
    }

    /// The message id to use. The gateway generates one when unset.
    pub fn message_id(mut self, message_id: impl Into<String>) -> Self {
        self.message_id = Some(message_id.into());
        self
    }

    /// Correlates the command with its `dtmf.accepted` or `error` frame.
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

/// Parameters of [`Client::get_summary`](crate::Client::get_summary).
#[derive(Debug, Clone)]
pub struct GetSummary {
    call_id: String,
    request_id: Option<String>,
}

impl GetSummary {
    /// Summary of the completed call `call_id`.
    pub fn new(call_id: impl Into<String>) -> Self {
        Self {
            call_id: call_id.into(),
            request_id: None,
        }
    }

    /// Correlates the command with its `call.summary` or `error` frame.
    pub fn request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

fn envelope(event: &str, data: Map<String, Value>) -> String {
    json!({ "event": event, "data": data }).to_string()
}

/// Adds `key` only when `value` is set and non-empty: the other SDKs treat an
/// empty optional string as absent.
fn insert_optional(data: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        data.insert(key.to_owned(), Value::String(value));
    }
}

pub(crate) fn auth_frame(api_key: &str) -> String {
    let mut data = Map::new();
    data.insert("token".into(), Value::String(api_key.to_owned()));
    envelope("auth", data)
}

pub(crate) fn create_call_frame(call: CreateCall, request_id: &str) -> String {
    let mut data = Map::new();
    data.insert("to".into(), Value::String(call.to));
    data.insert("prompt".into(), Value::String(call.prompt));
    if let Some(metadata) = call.metadata {
        data.insert("metadata".into(), metadata);
    }
    data.insert("requestId".into(), Value::String(request_id.to_owned()));
    envelope("createCall", data)
}

pub(crate) fn answer_frame(answer: Answer) -> String {
    let mut data = Map::new();
    data.insert("text".into(), Value::String(answer.text));
    insert_optional(&mut data, "messageId", answer.message_id);
    insert_optional(&mut data, "requestId", answer.request_id);
    envelope("answer", data)
}

pub(crate) fn send_dtmf_frame(dtmf: SendDtmf) -> String {
    let mut data = Map::new();
    data.insert("digits".into(), Value::String(dtmf.digits));
    insert_optional(&mut data, "messageId", dtmf.message_id);
    insert_optional(&mut data, "requestId", dtmf.request_id);
    envelope("sendDtmf", data)
}

pub(crate) fn cancel_frame() -> String {
    envelope("cancel", Map::new())
}

pub(crate) fn get_summary_frame(summary: GetSummary) -> String {
    let mut data = Map::new();
    data.insert("callId".into(), Value::String(summary.call_id));
    insert_optional(&mut data, "requestId", summary.request_id);
    envelope("getSummary", data)
}
