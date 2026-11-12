use reqwest::Method;
use serde_json::{json, Value};

use super::{Api, Request, CREATED, OK};
use crate::Error;

const CALL_DRAFTS: &str = "call-drafts";

/// Call drafts: `/v1/call-drafts`. Get one with [`Api::call_drafts`].
///
/// A draft is a call scenario to review before dialing. Every draft status
/// (`pendingConfirmation`, `needsClarification`, `refused`, `confirmed`) is a
/// success; branch on `status`. [`confirm`](Self::confirm) places a real,
/// billed call.
#[derive(Debug, Clone, Copy)]
pub struct CallDrafts<'a> {
    pub(super) api: &'a Api,
}

impl CallDrafts<'_> {
    /// `POST /call-drafts`: asks for a draft of the call `body` describes
    /// (`agentId`, `to` and the documented per-call fields). Usually nothing
    /// is dialed, but an agent with a ready scenario is called at once
    /// (status `confirmed` with a `callId`), so send an `idempotency_key` if
    /// you may resend.
    pub async fn create(self, body: &Value, idempotency_key: Option<&str>) -> Result<Value, Error> {
        self.api
            .object(
                Request::new(Method::POST, &[CALL_DRAFTS], CREATED)
                    .body(body)
                    .idempotency_key(idempotency_key)
                    .long(),
            )
            .await
    }

    /// `POST /call-drafts/{draftId}/revise` with `{"request": request}`:
    /// returns a draft with a **new id**; the revised one can no longer be
    /// confirmed.
    pub async fn revise(
        self,
        draft_id: &str,
        request: &str,
        idempotency_key: Option<&str>,
    ) -> Result<Value, Error> {
        let body = json!({ "request": request });
        self.api
            .object(
                Request::new(Method::POST, &[CALL_DRAFTS, draft_id, "revise"], OK)
                    .body(&body)
                    .idempotency_key(idempotency_key)
                    .long(),
            )
            .await
    }

    /// `POST /call-drafts/{draftId}/confirm` without a body: places the call
    /// of the draft and returns the call object. **This dials a real, billed
    /// call.**
    ///
    /// The API requires an `Idempotency-Key`: `None` sends a fresh UUID v4.
    /// Every error after the arguments are checked carries the key sent
    /// ([`ApiError::idempotency_key`](crate::ApiError::idempotency_key),
    /// [`ApiConnectionError::idempotency_key`](crate::ApiConnectionError::idempotency_key));
    /// resend with that key so the call is not placed twice.
    pub async fn confirm(
        self,
        draft_id: &str,
        idempotency_key: Option<&str>,
    ) -> Result<Value, Error> {
        let generated;
        let key = match idempotency_key {
            Some(key) => key,
            None => {
                generated = uuid::Uuid::new_v4().to_string();
                &generated
            }
        };
        self.api
            .object(
                Request::new(Method::POST, &[CALL_DRAFTS, draft_id, "confirm"], CREATED)
                    .idempotency_key(Some(key))
                    .long(),
            )
            .await
    }
}
