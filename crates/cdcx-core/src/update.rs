use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn releases_api_url() -> String {
    crate::github::api("releases/latest")
}

fn releases_html_url() -> String {
    crate::github::html("releases/latest")
}

pub struct UpdateChecker {
    meta_path: PathBuf,
    ttl: Duration,
}

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
    pub html_url: String,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("cdcx");
        Self {
            meta_path: cache_dir.join("update-meta.json"),
            ttl: Duration::from_secs(86400),
        }
    }
}

impl UpdateChecker {
    pub fn should_check(&self) -> bool {
        let Ok(content) = fs::read_to_string(&self.meta_path) else {
            return true;
        };
        let Ok(meta) = serde_json::from_str::<serde_json::Value>(&content) else {
            return true;
        };
        let Some(checked_at) = meta["checked_at"].as_str() else {
            return true;
        };
        let Ok(checked_time) = chrono::DateTime::parse_from_rfc3339(checked_at) else {
            return true;
        };
        let elapsed = chrono::Utc::now().signed_duration_since(checked_time);
        elapsed.num_seconds() > self.ttl.as_secs() as i64
    }

    pub async fn fetch_latest(&self) -> Result<ReleaseInfo, UpdateError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent("cdcx-cli")
            .build()
            .map_err(|e| UpdateError(format!("HTTP client error: {e}")))?;

        let resp: serde_json::Value = client
            .get(releases_api_url())
            .send()
            .await
            .map_err(|e| UpdateError(format!("Failed to check for updates: {e}")))?
            .json()
            .await
            .map_err(|e| UpdateError(format!("Failed to parse release info: {e}")))?;

        let tag = resp["tag_name"]
            .as_str()
            .ok_or_else(|| UpdateError("No tag_name in release".into()))?;
        let version = tag.strip_prefix('v').unwrap_or(tag).to_string();

        let html_url = resp["html_url"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(releases_html_url);

        let target = current_target();
        let (download_url, asset_name) = if let Some(assets) = resp["assets"].as_array() {
            find_asset(assets, &version, target)
        } else {
            (None, None)
        };

        let info = ReleaseInfo {
            version,
            download_url,
            asset_name,
            html_url,
        };

        self.save_meta(&info);
        Ok(info)
    }

    pub fn cached_release_info(&self) -> Option<ReleaseInfo> {
        let content = fs::read_to_string(&self.meta_path).ok()?;
        let meta: serde_json::Value = serde_json::from_str(&content).ok()?;
        let version = meta["latest_version"].as_str()?;
        Some(ReleaseInfo {
            version: version.to_string(),
            download_url: meta["download_url"].as_str().map(String::from),
            asset_name: meta["asset_name"].as_str().map(String::from),
            html_url: meta["html_url"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(releases_html_url),
        })
    }

    fn save_meta(&self, info: &ReleaseInfo) {
        let meta = serde_json::json!({
            "checked_at": chrono::Utc::now().to_rfc3339(),
            "latest_version": info.version,
            "download_url": info.download_url,
            "asset_name": info.asset_name,
            "html_url": info.html_url,
        });
        if let Some(parent) = self.meta_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(
            &self.meta_path,
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        );
    }
}

/// Progress update sent during download.
#[derive(Debug, Clone)]
pub enum UpdateProgress {
    Downloading { downloaded: u64, total: Option<u64> },
    Extracting,
    Installing,
    Done,
    Failed(String),
}

pub async fn download_and_install(info: &ReleaseInfo) -> Result<(), UpdateError> {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    download_and_install_with_progress(info, tx).await
}

pub async fn download_and_install_with_progress(
    info: &ReleaseInfo,
    progress: tokio::sync::mpsc::UnboundedSender<UpdateProgress>,
) -> Result<(), UpdateError> {
    use futures_util::StreamExt;

    let download_url = info
        .download_url
        .as_ref()
        .ok_or_else(|| UpdateError("No download URL available for this platform".into()))?;

    let current_exe = std::env::current_exe()
        .map_err(|e| UpdateError(format!("Cannot determine current binary path: {e}")))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent("cdcx-cli")
        .build()
        .map_err(|e| UpdateError(format!("HTTP client error: {e}")))?;

    let tmp_dir = std::env::temp_dir().join("cdcx-update");
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)
        .map_err(|e| UpdateError(format!("Failed to create temp dir: {e}")))?;

