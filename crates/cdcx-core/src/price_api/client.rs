use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::error::{CdcxError, ErrorEnvelope};
use crate::price_api::models::*;

const BASE: &str = "https://price-api.crypto.com";
/// The CDN gates responses on this Origin. Sending anything else (or nothing)
/// returns 403 or an empty body.
const ORIGIN: &str = "https://crypto.com";

pub struct PriceApiClient {
    http: Client,
    base_url: String,
}

impl PriceApiClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(format!("cdcx/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            http,
            base_url: BASE.to_string(),
        }
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into(),
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, CdcxError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.get(&url).header("Origin", ORIGIN).send().await?;
        if !resp.status().is_success() {
            return Err(CdcxError::Api(ErrorEnvelope::network(&format!(
                "price-api {} → {}",
                path,
                resp.status()
            ))));
        }
        let body = resp.bytes().await?;
        serde_json::from_slice::<T>(&body).map_err(CdcxError::from)
    }

    pub async fn trending_tokens(&self) -> Result<Vec<TrendingToken>, CdcxError> {
        self.get("/price/v1/trending-tokens").await
    }

    pub async fn statistics(
        &self,
        slug: &str,
        convert: &str,
    ) -> Result<StatisticsResponse, CdcxError> {
        self.get(&format!(
            "/price/v1/statistics/{}?convert={}",
            slug, convert
        ))
        .await
    }

    pub async fn social_metrics(&self, slug: &str) -> Result<SocialMetrics, CdcxError> {
        self.get(&format!("/market/v1/token/{}/social-metrics", slug))
            .await
    }

    pub async fn market_pairs(&self, slug: &str) -> Result<MarketPairsResponse, CdcxError> {
        self.get(&format!("/market/v1/token/{}/market-pairs", slug))
            .await
    }

    pub async fn social_news(&self, token_id: i64) -> Result<SocialNewsResponse, CdcxError> {
        self.get(&format!(
            "/market/v2/token/{}/social-news?platform=all",
            token_id
        ))
        .await
    }

    pub async fn video_news(
        &self,
        token_id: i64,
        size: u32,
    ) -> Result<VideoNewsResponse, CdcxError> {
        self.get(&format!(
            "/market/v2/token/{}/news?platform=video&page=1&size={}",
            token_id, size
        ))
        .await
    }

    pub async fn all_tokens(&self) -> Result<AllTokensResponse, CdcxError> {
        self.get("/meta/v2/all-tokens").await
    }

    pub async fn all_ranks(&self) -> Result<Vec<TokenRank>, CdcxError> {
        self.get("/price/v1/all-ranks").await
    }
}

impl Default for PriceApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn trending_tokens_parses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/price/v1/trending-tokens"))
            .and(header("Origin", "https://crypto.com"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"slug": "bitcoin", "symbol": "BTC", "name": "Bitcoin", "token_id": 1, "usd_price_change_24h": 0.012}
            ])))
            .mount(&server)
            .await;

        let client = PriceApiClient::with_base_url(server.uri());
        let out = client.trending_tokens().await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "bitcoin");
        assert!((out[0].usd_price_change_24h - 0.012).abs() < 1e-9);
    }

    #[tokio::test]
    async fn social_metrics_handles_null_platform() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/market/v1/token/bitcoin/social-metrics"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "reddit": {"followers": 4908145},
                "twitter": {"followers": 5749255},
                "telegram": {"followers": null}
            })))
            .mount(&server)
            .await;

        let client = PriceApiClient::with_base_url(server.uri());
        let m = client.social_metrics("bitcoin").await.unwrap();
        assert_eq!(m.reddit.unwrap().followers, Some(4908145));
        assert_eq!(m.telegram.unwrap().followers, None);
    }

    #[tokio::test]
    async fn http_error_surfaces_as_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/price/v1/trending-tokens"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let client = PriceApiClient::with_base_url(server.uri());
        let err = client.trending_tokens().await.unwrap_err();
        match err {
            CdcxError::Api(env) => assert!(env.message.contains("403")),
            other => panic!("expected Api, got {:?}", other),
        }
    }
}
