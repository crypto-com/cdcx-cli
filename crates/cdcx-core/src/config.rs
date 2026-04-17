use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::CdcxError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub api_key: String,
    pub api_secret: String,
    pub environment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default: Option<ProfileConfig>,
    #[serde(default)]
    pub profiles: Option<HashMap<String, ProfileConfig>>,
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
