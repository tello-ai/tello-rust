use std::fmt;
use std::sync::Arc;

/// Every failure the SDK reports.
///
/// Gateway error frames map onto the first eight variants by `code`, as
/// declared in `docs/errors/errors.v1.json`. Branch on [`Error::code`], never on
/// the message: the message is display text the gateway may reword.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The API key was refused: an `unauthenticated` error frame, close code
    /// 4401, or no `auth.ok` within the open timeout. Its code is always
    /// `unauthenticated`.
    #[error("authentication failed: {message}")]
    Authentication { message: String },

    /// A command was malformed or referred to a call that cannot be used:
    /// `toRequired`, `callIdRequired`, `callNotFound`, `callNotCompleted`,
    /// `dtmfDigitsRequired`, `dtmfDigitsInvalid`.
    #[error("{message} ({code})")]
    Validation { code: String, message: String },

    /// `createCall` was sent while a call was already active. The running call
    /// is unaffected. [`Client::wait_closed`](crate::Client::wait_closed)
    /// returns it only when it refused the `createCall` that opened the call:
    /// the gateway was still finishing the previous one, so retry shortly.
    #[error("{message} ({code})")]
    CallAlreadyActive { code: String, message: String },

    /// `answer` or `sendDtmf` was sent with no active call.
    #[error("{message} ({code})")]
    NoActiveCall { code: String, message: String },

    /// The call was rejected. `question` carries the gateway's follow-up
    /// question when it sent one.
    #[error("{message} ({code})")]
    CallRejected {
        code: String,
        message: String,
        question: Option<String>,
    },

    /// `createCall` was refused by an account policy before any call existed:
    /// no `call.created`, no call id, no charge. Codes `insufficientCredit`,
    /// `concurrentLimitExceeded`, `callerNotVerified`, `noRepresentativeNumber`.
    /// Only `concurrentLimitExceeded` can succeed on a later attempt, and the
    /// gateway never retries: any retry policy is yours.
    #[error("{message} ({code})")]
    CallRefused { code: String, message: String },

    /// `createCall` was refused by a condition on the service side that the
    /// caller neither caused nor can fix. Codes `callProviderUnauthorized`,
    /// `callProviderDraining`, `callProviderUnavailable`, `callSetupFailed`;
    /// only the draining and unavailable codes may succeed later.
    #[error("{message} ({code})")]
    CallProvider { code: String, message: String },

    /// A fault inside the gateway (`internalError`), or a code this SDK version
    /// does not know. `code` keeps the value the gateway sent.
    #[error("{message} ({code})")]
    Server { code: String, message: String },

    /// The connection closed, or was already closed, while the operation
    /// needed it.
    #[error("connection closed: {message}")]
    ConnectionClosed { message: String },

    /// The gateway closed the socket with 4429 because another session took
    /// its place.
    #[error("session replaced: {message}")]
    SessionReplaced { message: String },

    /// The WebSocket could not be opened: DNS, TCP, TLS, HTTP upgrade, or the
    /// open timeout.
    #[error("transport error: {0}")]
    Transport(TransportError),

    /// The configuration cannot be used, e.g. no API key was given and
    /// `TELLO_API_KEY` is unset.
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Error {
    /// The gateway error code. `None` for errors raised by the SDK itself:
    /// [`Error::ConnectionClosed`], [`Error::SessionReplaced`],
    /// [`Error::Transport`] and [`Error::Config`].
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Authentication { .. } => Some(UNAUTHENTICATED),
            Error::Validation { code, .. }
            | Error::CallAlreadyActive { code, .. }
            | Error::NoActiveCall { code, .. }
            | Error::CallRejected { code, .. }
            | Error::CallRefused { code, .. }
            | Error::CallProvider { code, .. }
            | Error::Server { code, .. } => Some(code),
            Error::ConnectionClosed { .. }
            | Error::SessionReplaced { .. }
            | Error::Transport(_)
            | Error::Config(_) => None,
        }
    }

    /// Maps a gateway error code to its typed error, as declared by the
    /// `sdkException` of each code in `docs/errors/errors.v1.json`.
    pub(crate) fn from_code(code: &str, message: &str, question: Option<&str>) -> Error {
        let code_owned = code.to_owned();
        let message = message.to_owned();
        match code {
            UNAUTHENTICATED => Error::Authentication { message },
            "toRequired" | "callIdRequired" | "callNotFound" | "callNotCompleted"
            | "dtmfDigitsRequired" | "dtmfDigitsInvalid" => Error::Validation {
                code: code_owned,
                message,
            },
            "callAlreadyActive" => Error::CallAlreadyActive {
                code: code_owned,
                message,
            },
            "noActiveCall" => Error::NoActiveCall {
                code: code_owned,
                message,
            },
            "callRejected" => Error::CallRejected {
                code: code_owned,
                message,
                question: question.map(str::to_owned),
            },
            "insufficientCredit"
            | "concurrentLimitExceeded"
            | "callerNotVerified"
            | "noRepresentativeNumber" => Error::CallRefused {
                code: code_owned,
                message,
            },
            "callProviderUnauthorized"
            | "callProviderDraining"
            | "callProviderUnavailable"
            | "callSetupFailed" => Error::CallProvider {
                code: code_owned,
                message,
            },
            // internalError, and any code this SDK version does not know.
            _ => Error::Server {
                code: code_owned,
                message,
            },
        }
    }

    pub(crate) fn connection_closed(message: impl Into<String>) -> Error {
        Error::ConnectionClosed {
            message: message.into(),
        }
    }
}

pub(crate) const UNAUTHENTICATED: &str = "unauthenticated";

/// Why the WebSocket could not be opened. Its [`source`](std::error::Error::source)
/// is the underlying WebSocket library error, when there is one.
#[derive(Clone)]
pub struct TransportError(Arc<TransportKind>);

#[derive(Debug)]
enum TransportKind {
    WebSocket(tokio_tungstenite::tungstenite::Error),
    Tls(rustls::Error),
    OpenTimeout,
}

impl TransportError {
    pub(crate) fn websocket(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self(Arc::new(TransportKind::WebSocket(error)))
    }

    pub(crate) fn tls(error: rustls::Error) -> Self {
        Self(Arc::new(TransportKind::Tls(error)))
    }

    pub(crate) fn open_timeout() -> Self {
        Self(Arc::new(TransportKind::OpenTimeout))
    }
}

impl fmt::Debug for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.0 {
            TransportKind::WebSocket(error) => fmt::Display::fmt(error, f),
            TransportKind::Tls(error) => write!(f, "TLS setup failed: {error}"),
            TransportKind::OpenTimeout => f.write_str("timed out opening the WebSocket"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &*self.0 {
            TransportKind::WebSocket(error) => Some(error),
            TransportKind::Tls(error) => Some(error),
            TransportKind::OpenTimeout => None,
        }
    }
}
