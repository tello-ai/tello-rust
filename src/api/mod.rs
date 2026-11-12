//! The public REST API (`/v1`): tools, agents, crews, crew generation and
//! call drafts. A separate client from the WebSocket [`Client`](crate::Client).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, StatusCode, Url};
use serde_json::Value;

use crate::config::ENV_API_KEY;
use crate::error::{ApiConnectionError, ApiError};
use crate::Error;

mod tools;

pub use tools::{ListTools, Tools};

/// REST API base URL used when neither [`ApiConfig::with_api_url`] nor
/// `TELLO_API_URL` is set.
pub const DEFAULT_API_URL: &str = "https://api.telloai.io/v1";
/// Environment variable read for the REST API base URL.
pub const ENV_API_URL: &str = "TELLO_API_URL";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LONG_TIMEOUT: Duration = Duration::from_secs(120);
const USER_AGENT: &str = concat!("tello-rust-sdk/", env!("CARGO_PKG_VERSION"));
const IDEMPOTENCY_KEY: &str = "idempotency-key";
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 255;

const OK: &[StatusCode] = &[StatusCode::OK];
const CREATED: &[StatusCode] = &[StatusCode::CREATED];
const NO_CONTENT: &[StatusCode] = &[StatusCode::NO_CONTENT];

/// Settings for [`Api::new`].
///
/// The API key goes only into the `Authorization` header. `Debug` prints it
/// redacted and no error contains it.
///
/// ```
/// use std::time::Duration;
///
/// let config = tello::ApiConfig::new("tello_live_xxx")
///     .with_api_url("https://api.telloai.io/v1")
///     .with_timeout(Duration::from_secs(10));
/// assert_eq!(config.api_url(), "https://api.telloai.io/v1");
/// ```
#[derive(Clone)]
pub struct ApiConfig {
    api_key: String,
    api_url: String,
    timeout: Duration,
    long_timeout: Duration,
}

impl ApiConfig {
    /// Uses `api_key`, or `TELLO_API_KEY` when it is empty. The base URL is
    /// `TELLO_API_URL` when set, otherwise [`DEFAULT_API_URL`]. Requests time
    /// out after 30s; call draft create, revise and confirm and crew
    /// generation after 120s.
    ///
    /// A missing key is reported by [`Api::new`] as
    /// [`Error::Config`](crate::Error::Config).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::resolve(api_key.into(), |name| std::env::var(name).ok())
    }

    /// Reads the API key from `TELLO_API_KEY`. Same as `ApiConfig::new("")`.
    pub fn from_env() -> Self {
        Self::new("")
    }

    /// Overrides the base URL, e.g. `https://api.telloai.io/v1`. It must be
    /// `https`; `http` is accepted only for `127.0.0.1`, `localhost` and
    /// `::1`.
    pub fn with_api_url(mut self, api_url: impl Into<String>) -> Self {
        self.api_url = api_url.into();
        self
    }

    /// Bounds each request, from connecting to the end of the response body,
    /// except the ones [`with_long_timeout`](Self::with_long_timeout) covers.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Bounds call draft create, revise and confirm and crew generation,
    /// which may wait for the server to write a scenario or build a crew.
    pub fn with_long_timeout(mut self, timeout: Duration) -> Self {
        self.long_timeout = timeout;
        self
    }

    /// The base URL.
    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    /// The request timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The timeout of the long requests.
    pub fn long_timeout(&self) -> Duration {
        self.long_timeout
    }

    fn resolve(api_key: String, env: impl Fn(&str) -> Option<String>) -> Self {
        let non_empty = |name: &str| env(name).filter(|value| !value.is_empty());
        let api_key = if api_key.is_empty() {
            non_empty(ENV_API_KEY).unwrap_or_default()
        } else {
            api_key
        };
        Self {
            api_key,
            api_url: non_empty(ENV_API_URL).unwrap_or_else(|| DEFAULT_API_URL.to_owned()),
            timeout: DEFAULT_TIMEOUT,
            long_timeout: DEFAULT_LONG_TIMEOUT,
        }
    }
}

impl fmt::Debug for ApiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiConfig")
            .field("api_key", &"[redacted]")
            .field("api_url", &self.api_url)
            .field("timeout", &self.timeout)
            .field("long_timeout", &self.long_timeout)
            .finish()
    }
}

/// A client of the public REST API (`/v1`).
///
/// Each resource has a namespace: [`tools`](Self::tools),
/// [`agents`](Self::agents), [`crews`](Self::crews),
/// [`crew_generations`](Self::crew_generations) and
/// [`call_drafts`](Self::call_drafts). Request and response bodies are JSON
/// objects as [`serde_json::Value`], so fields this version does not know
/// are kept.
///
/// Requests are never retried. Writes go live at once; a write that fails
/// with [`Error::ApiConnection`](crate::Error::ApiConnection) may still have
/// been applied, so check before resending (with the same idempotency key
/// where the request takes one).
///
/// `Api` is a cheap, cloneable handle; clones share one connection pool.
///
/// ```no_run
/// # async fn run() -> Result<(), tello::Error> {
/// // Reads TELLO_API_KEY, and TELLO_API_URL or https://api.telloai.io/v1.
/// let api = tello::Api::from_env()?;
/// let page = api.tools().list(tello::ListTools::new().limit(20)).await?;
/// println!("{}", page["data"]);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Api {
    inner: Arc<Inner>,
}

