use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const DEFAULT_BASE_URL: &str = "https://exchange-developer.crypto.com/exchange/v1/openapi/";
const OPENAPI_FILENAME: &str = "exchange-openapi.generated.yaml";
const SCHEMA_FILENAME: &str = "exchange-schema.generated.yaml";

pub struct SpecFetcher {
    pub base_url: String,
    pub cache_path: PathBuf,
    pub meta_path: PathBuf,
    pub ttl: Duration,
}

impl Default for SpecFetcher {
    fn default() -> Self {
        let base_url = std::env::var("CDC_OPENAPI_URL")
            .ok()
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("cdcx");
        Self {
            base_url,
            cache_path: cache_dir.join("openapi-spec.yaml"),
            meta_path: cache_dir.join("openapi-meta.json"),
            ttl: Duration::from_secs(86400),
        }
    }
}

impl SpecFetcher {
    /// Returns true if cache file exists and was modified within TTL.
    pub fn cache_is_fresh(&self) -> bool {
        let Ok(metadata) = fs::metadata(&self.cache_path) else {
            return false;
        };
        let Ok(modified) = metadata.modified() else {
            return false;
        };
        let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
            return false;
        };
        elapsed < self.ttl
    }

    /// Returns cached spec content if file exists (regardless of freshness).
    pub fn load_cache(&self) -> Option<String> {
        fs::read_to_string(&self.cache_path).ok()
    }

    /// Writes spec content to cache file. Creates parent directories.
    pub fn write_cache(&self, content: &str) -> Result<(), io::Error> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.cache_path, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.cache_path, fs::Permissions::from_mode(0o644))?;
        }
        Ok(())
    }

    /// Writes metadata JSON alongside the cache.
    pub fn write_meta(&self, endpoint_count: usize) -> Result<(), io::Error> {
        if let Some(parent) = self.meta_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let meta = serde_json::json!({
            "fetched_at": chrono::Utc::now().to_rfc3339(),
            "endpoint_count": endpoint_count,
        });
        let meta_str = serde_json::to_string_pretty(&meta)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(&self.meta_path, meta_str)?;
        Ok(())
    }

    /// Reads previous endpoint count from metadata, if available.
    pub fn previous_endpoint_count(&self) -> Option<usize> {
        let content = fs::read_to_string(&self.meta_path).ok()?;
        let meta: serde_json::Value = serde_json::from_str(&content).ok()?;
        meta["endpoint_count"].as_u64().map(|n| n as usize)
    }

    /// Fetches the OpenAPI spec and schema from remote, concatenates into a single string.
    pub async fn fetch_remote(&self) -> Result<String, FetchError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| FetchError(format!("HTTP client error: {}", e)))?;

        let openapi_url = format!("{}{}", self.base_url, OPENAPI_FILENAME);
        let schema_url = format!("{}{}", self.base_url, SCHEMA_FILENAME);

        let openapi_resp = client
            .get(&openapi_url)
            .send()
            .await
            .map_err(|e| FetchError(format!("Failed to fetch {}: {}", openapi_url, e)))?
            .error_for_status()
            .map_err(|e| FetchError(format!("HTTP error fetching {}: {}", openapi_url, e)))?;
        let openapi_text = openapi_resp
            .text()
            .await
            .map_err(|e| FetchError(format!("Failed to read openapi response: {}", e)))?;

        let schema_resp = client
            .get(&schema_url)
            .send()
            .await
            .map_err(|e| FetchError(format!("Failed to fetch {}: {}", schema_url, e)))?
            .error_for_status()
            .map_err(|e| FetchError(format!("HTTP error fetching {}: {}", schema_url, e)))?;
        let schema_text = schema_resp
            .text()
            .await
            .map_err(|e| FetchError(format!("Failed to read schema response: {}", e)))?;

        Ok(format!(
            "{}\n{}\n{}",
            openapi_text,
            crate::openapi::parser::SCHEMA_SEPARATOR,
            schema_text
        ))
    }
}

#[derive(Debug)]
pub struct FetchError(pub String);

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FetchError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_fetcher() -> SpecFetcher {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let test_id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("cdcx-test-{}-{}", std::process::id(), test_id));
        fs::create_dir_all(&dir).unwrap();
        SpecFetcher {
            base_url: "https://example.com/openapi/".to_string(),
            cache_path: dir.join("openapi-spec.yaml"),
            meta_path: dir.join("openapi-meta.json"),
            ttl: Duration::from_secs(86400),
        }
    }

    #[test]
    fn test_no_cache_is_not_fresh() {
        let f = temp_fetcher();
        assert!(!f.cache_is_fresh());
        assert!(f.load_cache().is_none());
    }

    #[test]
    fn test_write_then_read_cache() {
        let f = temp_fetcher();
        f.write_cache("openapi: 3.0.3\npaths: {}").unwrap();
        assert!(f.cache_is_fresh());
        let content = f.load_cache().unwrap();
        assert!(content.contains("openapi: 3.0.3"));
        let _ = fs::remove_dir_all(f.cache_path.parent().unwrap());
    }

    #[test]
    fn test_stale_cache_is_not_fresh() {
        let f = SpecFetcher {
            ttl: Duration::from_secs(0),
            ..temp_fetcher()
        };
        f.write_cache("test").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(!f.cache_is_fresh());
        let content = f.load_cache();
        assert!(content.is_some());
        let parent = f.cache_path.parent().unwrap().to_path_buf();
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn test_write_and_read_meta() {
        let f = temp_fetcher();
        f.write_meta(75).unwrap();
        assert_eq!(f.previous_endpoint_count(), Some(75));
        let _ = fs::remove_dir_all(f.cache_path.parent().unwrap());
    }
}
