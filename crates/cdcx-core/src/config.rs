use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::CdcxError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub api_key: String,
    pub api_secret: String,
    pub environment: String,
    #[serde(default)]
    pub envs: HashMap<String, String>,
}

impl ProfileConfig {
    /// Load envs into process environment variables.
    /// Each key is uppercased before being set.
    pub fn apply_env(&self) {
        for (key, value) in &self.envs {
            std::env::set_var(key.to_uppercase(), value);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default: Option<ProfileConfig>,
    #[serde(default)]
    pub profiles: Option<HashMap<String, ProfileConfig>>,
    #[serde(default)]
    pub disable_update_check: bool,
}

/// Check that a config file and its parent directory have owner-only permissions.
///
/// Returns an error if group or others have any access bits set, instructing the
/// user to tighten permissions manually.
///
/// On non-Unix platforms (Windows), this is a no-op since Windows uses ACLs
/// rather than POSIX permission bits, and user-profile directories are
/// protected by default.
#[cfg(unix)]
pub fn check_config_permissions(path: &std::path::Path) -> Result<(), CdcxError> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = path.parent() {
        if let Ok(meta) = std::fs::metadata(parent) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(CdcxError::Config(format!(
                    "Config directory has insecure permissions ({:04o}): {}\nRun: chmod 700 {}",
                    mode,
                    parent.display(),
                    parent.display(),
                )));
            }
        }
    }

    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(CdcxError::Config(format!(
                "Config file has insecure permissions ({:04o}): {}\nRun: chmod 600 {}\nIf this file was readable by others, consider rotating your API credentials.",
                mode,
                path.display(),
                path.display(),
            )));
        }
    }

    Ok(())
}

/// On Windows, checks ACLs via `icacls` to ensure no unexpected users have access.
/// Returns Ok if only the current user, SYSTEM, Administrators, and CREATOR OWNER
/// have access, or if `icacls` is unavailable (graceful degradation).
#[cfg(windows)]
pub fn check_config_permissions(path: &std::path::Path) -> Result<(), CdcxError> {
    if !path.exists() {
        return Ok(());
    }

    let username = match std::env::var("USERNAME") {
        Ok(u) if !u.is_empty() => u.to_lowercase(),
        _ => return Ok(()), // can't determine user; degrade gracefully
    };

    let output = match std::process::Command::new("icacls")
        .arg(path.as_os_str())
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Ok(()), // icacls not available or failed; degrade gracefully
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Each ACL line looks like:  DESKTOP\User:(F)  or  NT AUTHORITY\SYSTEM:(F)
    // We allow: current user, SYSTEM, Administrators, CREATOR OWNER
    for line in stdout.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Successfully") {
            continue;
        }
        let lower = trimmed.to_lowercase();
        let is_allowed = lower.contains(&username)
            || lower.contains("nt authority\\system")
            || lower.contains("builtin\\administrators")
            || lower.contains("creator owner");
        if !is_allowed {
            return Err(CdcxError::Config(format!(
                "Config file has insecure permissions: {}\nUnexpected access: {}\nRun: icacls \"{}\" /inheritance:r /grant:r \"%USERNAME%:(F)\"",
                path.display(),
                trimmed,
                path.display(),
            )));
        }
    }

    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub fn check_config_permissions(_path: &std::path::Path) -> Result<(), CdcxError> {
    Ok(())
}

/// Set owner-only permissions on a file (chmod 600 equivalent).
/// On Unix: `mode 0o600`. On Windows: `icacls /inheritance:r /grant:r %USERNAME%:(F)`.
pub fn set_file_owner_only(path: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        let username = std::env::var("USERNAME").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "USERNAME env var not set")
        })?;
        if username.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "USERNAME env var is empty",
            ));
        }
        let status = std::process::Command::new("icacls")
            .arg(path.as_os_str())
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("{}:(F)", username))
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "icacls failed to set file permissions",
            ));
        }
    }
    Ok(())
}

/// Set owner-only permissions on a directory (chmod 700 equivalent).
/// On Unix: `mode 0o700`. On Windows: `icacls /inheritance:r /grant:r %USERNAME%:(OI)(CI)(F)`.
pub fn set_dir_owner_only(path: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        let username = std::env::var("USERNAME").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "USERNAME env var not set")
        })?;
        if username.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "USERNAME env var is empty",
            ));
        }
        // (OI)(CI) = Object Inherit + Container Inherit — applies to files and subdirs
        let status = std::process::Command::new("icacls")
            .arg(path.as_os_str())
            .args(["/inheritance:r", "/grant:r"])
            .arg(format!("{}:(OI)(CI)(F)", username))
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "icacls failed to set directory permissions",
            ));
        }
    }
    Ok(())
}

