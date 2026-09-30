use std::fmt;
use std::time::Duration;

/// Gateway URL used when neither [`Config::with_url`] nor `TELLO_URL` is set.
pub const DEFAULT_URL: &str = "ws://localhost:3000/sdk";
/// Environment variable read when the API key argument is empty.
pub const ENV_API_KEY: &str = "TELLO_API_KEY";
/// Environment variable read for the gateway URL.
pub const ENV_URL: &str = "TELLO_URL";

const DEFAULT_OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Connection settings for [`Client::connect`](crate::Client::connect).
///
/// The API key is only ever written into the `auth` frame. `Debug` prints it
/// redacted and no error message contains it.
///
/// ```
/// use std::time::Duration;
///
/// let config = tello::Config::new("tello_live_xxx")
///     .with_url("wss://gateway.example.com/sdk")
///     .with_open_timeout(Duration::from_secs(5));
/// assert_eq!(config.url(), "wss://gateway.example.com/sdk");
/// ```
#[derive(Clone)]
pub struct Config {
    api_key: String,
    url: String,
    open_timeout: Duration,
    close_timeout: Duration,
}

impl Config {
    /// Uses `api_key`, or `TELLO_API_KEY` when it is empty. The URL is
    /// `TELLO_URL` when set, otherwise [`DEFAULT_URL`]. The open timeout is 10s
    /// and the close timeout 5s.
    ///
    /// A missing key is reported by [`Client::connect`](crate::Client::connect)
    /// as [`Error::Config`](crate::Error::Config).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::resolve(api_key.into(), |name| std::env::var(name).ok())
    }

    /// Reads the API key from `TELLO_API_KEY`. Same as `Config::new("")`.
    pub fn from_env() -> Self {
        Self::new("")
    }

    /// Overrides the gateway URL, `ws(s)://host/sdk`.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    /// Bounds the WebSocket opening handshake and, separately, the wait for
    /// `auth.ok`.
    pub fn with_open_timeout(mut self, timeout: Duration) -> Self {
        self.open_timeout = timeout;
        self
    }

    /// Bounds how long [`Client::close`](crate::Client::close) waits for the
    /// server to finish the close handshake.
    pub fn with_close_timeout(mut self, timeout: Duration) -> Self {
        self.close_timeout = timeout;
        self
    }

    /// The gateway URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The open timeout.
    pub fn open_timeout(&self) -> Duration {
        self.open_timeout
    }

    /// The close timeout.
    pub fn close_timeout(&self) -> Duration {
        self.close_timeout
    }

    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    pub(crate) fn resolve(api_key: String, env: impl Fn(&str) -> Option<String>) -> Self {
        let non_empty = |name: &str| env(name).filter(|value| !value.is_empty());
        let api_key = if api_key.is_empty() {
            non_empty(ENV_API_KEY).unwrap_or_default()
        } else {
            api_key
        };
        Self {
            api_key,
            url: non_empty(ENV_URL).unwrap_or_else(|| DEFAULT_URL.to_owned()),
            open_timeout: DEFAULT_OPEN_TIMEOUT,
            close_timeout: DEFAULT_CLOSE_TIMEOUT,
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("api_key", &"[redacted]")
            .field("url", &self.url)
            .field("open_timeout", &self.open_timeout)
            .field("close_timeout", &self.close_timeout)
            .finish()
    }
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
        let config = Config::resolve("arg-key".into(), env(&[(ENV_API_KEY, "env-key")]));
        assert_eq!(config.api_key(), "arg-key");
    }

    #[test]
    fn empty_api_key_falls_back_to_env() {
        let config = Config::resolve(String::new(), env(&[(ENV_API_KEY, "env-key")]));
        assert_eq!(config.api_key(), "env-key");
    }

    #[test]
    fn url_comes_from_env_then_default() {
        let from_env = Config::resolve("k".into(), env(&[(ENV_URL, "wss://gw.example/sdk")]));
        assert_eq!(from_env.url(), "wss://gw.example/sdk");

        let empty_env = Config::resolve("k".into(), env(&[(ENV_URL, "")]));
        assert_eq!(empty_env.url(), DEFAULT_URL);

        let unset = Config::resolve("k".into(), env(&[]));
        assert_eq!(unset.url(), DEFAULT_URL);
    }

    #[test]
    fn explicit_url_overrides_env() {
        let config = Config::resolve("k".into(), env(&[(ENV_URL, "ws://env/sdk")]))
            .with_url("ws://explicit/sdk");
        assert_eq!(config.url(), "ws://explicit/sdk");
    }

    #[test]
    fn default_timeouts_match_other_sdks() {
        let config = Config::resolve("k".into(), env(&[]));
        assert_eq!(config.open_timeout(), Duration::from_secs(10));
        assert_eq!(config.close_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn debug_output_redacts_api_key() {
        let config = Config::resolve("tello_live_secret".into(), env(&[]));
        let debug = format!("{config:?}");
        assert!(!debug.contains("tello_live_secret"), "{debug}");
        assert!(debug.contains("ws://localhost:3000/sdk"), "{debug}");
    }
}