struct Inner {
    base: Url,
    authorization: HeaderValue,
    http: reqwest::Client,
    timeout: Duration,
    long_timeout: Duration,
}

impl Api {
    /// Checks `config` and builds the client. Nothing is sent yet.
    ///
    /// Fails with [`Error::Config`](crate::Error::Config) when there is no
    /// API key or the base URL is not `https` (or `http` on a loopback host).
    pub fn new(config: ApiConfig) -> Result<Api, Error> {
        if config.api_key.is_empty() {
            return Err(Error::Config(format!(
                "an API key is required: pass one to ApiConfig::new or set {ENV_API_KEY}"
            )));
        }
        let base = base_url(&config.api_url)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", config.api_key))
            .map_err(|_| {
                Error::Config("the API key has characters an HTTP header cannot carry".into())
            })?;
        authorization.set_sensitive(true);
        let tls = crate::client::tls_config()
            .map_err(|error| Error::Config(format!("TLS setup failed: {error}")))?;
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Config(format!("could not build the HTTP client: {error}")))?;
        Ok(Api {
            inner: Arc::new(Inner {
                base,
                authorization,
                http,
                timeout: config.timeout,
                long_timeout: config.long_timeout,
            }),
        })
    }

    /// Reads the API key from `TELLO_API_KEY`. Same as
    /// `Api::new(ApiConfig::from_env())`.
    pub fn from_env() -> Result<Api, Error> {
        Self::new(ApiConfig::from_env())
    }

    /// Tools: list, get, create, update, delete.
    pub fn tools(&self) -> Tools<'_> {
        Tools { api: self }
    }

    async fn object(&self, request: Request<'_>) -> Result<Value, Error> {
        let key = request.idempotency_key;
        let (status, body) = self.send(request).await?;
        match serde_json::from_slice(&body) {
            Ok(object @ Value::Object(_)) => Ok(object),
            _ => Err(invalid_response(
                status,
                "the response body is not a JSON object",
                key,
            )),
        }
    }

    async fn empty(&self, request: Request<'_>) -> Result<(), Error> {
        self.send(request).await.map(drop)
    }

    /// Sends `request` and returns the status and body of a documented
    /// success answer. Arguments are checked before anything is sent.
    async fn send(&self, request: Request<'_>) -> Result<(StatusCode, Vec<u8>), Error> {
        let key = request.idempotency_key;
        let key_header = key.map(idempotency_key_header).transpose()?;
        let url = self.endpoint(request.segments, &request.query)?;
        let timeout = if request.long {
            self.inner.long_timeout
        } else {
            self.inner.timeout
        };
        let mut builder = self
            .inner
            .http
            .request(request.method, url)
            .timeout(timeout)
            .header(AUTHORIZATION, self.inner.authorization.clone())
            .header(ACCEPT, HeaderValue::from_static("application/json"));
        if let Some(body) = request.body {
            let bytes = serde_json::to_vec(body).map_err(|error| {
                Error::InvalidArgument(format!("the request body cannot be encoded: {error}"))
            })?;
            builder = builder
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(bytes);
        }
        if let Some(value) = key_header {
            builder = builder.header(IDEMPOTENCY_KEY, value);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| connection_error(error, key))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| connection_error(error, key))?;
        if !status.is_success() {
            return Err(error_response(status, &body, key));
        }
        if !request.success.contains(&status) {
            return Err(invalid_response(
                status,
                &format!("the API answered {status}, not the documented success status"),
                key,
            ));
        }
        Ok((status, body.into()))
    }

    /// The base URL with `segments` appended, each percent-encoded, and
    /// `query`. An empty segment is an empty id.
    fn endpoint(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Url, Error> {
        if segments.iter().any(|segment| segment.is_empty()) {
            return Err(Error::InvalidArgument("an id must not be empty".into()));
        }
        let mut url = self.inner.base.clone();
        url.path_segments_mut()
            .map_err(|()| Error::Config("the API URL cannot take a path".into()))?
            .pop_if_empty()
            .extend(segments);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }
}

impl fmt::Debug for Api {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Api")
            .field("api_key", &"[redacted]")
            .field("api_url", &self.inner.base.as_str())
            .field("timeout", &self.inner.timeout)
            .field("long_timeout", &self.inner.long_timeout)
            .finish()
    }
}

/// One REST call: what to send and which success status the API documents.
struct Request<'a> {
    method: Method,
    segments: &'a [&'a str],
    query: Vec<(&'static str, String)>,
    body: Option<&'a Value>,
    idempotency_key: Option<&'a str>,
    long: bool,
    success: &'static [StatusCode],
}