pub const MCP_SERVICE_GROUPS: &[(&str, &str)] = &[
    ("market", "Tickers, orderbook, candles"),
    ("account", "Balances, positions, history"),
    ("trade", "Place, amend, cancel orders"),
    ("advanced", "OCO, OTO, OTOCO orders"),
    ("margin", "Margin transfers, leverage"),
    ("staking", "Stake/unstake operations"),
    ("funding", "Withdrawals (dangerous)"),
    ("fiat", "Fiat operations (dangerous)"),
    ("otc", "OTC desk operations"),
    ("stream", "Real-time data streams"),
];

pub fn valid_mcp_service_names() -> Vec<&'static str> {
    MCP_SERVICE_GROUPS.iter().map(|(name, _)| *name).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default = "McpConfig::default_services")]
    pub services: Vec<String>,
    #[serde(default)]
    pub allow_dangerous: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            services: Self::default_services(),
            allow_dangerous: false,
        }
    }
}

impl McpConfig {
    fn default_services() -> Vec<String> {
        vec!["market".to_string()]
    }

    pub fn default_path() -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("cdcx").join("mcp.toml"))
    }

    pub fn load_default() -> Result<Option<Self>, CdcxError> {
        let path = match Self::default_path() {
            Some(p) => p,
            None => return Ok(None),
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(CdcxError::Config(format!(
                    "Failed to read mcp config: {}",
                    e
                )))
            }
        };
        let config: Self = toml::from_str(&content)
            .map_err(|e| CdcxError::Config(format!("Failed to parse mcp.toml: {}", e)))?;
        Ok(Some(config))
    }

    pub fn save(&self) -> Result<(), CdcxError> {
        let path = Self::default_path()
            .ok_or_else(|| CdcxError::Config("Cannot determine home directory".into()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CdcxError::Config(format!("Failed to create config dir: {}", e)))?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| CdcxError::Config(format!("Failed to serialize mcp config: {}", e)))?;
        std::fs::write(&path, content)
            .map_err(|e| CdcxError::Config(format!("Failed to write mcp.toml: {}", e)))?;
        Ok(())
    }

    pub fn delete() -> Result<(), CdcxError> {
        let path = Self::default_path()
            .ok_or_else(|| CdcxError::Config("Cannot determine home directory".into()))?;
        match std::fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CdcxError::Config(format!(
                "Failed to remove mcp.toml: {}",
                e
            ))),
        }
    }

    pub fn services_string(&self) -> String {
        self.services.join(",")
    }
}

