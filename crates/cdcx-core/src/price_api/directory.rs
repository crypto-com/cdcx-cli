use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::CdcxError;
use crate::price_api::client::PriceApiClient;

/// One entry keyed by exchange-asset symbol (e.g. "BTC"). `rank` is populated
/// from `all-ranks` after the token list is loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub id: i64,
    pub slug: String,
    pub symbol: String,
    pub name: String,
    pub rank: Option<i64>,
}

/// Cached directory mapping `BTC` -> slug `bitcoin` and token id `1`.
///
/// Lookups happen on-demand from the Discover tab; the directory is populated
/// via a single `refresh()` call at tab activation and re-used until the cache
/// file ages out. The full payload is ~4.5 MB but small once filtered to the
/// fields we care about — we persist the filtered form, not the raw response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenDirectory {
    pub entries: Vec<DirectoryEntry>,
}

impl TokenDirectory {
    /// Exchange-asset symbol → directory entry. Case-insensitive match against
    /// the upper-cased `symbol` field.
    pub fn by_symbol(&self, symbol: &str) -> Option<&DirectoryEntry> {
        let needle = symbol.to_uppercase();
        self.entries
            .iter()
            .find(|e| e.symbol.to_uppercase() == needle)
    }

    /// Resolve a Crypto.com Exchange instrument name like `BTC_USDT` to its
    /// base-asset entry. Falls back to `None` if the base asset isn't in the
    /// directory (e.g. perpetual-only instruments or quote-only listings).
    pub fn by_instrument(&self, instrument: &str) -> Option<&DirectoryEntry> {
        let base = instrument.split('_').next().unwrap_or(instrument);
        self.by_symbol(base)
    }

    pub fn default_cache_path() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("cdcx")
            .join("price-directory.json")
    }
}

const DIRECTORY_TTL: Duration = Duration::from_secs(86400);

/// Load from disk if fresh, otherwise fetch and write. Returns a stale cache
/// on fetch failure rather than erroring — the directory is a convenience
/// lookup and the rest of the tab degrades gracefully without it.
pub async fn load_or_refresh(client: &PriceApiClient) -> Result<TokenDirectory, CdcxError> {
    let path = TokenDirectory::default_cache_path();
    if let Some(dir) = load_from_disk(&path, DIRECTORY_TTL) {
        return Ok(dir);
    }
    match refresh(client).await {
        Ok(dir) => {
            let _ = write_to_disk(&path, &dir);
            Ok(dir)
        }
        Err(e) => {
            // Stale cache is better than nothing.
            if let Some(dir) = load_from_disk_any_age(&path) {
                tracing::warn!("price directory refresh failed, using stale cache: {}", e);
                return Ok(dir);
            }
            Err(e)
        }
    }
}

fn load_from_disk(path: &PathBuf, ttl: Duration) -> Option<TokenDirectory> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let elapsed = SystemTime::now().duration_since(modified).ok()?;
    if elapsed > ttl {
        return None;
    }
    load_from_disk_any_age(path)
}

fn load_from_disk_any_age(path: &PathBuf) -> Option<TokenDirectory> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice::<TokenDirectory>(&bytes).ok()
}

fn write_to_disk(path: &PathBuf, dir: &TokenDirectory) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec(dir).map_err(|e| std::io::Error::other(e.to_string()))?;
    fs::write(path, body)?;
    Ok(())
}

async fn refresh(client: &PriceApiClient) -> Result<TokenDirectory, CdcxError> {
    let tokens = client.all_tokens().await?;
    // Ranks are a separate endpoint; failure is non-fatal.
    let ranks_map: HashMap<String, i64> = match client.all_ranks().await {
        Ok(ranks) => ranks.into_iter().map(|r| (r.slug, r.rank)).collect(),
        Err(e) => {
            tracing::warn!("all-ranks fetch failed: {}", e);
            HashMap::new()
        }
    };
    let entries = tokens
        .data
        .into_iter()
        .map(|t| DirectoryEntry {
            rank: ranks_map.get(&t.slug).copied(),
            id: t.id,
            slug: t.slug,
            symbol: t.symbol,
            name: t.name,
        })
        .collect();
    Ok(TokenDirectory { entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_symbol_is_case_insensitive() {
        let dir = TokenDirectory {
            entries: vec![DirectoryEntry {
                id: 1,
                slug: "bitcoin".into(),
                symbol: "BTC".into(),
                name: "Bitcoin".into(),
                rank: Some(1),
            }],
        };
        assert_eq!(dir.by_symbol("BTC").unwrap().slug, "bitcoin");
        assert_eq!(dir.by_symbol("btc").unwrap().slug, "bitcoin");
        assert!(dir.by_symbol("ETH").is_none());
    }

    #[test]
    fn by_instrument_strips_quote() {
        let dir = TokenDirectory {
            entries: vec![DirectoryEntry {
                id: 1,
                slug: "bitcoin".into(),
                symbol: "BTC".into(),
                name: "Bitcoin".into(),
                rank: Some(1),
            }],
        };
        assert_eq!(dir.by_instrument("BTC_USDT").unwrap().slug, "bitcoin");
        assert_eq!(dir.by_instrument("BTC").unwrap().slug, "bitcoin");
    }
}
