use crate::error::CdcxError;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Uat,
}

impl Environment {
    pub fn rest_url(&self) -> String {
        if let Ok(url) = std::env::var("CDCX_REST_URL") {
            return url;
        }
        match self {
            Self::Production => "https://api.crypto.com/exchange/v1",
            Self::Uat => "https://uat-api.3ona.co/exchange/v1",
        }
        .to_string()
    }

    pub fn ws_market_url(&self) -> String {
        if let Ok(url) = std::env::var("CDCX_WS_MARKET_URL") {
            return url;
        }
        match self {
            Self::Production => "wss://stream.crypto.com/exchange/v1/market",
            Self::Uat => "wss://uat-stream.3ona.co/exchange/v1/market",
        }
        .to_string()
    }

    pub fn ws_user_url(&self) -> String {
        if let Ok(url) = std::env::var("CDCX_WS_USER_URL") {
            return url;
        }
        match self {
            Self::Production => "wss://stream.crypto.com/exchange/v1/user",
            Self::Uat => "wss://uat-stream.3ona.co/exchange/v1/user",
        }
        .to_string()
    }
}

impl Environment {
    /// Resolve environment with priority: CLI flag > env var > config file > default (Production).
    ///
    /// Returns an error if an explicit CLI flag is provided but invalid (preserves
    /// the old behavior of rejecting `--env staging`). Env vars and config values
    /// silently fall through on parse failure since they are best-effort sources.
    pub fn resolve(
        flag_env: Option<&str>,
        config: Option<&crate::config::Config>,
        profile: Option<&str>,
    ) -> Result<Self, CdcxError> {
        // 1. Explicit CLI flag — must be valid if provided
        if let Some(s) = flag_env {
            return s.parse::<Self>();
        }

        // 2. Environment variable (CDCX_ prefix takes priority, CDC_ as fallback)
        let from_env =
            std::env::var("CDCX_ENVIRONMENT").or_else(|_| std::env::var("CDC_ENVIRONMENT"));
        if let Ok(s) = from_env {
            if let Ok(env) = s.parse::<Self>() {
                return Ok(env);
            }
        }

        // 3. Config file profile's environment field
        if let Some(cfg) = config {
            if let Ok(profile_config) = cfg.profile(profile) {
                if let Ok(env) = profile_config.environment.parse::<Self>() {
                    return Ok(env);
                }
            }
        }

        // 4. Default
        Ok(Self::Production)
    }
}

impl FromStr for Environment {
    type Err = CdcxError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "production" => Ok(Self::Production),
            "uat" => Ok(Self::Uat),
            _ => Err(CdcxError::Config(format!(
                "Unknown environment: {}. Valid values are: production, uat",
                s
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Covers both the unset-default URLs and the CDCX_* env-var overrides in
    /// one sequential test. These used to live in two `#[test]` fns, but Rust
    /// runs tests in parallel threads by default and `std::env::set_var` /
    /// `remove_var` are process-global — so the override test's mid-flight
    /// `set_var("CDCX_WS_USER_URL", ...)` would race the default test's read,
    /// producing sporadic Windows CI failures (slower env syscalls amplify the
    /// race). Collapsing into one test pins the set/read/remove cycles to a
    /// single thread's timeline and removes the race without new dependencies.
    #[test]
    fn test_environment_urls_and_overrides() {
        // Clear any overrides the environment might inject before we start.
        std::env::remove_var("CDCX_REST_URL");
        std::env::remove_var("CDCX_WS_MARKET_URL");
        std::env::remove_var("CDCX_WS_USER_URL");

        // --- Defaults ---
        assert_eq!(
            Environment::Production.rest_url(),
            "https://api.crypto.com/exchange/v1"
        );
        assert_eq!(
            Environment::Uat.rest_url(),
            "https://uat-api.3ona.co/exchange/v1"
        );
        assert_eq!(
            Environment::Production.ws_market_url(),
            "wss://stream.crypto.com/exchange/v1/market"
        );
        assert_eq!(
            Environment::Production.ws_user_url(),
            "wss://stream.crypto.com/exchange/v1/user"
        );
        assert_eq!(
            Environment::Uat.ws_market_url(),
            "wss://uat-stream.3ona.co/exchange/v1/market"
        );
        assert_eq!(
            Environment::Uat.ws_user_url(),
            "wss://uat-stream.3ona.co/exchange/v1/user"
        );

        // --- Overrides (each sets, asserts, then restores before the next) ---
        std::env::set_var("CDCX_REST_URL", "https://custom-api.example.com/v1");
        assert_eq!(
            Environment::Production.rest_url(),
            "https://custom-api.example.com/v1"
        );
        std::env::remove_var("CDCX_REST_URL");

        std::env::set_var("CDCX_WS_MARKET_URL", "wss://custom-ws.example.com/market");
        assert_eq!(
            Environment::Production.ws_market_url(),
            "wss://custom-ws.example.com/market"
        );
        std::env::remove_var("CDCX_WS_MARKET_URL");

        std::env::set_var("CDCX_WS_USER_URL", "wss://custom-ws.example.com/user");
        assert_eq!(
            Environment::Production.ws_user_url(),
            "wss://custom-ws.example.com/user"
        );
        std::env::remove_var("CDCX_WS_USER_URL");
    }

    #[test]
    fn test_environment_from_str() {
        assert_eq!(
            "production".parse::<Environment>().unwrap(),
            Environment::Production
        );
        assert_eq!("uat".parse::<Environment>().unwrap(), Environment::Uat);
        assert!("invalid".parse::<Environment>().is_err());
    }

    #[test]
    fn test_resolve_flag_wins() {
        use crate::config::{Config, ProfileConfig};
        let config = Config {
            default: Some(ProfileConfig {
                api_key: "k".into(),
                api_secret: "s".into(),
                environment: "uat".into(),
            }),
            profiles: None,
        };
        // Explicit flag overrides config
        assert_eq!(
            Environment::resolve(Some("production"), Some(&config), None).unwrap(),
            Environment::Production
        );
    }

    #[test]
    fn test_resolve_invalid_flag_errors() {
        // An explicit but invalid CLI flag must produce an error (not silently fall through)
        let result = Environment::resolve(Some("staging"), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_config_used_when_no_flag() {
        use crate::config::{Config, ProfileConfig};
        let config = Config {
            default: Some(ProfileConfig {
                api_key: "k".into(),
                api_secret: "s".into(),
                environment: "uat".into(),
            }),
            profiles: None,
        };
        assert_eq!(
            Environment::resolve(None, Some(&config), None).unwrap(),
            Environment::Uat
        );
    }

    #[test]
    fn test_resolve_defaults_to_production() {
        assert_eq!(
            Environment::resolve(None, None, None).unwrap(),
            Environment::Production
        );
    }
}