    let asset_name = info.asset_name.as_deref().unwrap_or("archive");
    let archive_path = tmp_dir.join(asset_name);

    let resp = client
        .get(download_url)
        .send()
        .await
        .map_err(|e| UpdateError(format!("Download failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(UpdateError(format!(
            "Download failed: HTTP {}",
            resp.status()
        )));
    }

    let total = resp.content_length();
    let mut downloaded: u64 = 0;
    let mut body = resp.bytes_stream();
    let mut file_bytes: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);

    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| UpdateError(format!("Download error: {e}")))?;
        downloaded += chunk.len() as u64;
        file_bytes.extend_from_slice(&chunk);
        let _ = progress.send(UpdateProgress::Downloading { downloaded, total });
    }

    let _ = progress.send(UpdateProgress::Extracting);

    fs::write(&archive_path, &file_bytes)
        .map_err(|e| UpdateError(format!("Failed to write archive: {e}")))?;

    let extract_dir = tmp_dir.join("extracted");
    fs::create_dir_all(&extract_dir)
        .map_err(|e| UpdateError(format!("Failed to create extract dir: {e}")))?;

    extract_archive(&archive_path, &extract_dir, asset_name)?;

    let _ = progress.send(UpdateProgress::Installing);

    let binary_name = if cfg!(windows) { "cdcx.exe" } else { "cdcx" };
    let new_binary = find_binary_in_dir(&extract_dir, binary_name)?;

    replace_binary(&new_binary, &current_exe)?;

    let _ = fs::remove_dir_all(&tmp_dir);
    let _ = progress.send(UpdateProgress::Done);
    Ok(())
}

fn extract_archive(
    archive_path: &Path,
    extract_dir: &Path,
    asset_name: &str,
) -> Result<(), UpdateError> {
    let status = if cfg!(windows) && asset_name.ends_with(".zip") {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    archive_path.display(),
                    extract_dir.display()
                ),
            ])
            .status()
    } else {
        let args = if asset_name.ends_with(".zip") {
            vec!["xf"]
        } else {
            vec!["xzf"]
        };
        std::process::Command::new("tar")
            .args(args)
            .arg(archive_path)
            .arg("-C")
            .arg(extract_dir)
            .status()
    }
    .map_err(|e| UpdateError(format!("Failed to extract archive: {e}")))?;

    if !status.success() {
        return Err(UpdateError("Archive extraction failed".into()));
    }
    Ok(())
}

fn find_binary_in_dir(dir: &Path, name: &str) -> Result<PathBuf, UpdateError> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().map(|n| n == name).unwrap_or(false) {
                return Ok(path);
            }
            if path.is_dir() {
                if let Ok(found) = find_binary_in_dir(&path, name) {
                    return Ok(found);
                }
            }
        }
    }
    Err(UpdateError(format!("Binary '{name}' not found in archive")))
}

