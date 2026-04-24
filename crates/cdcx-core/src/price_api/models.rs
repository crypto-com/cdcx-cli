use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct TrendingToken {
    pub slug: String,
    pub symbol: String,
    pub name: String,
    pub token_id: i64,
    #[serde(default)]
    pub usd_price_change_24h: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatisticsResponse {
    pub statistics: Vec<StatisticPeriod>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatisticPeriod {
    pub period: String,
    pub high: f64,
    pub low: f64,
}

/// Follower counts for social platforms. Any platform may be absent.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SocialMetrics {
    #[serde(default)]
    pub reddit: Option<SocialPlatform>,
    #[serde(default)]
    pub twitter: Option<SocialPlatform>,
    #[serde(default)]
    pub telegram: Option<SocialPlatform>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SocialPlatform {
    #[serde(default)]
    pub followers: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketPairsResponse {
    #[serde(default)]
    pub data: Vec<MarketPair>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketPair {
    #[serde(default)]
    pub exchange_name: Option<String>,
    #[serde(default)]
    pub market_pair_name: Option<String>,
    #[serde(default)]
    pub quote_usd_price: Option<f64>,
    #[serde(default)]
    pub quote_usd_volume_24h: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SocialNewsResponse {
    #[serde(default)]
    pub reddit_posts: Vec<RedditPost>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedditPost {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub upvotes: i64,
    #[serde(default)]
    pub create_time: Option<String>,
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoNewsResponse {
    #[serde(default)]
    pub videos: Vec<VideoNews>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoNews {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub create_time: Option<String>,
}

/// Response from `meta/v2/all-tokens` — the slug/id directory.
#[derive(Debug, Clone, Deserialize)]
pub struct AllTokensResponse {
    pub data: Vec<TokenMeta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenMeta {
    pub id: i64,
    pub name: String,
    pub symbol: String,
    pub slug: String,
}

/// Response from `price/v1/all-ranks` — slug → rank.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenRank {
    pub slug: String,
    pub rank: i64,
}
