use reqwest::Method;
use serde_json::Value;

use super::{Api, Request, OK};
use crate::Error;

const TOOLS: &str = "tools";

/// Tools: `/v1/tools`. Get one with [`Api::tools`].
///
/// Creates, updates and deletes save, deploy and publish at once: the next
/// call uses the result.
#[derive(Debug, Clone, Copy)]
pub struct Tools<'a> {
    pub(super) api: &'a Api,
}

/// The query of [`Tools::list`]. Unset options are not sent.
///
/// ```
/// let query = tello::ListTools::new().limit(20).cursor("next-page");
/// ```
#[derive(Debug, Clone, Default)]
pub struct ListTools {
    limit: Option<u32>,
    cursor: Option<String>,
}

impl ListTools {
    /// No limit and no cursor: the first page at the server's default size.
    pub fn new() -> Self {
        Self::default()
    }

    /// The page size.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// The `nextCursor` of the previous page. `""` is not sent.
    pub fn cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into()).filter(|cursor| !cursor.is_empty());
        self
    }
}

impl Tools<'_> {
    /// `GET /tools`: one page by ascending id,
    /// `{"data": [tool, …], "nextCursor": string | null}`, as sent.
    pub async fn list(self, query: ListTools) -> Result<Value, Error> {
        let mut pairs = Vec::new();
        if let Some(limit) = query.limit {
            pairs.push(("limit", limit.to_string()));
        }
        if let Some(cursor) = query.cursor {
            pairs.push(("cursor", cursor));
        }
        self.api
            .object(Request::new(Method::GET, &[TOOLS], OK).query(pairs))
            .await
    }

    /// `GET /tools/{id}`: one tool with its full settings.
    pub async fn get(self, id: &str) -> Result<Value, Error> {
        self.api.get_item(TOOLS, id).await
    }

    /// `POST /tools`: creates a tool from `body` (a JSON object) and returns
    /// it with its `revision`. With `idempotency_key`, resending the same body
    /// replays the first answer instead of creating another tool.
    pub async fn create(self, body: &Value, idempotency_key: Option<&str>) -> Result<Value, Error> {
        self.api.create_item(TOOLS, body, idempotency_key).await
    }

    /// `PATCH /tools/{id}`: changes only the fields in `patch`.
    pub async fn update(self, id: &str, patch: &Value) -> Result<Value, Error> {
        self.api.update_item(TOOLS, id, patch).await
    }

    /// `DELETE /tools/{id}`; the API answers `204` without a body.
    pub async fn delete(self, id: &str) -> Result<(), Error> {
        self.api.delete_item(TOOLS, id).await
    }
}