impl<'a> Request<'a> {
    fn new(method: Method, segments: &'a [&'a str], success: &'static [StatusCode]) -> Self {
        Self {
            method,
            segments,
            query: Vec::new(),
            body: None,
            idempotency_key: None,
            long: false,
            success,
        }
    }

    fn query(mut self, query: Vec<(&'static str, String)>) -> Self {
        self.query = query;
        self
    }

    fn body(mut self, body: &'a Value) -> Self {
        self.body = Some(body);
        self
    }

    fn idempotency_key(mut self, key: Option<&'a str>) -> Self {
        self.idempotency_key = key;
        self
    }
}

/// Parses the base URL: `https`, or `http` on a loopback host. Trailing
/// slashes are dropped.
fn base_url(raw: &str) -> Result<Url, Error> {
    let invalid = |why: &str| Error::Config(format!("invalid API URL {raw:?}: {why}"));
    let url = Url::parse(raw.trim_end_matches('/')).map_err(|error| invalid(&error.to_string()))?;
    match url.scheme() {
        "https" => {}
        "http" if matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) => {}
        "http" => {
            return Err(invalid(
                "http is only allowed for 127.0.0.1, localhost and ::1",
            ))
        }
        _ => return Err(invalid("the scheme must be https")),
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("a query or fragment is not allowed"));
    }
    Ok(url)
}

/// Checks a caller's idempotency key: 1 to 255 bytes, no comma, no control
/// character.
fn idempotency_key_header(key: &str) -> Result<HeaderValue, Error> {
    if key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES {
        return Err(Error::InvalidArgument(format!(
            "an idempotency key must be 1 to {MAX_IDEMPOTENCY_KEY_BYTES} bytes"
        )));
    }
    if key.chars().any(|c| c == ',' || c.is_control()) {
        return Err(Error::InvalidArgument(
            "an idempotency key must not contain a comma or a control character".into(),
        ));
    }
    HeaderValue::from_str(key).map_err(|_| {
        Error::InvalidArgument("an idempotency key must be a valid header value".into())
    })
}

/// The error of a non-2xx answer: the envelope's `error`, `message` and
/// `data`, or `http_<status>` when the body is not an envelope.
fn error_response(status: StatusCode, body: &[u8], key: Option<&str>) -> Error {
    let mut envelope = match serde_json::from_slice(body) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    let code = match envelope.remove("error") {
        Some(Value::String(code)) if !code.is_empty() => Some(code),
        _ => None,
    };
    let (code, message, data) = match code {
        Some(code) => {
            let message = match envelope.remove("message") {
                Some(Value::String(message)) if !message.is_empty() => message,
                _ => code.clone(),
            };
            let data = envelope.remove("data").filter(|data| !data.is_null());
            (code, message, data)
        }
        None => (
            format!("http_{}", status.as_u16()),
            status
                .canonical_reason()
                .map_or_else(|| format!("HTTP {}", status.as_u16()), str::to_owned),
            None,
        ),
    };
    Error::Api(Box::new(ApiError {
        status: status.as_u16(),
        code,
        message,
        data,
        idempotency_key: key.map(str::to_owned),
    }))
}

fn invalid_response(status: StatusCode, message: &str, key: Option<&str>) -> Error {
    Error::Api(Box::new(ApiError {
        status: status.as_u16(),
        code: "invalid_response".to_owned(),
        message: message.to_owned(),
        data: None,
        idempotency_key: key.map(str::to_owned),
    }))
}

fn connection_error(error: reqwest::Error, key: Option<&str>) -> Error {
    Error::ApiConnection(ApiConnectionError::new(error, key.map(str::to_owned)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn api_key_argument_wins_over_env() {
        let config = ApiConfig::resolve("arg-key".into(), env(&[(ENV_API_KEY, "env-key")]));
        assert_eq!(config.api_key, "arg-key");
    }

    #[test]
    fn empty_api_key_falls_back_to_env() {
        let config = ApiConfig::resolve(String::new(), env(&[(ENV_API_KEY, "env-key")]));
        assert_eq!(config.api_key, "env-key");
    }

    #[test]
    fn missing_api_key_is_a_config_error() {
        let config = ApiConfig::resolve(String::new(), env(&[(ENV_API_KEY, "")]));
        let error = Api::new(config).expect_err("no key");
        assert!(matches!(&error, Error::Config(message) if message.contains(ENV_API_KEY)));
    }

    #[test]
    fn api_url_comes_from_argument_then_env_then_default() {
        let from_env = ApiConfig::resolve("k".into(), env(&[(ENV_API_URL, "https://env/v1")]));
        assert_eq!(from_env.api_url(), "https://env/v1");
        assert_eq!(
            from_env.with_api_url("https://arg/v1").api_url(),
            "https://arg/v1"
        );

        let empty_env = ApiConfig::resolve("k".into(), env(&[(ENV_API_URL, "")]));
        assert_eq!(empty_env.api_url(), DEFAULT_API_URL);
        assert_eq!(
            ApiConfig::resolve("k".into(), env(&[])).api_url(),
            DEFAULT_API_URL
        );
    }

    #[test]
    fn a_base_url_with_a_query_is_refused() {
        let config = ApiConfig::resolve("k".into(), env(&[])).with_api_url("https://x/v1?a=b");
        assert!(matches!(Api::new(config), Err(Error::Config(_))));
    }
}
