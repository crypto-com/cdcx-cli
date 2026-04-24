//! Client for price-api.crypto.com — the consumer-web API that powers
//! crypto.com/price. Not a supported developer API; surface is undocumented
//! and subject to change. Used by the TUI Discover tab for metadata, social
//! metrics, and news that the Exchange v1 API doesn't expose.
//!
//! Gated by `Origin: https://crypto.com` at the CDN layer.

pub mod client;
pub mod directory;
pub mod models;

pub use client::PriceApiClient;
pub use directory::{DirectoryEntry, TokenDirectory};
pub use models::*;