impl Config {
    /// Return the default config path (~/.config/cdcx/config.toml).
    pub fn default_path() -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("cdcx").join("config.toml"))
    }

    /// Load config from the default path (~/.config/cdcx/config.toml).
    ///
    /// Does NOT check file permissions — callers that read credentials from
    /// the config should call `check_config_permissions()` separately.
    pub fn load_default() -> Result<Option<Self>, CdcxError> {
        let path = match Self::default_path() {
            Some(p) => p,
            None => return Ok(None),
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(CdcxError::Config(format!("Failed to read config: {}", e))),
        };

        Ok(Self::parse(&content).ok())
    }

    pub fn parse(toml_str: &str) -> Result<Self, CdcxError> {
        toml::from_str(toml_str)
            .map_err(|e| CdcxError::Config(format!("Failed to parse TOML: {}", e)))
    }

    pub fn profile(&self, name: Option<&str>) -> Result<ProfileConfig, CdcxError> {
        match name {
            None => self
                .default
                .clone()
                .or_else(|| {
                    self.profiles
                        .as_ref()
                        .and_then(|p| p.get("default").cloned())
                })
                .ok_or_else(|| CdcxError::Config("No default profile found in config".to_string())),
            Some(profile_name) => {
                let profiles = self.profiles.as_ref().ok_or_else(|| {
                    CdcxError::Config(format!("Profile '{}' not found", profile_name))
                })?;
                profiles.get(profile_name).cloned().ok_or_else(|| {
                    CdcxError::Config(format!("Profile '{}' not found", profile_name))
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parse() {
        let toml = r#"
[default]
api_key = "key1"
api_secret = "secret1"
environment = "production"

[profiles.uat]
api_key = "key2"
api_secret = "secret2"
environment = "uat"
"#;
        let config = Config::parse(toml).unwrap();
        let default = config.profile(None).unwrap();
        assert_eq!(default.api_key, "key1");
        let uat = config.profile(Some("uat")).unwrap();
        assert_eq!(uat.api_key, "key2");
    }

    #[test]
    fn test_config_missing_profile() {
        let toml = "[default]\napi_key = \"k\"\napi_secret = \"s\"\nenvironment = \"production\"\n";
        let config = Config::parse(toml).unwrap();
        assert!(config.profile(Some("nonexistent")).is_err());
    }

    #[test]
    fn test_disable_update_check_defaults_false() {
        let toml = r#"
[default]
api_key = "k"
api_secret = "s"
environment = "production"
"#;
        let config = Config::parse(toml).unwrap();
        assert!(!config.disable_update_check);
    }

    #[test]
    fn test_disable_update_check_true() {
        let toml = r#"
disable_update_check = true

[default]
api_key = "k"
api_secret = "s"
environment = "production"
"#;
        let config = Config::parse(toml).unwrap();
        assert!(config.disable_update_check);
    }

    #[test]
    fn test_disable_update_check_false() {
        let toml = r#"
disable_update_check = false

[default]
api_key = "k"
api_secret = "s"
environment = "production"
"#;
        let config = Config::parse(toml).unwrap();
        assert!(!config.disable_update_check);
    }

    #[test]
    fn test_disable_update_check_without_credentials() {
        let toml = "disable_update_check = true\n";
        let config = Config::parse(toml).unwrap();
        assert!(config.disable_update_check);
        assert!(config.default.is_none());
    }

    #[test]
    fn test_profiles_default_fallback() {
        let toml = r#"
[profiles.default]
api_key = "pkey"
api_secret = "psecret"
environment = "uat"
"#;
        let config = Config::parse(toml).unwrap();
        let profile = config.profile(None).unwrap();
        assert_eq!(profile.api_key, "pkey");
        assert_eq!(profile.api_secret, "psecret");
        assert_eq!(profile.environment, "uat");
    }

    mod mcp_config_tests {
        use super::*;

        #[test]
        fn test_default_mcp_config() {
            let config = McpConfig::default();
            assert_eq!(config.services, vec!["market"]);
            assert!(!config.allow_dangerous);
        }

        #[test]
        fn test_parse_mcp_config() {
            let toml_str =
                "services = [\"market\", \"trade\", \"account\"]\nallow_dangerous = true\n";
            let config: McpConfig = toml::from_str(toml_str).unwrap();
            assert_eq!(config.services, vec!["market", "trade", "account"]);
            assert!(config.allow_dangerous);
        }

        #[test]
        fn test_parse_empty_mcp_config() {
            let config: McpConfig = toml::from_str("").unwrap();
            assert_eq!(config.services, vec!["market"]);
            assert!(!config.allow_dangerous);
        }

        #[test]
        fn test_services_string() {
            let config = McpConfig {
                services: vec!["market".into(), "trade".into()],
                allow_dangerous: false,
            };
            assert_eq!(config.services_string(), "market,trade");
        }
    }

    #[cfg(unix)]
    mod permission_tests {
        use super::super::check_config_permissions;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU32, Ordering};

        static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

        fn write_temp_config(
            dir_mode: u32,
            file_mode: u32,
        ) -> (std::path::PathBuf, std::path::PathBuf) {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir =
                std::env::temp_dir().join(format!("cdcx_perm_test_{}_{}", std::process::id(), id));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("config.toml");
            std::fs::write(
                &file,
                "[default]\napi_key=\"k\"\napi_secret=\"s\"\nenvironment=\"production\"\n",
            )
            .unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(dir_mode)).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(file_mode)).unwrap();
            (dir, file)
        }

        fn cleanup(dir: &std::path::Path) {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let _ = std::fs::set_permissions(
                        entry.path(),
                        std::fs::Permissions::from_mode(0o644),
                    );
                    let _ = std::fs::remove_file(entry.path());
                }
            }
            let _ = std::fs::remove_dir(dir);
        }

        #[test]
        fn test_check_permissions_rejects_group_readable() {
            let (dir, file) = write_temp_config(0o700, 0o640);
            let result = check_config_permissions(&file);
            cleanup(&dir);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("insecure permissions"),
                "unexpected message: {}",
                msg
            );
        }

        #[test]
        fn test_check_permissions_rejects_world_readable() {
            let (dir, file) = write_temp_config(0o700, 0o644);
            let result = check_config_permissions(&file);
            cleanup(&dir);
            assert!(result.is_err());
        }

        #[test]
        fn test_check_permissions_rejects_open_directory() {
            let (dir, file) = write_temp_config(0o755, 0o600);
            let result = check_config_permissions(&file);
            cleanup(&dir);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(msg.contains("directory"), "unexpected message: {}", msg);
        }

        #[test]
        fn test_check_permissions_accepts_owner_only() {
            let (dir, file) = write_temp_config(0o700, 0o600);
            let result = check_config_permissions(&file);
            cleanup(&dir);
            assert!(result.is_ok());
        }
    }
}
