use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Auth,
    Validation,
    Safety,
    RateLimit,
    InsufficientFunds,
    OrderError,
    Network,
    Api,
    Config,
    Io,
}

impl ErrorCategory {
    pub fn from_cdc_code(code: i64) -> Self {
        match code {
            10001 | 10004 | 10005 | 10008 | 10009 => Self::Validation,
            10002 | 10003 | 10006 | 10007 | 40101 | 40102 => Self::Auth,
            20001 | 306 => Self::InsufficientFunds,
            20002 | 20005 | 20006 | 20007 | 318 => Self::OrderError,
            42901 => Self::RateLimit,
            _ => Self::Api,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimit | Self::Network)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub category: ErrorCategory,
    pub code: i64,
    pub message: String,
    pub retryable: bool,
}

impl ErrorEnvelope {
    pub fn api(code: i64, message: &str) -> Self {
        let category = ErrorCategory::from_cdc_code(code);
        let retryable = category.is_retryable();
        Self {
            category,
            code,
            message: message.to_string(),
            retryable,
        }
    }

    pub fn validation(message: &str) -> Self {
        Self {
            category: ErrorCategory::Validation,
            code: 0,
            message: message.to_string(),
            retryable: false,
        }
    }

    pub fn safety(message: &str) -> Self {
        Self {
            category: ErrorCategory::Safety,
            code: 0,
            message: message.to_string(),
            retryable: false,
        }
    }

    pub fn network(message: &str) -> Self {
        Self {
            category: ErrorCategory::Network,
            code: 0,
            message: message.to_string(),
            retryable: true,
        }
    }
}

impl fmt::Display for ErrorEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.category.category_name(), self.message)
    }
}

impl ErrorCategory {
    pub fn category_name(&self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Validation => "validation",
            Self::Safety => "safety",
            Self::RateLimit => "rate_limit",
            Self::InsufficientFunds => "insufficient_funds",
            Self::OrderError => "order_error",
            Self::Network => "network",
            Self::Api => "api",
            Self::Config => "config",
            Self::Io => "io",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CdcxError {
    #[error("API error: {0}")]
    Api(ErrorEnvelope),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("Config error: {0}")]
    Config(String),
}

impl From<tokio_tungstenite::tungstenite::Error> for CdcxError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(err))
    }
}

impl CdcxError {
    pub fn to_envelope(&self) -> ErrorEnvelope {
        match self {
            Self::Api(e) => e.clone(),
            Self::Http(e) => ErrorEnvelope::network(&e.to_string()),
            Self::Json(e) => ErrorEnvelope::validation(&format!("JSON parse error: {e}")),
            Self::Io(e) => ErrorEnvelope {
                category: ErrorCategory::Io,
                code: 0,
                message: e.to_string(),
                retryable: false,
            },
            Self::WebSocket(e) => ErrorEnvelope::network(&e.to_string()),
            Self::Config(msg) => ErrorEnvelope {
                category: ErrorCategory::Config,
                code: 0,
                message: msg.clone(),
                retryable: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_category_from_cdc_code() {
        // Validation
        assert_eq!(
            ErrorCategory::from_cdc_code(10001),
            ErrorCategory::Validation
        );
        assert_eq!(
            ErrorCategory::from_cdc_code(10005),
            ErrorCategory::Validation
        );
        // Auth
        assert_eq!(ErrorCategory::from_cdc_code(10002), ErrorCategory::Auth);
        assert_eq!(ErrorCategory::from_cdc_code(10007), ErrorCategory::Auth);
        assert_eq!(ErrorCategory::from_cdc_code(40101), ErrorCategory::Auth);
        // InsufficientFunds
        assert_eq!(
            ErrorCategory::from_cdc_code(20001),
            ErrorCategory::InsufficientFunds
        );
        assert_eq!(
            ErrorCategory::from_cdc_code(306),
            ErrorCategory::InsufficientFunds
        );
        // OrderError
        assert_eq!(
            ErrorCategory::from_cdc_code(20002),
            ErrorCategory::OrderError
        );
        assert_eq!(
            ErrorCategory::from_cdc_code(20005),
            ErrorCategory::OrderError
        );
        assert_eq!(ErrorCategory::from_cdc_code(318), ErrorCategory::OrderError);
        // RateLimit
        assert_eq!(
            ErrorCategory::from_cdc_code(42901),
            ErrorCategory::RateLimit
        );
        // Unknown -> Api
        assert_eq!(ErrorCategory::from_cdc_code(99999), ErrorCategory::Api);
    }

    #[test]
    fn test_error_envelope_json() {
        let envelope = ErrorEnvelope::api(42901, "rate limit exceeded");
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["category"], "rate_limit");
        assert_eq!(json["code"], 42901);
    }

    #[test]
    fn test_is_retryable() {
        assert!(ErrorCategory::RateLimit.is_retryable());
        assert!(ErrorCategory::Network.is_retryable());
        assert!(!ErrorCategory::Auth.is_retryable());
        assert!(!ErrorCategory::Validation.is_retryable());
    }
}
