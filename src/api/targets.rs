use reqwest::Method;
use serde_json::Value;

use super::{Api, Request, OK};
use crate::Error;

const AGENTS: &str = "agents";
const CREWS: &str = "crews";

/// Agents: `/v1/agents`. Get one with [`Api::agents`].
///
/// Creates, updates and deletes save, deploy and publish at once: the next
/// call uses the result.
#[derive(Debug, Clone, Copy)]
pub struct Agents<'a> {
    pub(super) api: &'a Api,
}

impl Agents<'_> {
    /// `GET /agents`: `{"data": [{"id", "name", "description", "isDefault"}, …]}`,
    /// as sent.
    pub async fn list(self) -> Result<Value, Error> {
        self.api
            .object(Request::new(Method::GET, &[AGENTS], OK))
            .await
    }

    /// `GET /agents/{id}`: one agent with its full settings.
    pub async fn get(self, id: &str) -> Result<Value, Error> {
        self.api.get_item(AGENTS, id).await
    }

    /// `POST /agents`: creates an agent from `body` (a JSON object). With
    /// `idempotency_key`, resending the same body replays the first answer
    /// instead of creating another agent.
    pub async fn create(self, body: &Value, idempotency_key: Option<&str>) -> Result<Value, Error> {
        self.api.create_item(AGENTS, body, idempotency_key).await
    }

    /// `PATCH /agents/{id}`: changes only the fields in `patch`.
    pub async fn update(self, id: &str, patch: &Value) -> Result<Value, Error> {
        self.api.update_item(AGENTS, id, patch).await
    }

    /// `DELETE /agents/{id}`; the API answers `204` without a body.
    pub async fn delete(self, id: &str) -> Result<(), Error> {
        self.api.delete_item(AGENTS, id).await
    }
}

/// Crews: `/v1/crews`. Get one with [`Api::crews`].
///
/// Creates, updates and deletes save, deploy and publish at once: the next
/// call uses the result.
#[derive(Debug, Clone, Copy)]
pub struct Crews<'a> {
    pub(super) api: &'a Api,
}

impl Crews<'_> {
    /// `GET /crews`: `{"data": [crew, …]}`, as sent.
    pub async fn list(self) -> Result<Value, Error> {
        self.api
            .object(Request::new(Method::GET, &[CREWS], OK))
            .await
    }

    /// `GET /crews/{id}`: one crew with its full settings.
    pub async fn get(self, id: &str) -> Result<Value, Error> {
        self.api.get_item(CREWS, id).await
    }

    /// `POST /crews`: creates a crew from `body` (a JSON object). With
    /// `idempotency_key`, resending the same body replays the first answer
    /// instead of creating another crew.
    pub async fn create(self, body: &Value, idempotency_key: Option<&str>) -> Result<Value, Error> {
        self.api.create_item(CREWS, body, idempotency_key).await
    }

    /// `PATCH /crews/{id}`: changes only the fields in `patch`.
    pub async fn update(self, id: &str, patch: &Value) -> Result<Value, Error> {
        self.api.update_item(CREWS, id, patch).await
    }

    /// `DELETE /crews/{id}`; the API answers `204` without a body.
    pub async fn delete(self, id: &str) -> Result<(), Error> {
        self.api.delete_item(CREWS, id).await
    }
}