fn replace_binary(new_binary: &Path, current_exe: &Path) -> Result<(), UpdateError> {
    // Copy to a temp file in the same directory as the target (guarantees same mount point)
    let tmp_dest = current_exe.with_file_name(".cdcx.update.tmp");

    fs::copy(new_binary, &tmp_dest)
        .map_err(|e| UpdateError(format!("Failed to copy new binary: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp_dest, fs::Permissions::from_mode(0o755));
    }

    #[cfg(not(windows))]
    {
        fs::rename(&tmp_dest, current_exe).map_err(|e| {
            let _ = fs::remove_file(&tmp_dest);
            UpdateError(format!("Failed to replace binary: {e}"))
        })?;
    }

    #[cfg(windows)]
    {
        let old_path = current_exe.with_extension("exe.old");
        let _ = fs::remove_file(&old_path);
        fs::rename(current_exe, &old_path).map_err(|e| {
            let _ = fs::remove_file(&tmp_dest);
            UpdateError(format!("Failed to move old binary: {e}"))
        })?;
        if let Err(e) = fs::rename(&tmp_dest, current_exe) {
            let _ = fs::rename(&old_path, current_exe);
            let _ = fs::remove_file(&tmp_dest);
            return Err(UpdateError(format!("Failed to place new binary: {e}")));
        }
    }

    Ok(())
}

fn find_asset(
    assets: &[serde_json::Value],
    version: &str,
    target: &str,
) -> (Option<String>, Option<String>) {
    let expected_prefix = format!("cdcx-{version}-{target}");
    for asset in assets {
        if let Some(name) = asset["name"].as_str() {
            if name.starts_with(&expected_prefix)
                && (name.ends_with(".tar.gz") || name.ends_with(".zip"))
                && !name.ends_with(".sha256")
            {
                let url = asset["browser_download_url"].as_str().map(String::from);
                return (url, Some(name.to_string()));
            }
        }
    }
    (None, None)
}

pub fn current_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-musl"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-musl"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
    )))]
    {
        "unknown"
    }
}

fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let parts: Vec<&str> = s.splitn(3, '.').collect();
    if parts.len() != 3 {
        return None;
    }
    let patch_str = parts[2].split('-').next().unwrap_or(parts[2]);
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        patch_str.parse().ok()?,
    ))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[derive(Debug)]
pub struct UpdateError(pub String);

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for UpdateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_semver() {
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_semver("invalid"), None);
        assert_eq!(parse_semver("1.2"), None);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.2.0", "1.1.1"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.1.2", "1.1.1"));
        assert!(!is_newer("1.1.1", "1.1.1"));
        assert!(!is_newer("1.0.0", "1.1.1"));
        assert!(!is_newer("invalid", "1.1.1"));
    }

    #[test]
    fn test_current_target_is_known() {
        assert_ne!(current_target(), "unknown");
    }

    #[test]
    fn test_find_asset_matches() {
        let assets = vec![
            serde_json::json!({
                "name": "cdcx-1.2.0-x86_64-apple-darwin.tar.gz",
                "browser_download_url": "https://example.com/cdcx-1.2.0-x86_64-apple-darwin.tar.gz"
            }),
            serde_json::json!({
                "name": "cdcx-1.2.0-x86_64-apple-darwin.tar.gz.sha256",
                "browser_download_url": "https://example.com/cdcx-1.2.0-x86_64-apple-darwin.tar.gz.sha256"
            }),
            serde_json::json!({
                "name": "cdcx-1.2.0-aarch64-apple-darwin.tar.gz",
                "browser_download_url": "https://example.com/cdcx-1.2.0-aarch64-apple-darwin.tar.gz"
            }),
        ];

        let (url, name) = find_asset(&assets, "1.2.0", "x86_64-apple-darwin");
        assert_eq!(
            name.as_deref(),
            Some("cdcx-1.2.0-x86_64-apple-darwin.tar.gz")
        );
        assert!(url.is_some());

        let (url, _) = find_asset(&assets, "1.2.0", "x86_64-unknown-linux-musl");
        assert!(url.is_none());
    }

    #[test]
    fn test_find_asset_skips_sha256() {
        let assets = vec![serde_json::json!({
            "name": "cdcx-1.0.0-x86_64-apple-darwin.tar.gz.sha256",
            "browser_download_url": "https://example.com/sha256"
        })];
        let (url, _) = find_asset(&assets, "1.0.0", "x86_64-apple-darwin");
        assert!(url.is_none());
    }

    #[test]
    fn test_checker_should_check_no_cache() {
        let checker = UpdateChecker {
            meta_path: PathBuf::from("/tmp/cdcx-test-nonexistent-meta.json"),
            ttl: Duration::from_secs(86400),
        };
        assert!(checker.should_check());
    }
}
