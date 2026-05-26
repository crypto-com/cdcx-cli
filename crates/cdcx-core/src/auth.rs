use crate::config::Config;
use crate::error::CdcxError;

#[derive(Clone)]
pub struct Credentials {
    pub api_key: String,
    pub api_secret: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("api_key", &"<redacted>")
            .field("api_secret", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    pub fn from_parts(api_key: String, api_secret: String) -> Self {
        Self {
            api_key,
            api_secret,
        }
    }

    pub fn resolve(config: Option<&Config>, profile: Option<&str>) -> Result<Self, CdcxError> {
        // 1. Environment variables (CDCX_ prefix takes priority, CDC_ as fallback)
        // Treat empty strings as unset (plugin frameworks may pass "" for unconfigured values)
        let env_key = std::env::var("CDCX_API_KEY")
            .or_else(|_| std::env::var("CDC_API_KEY"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        let env_secret = std::env::var("CDCX_API_SECRET")
            .or_else(|_| std::env::var("CDC_API_SECRET"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        match (&env_key, &env_secret) {
            (Some(key), Some(secret)) => {
                return Ok(Self {
                    api_key: key.clone(),
                    api_secret: secret.clone(),
                });
            }
            (Some(_), None) => {
                eprintln!("warning: CDCX_API_KEY/CDC_API_KEY is set but CDCX_API_SECRET/CDC_API_SECRET is not — ignoring partial env credentials");
            }
            (None, Some(_)) => {
                eprintln!("warning: CDCX_API_SECRET/CDC_API_SECRET is set but CDCX_API_KEY/CDC_API_KEY is not — ignoring partial env credentials");
            }
            _ => {}
        }

        // 2. Config file
        if let Some(cfg) = config {
            let profile_config = cfg.profile(profile)?;
            return Ok(Self {
                api_key: profile_config.api_key,
                api_secret: profile_config.api_secret,
            });
        }

        Err(CdcxError::Config(
            "No credentials found. Set CDC_API_KEY/CDC_API_SECRET environment variables or configure credentials in ~/.config/cdcx/config.toml (run cdcx auth login)".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: env var tests are limited here to avoid process-global env var races.
    // The env var resolution path is tested via integration tests.

    #[test]
    fn test_resolve_from_config() {
        use crate::config::{Config, ProfileConfig};
        use std::collections::HashMap;
        let config = Config {
            default: Some(ProfileConfig {
                api_key: "cfg_key".into(),
                api_secret: "cfg_secret".into(),
                environment: "production".into(),
                envs: HashMap::new(),
            }),
            profiles: None,
            disable_update_check: false,
        };
        let creds = Credentials::resolve(Some(&config), None).unwrap();
        assert_eq!(creds.api_key, "cfg_key");
        assert_eq!(creds.api_secret, "cfg_secret");
    }
}
