//! Bloomberg-style research pane that docks on the right side of the screen
//! in split view. Selection in the Market / Watchlist / Positions tab drives
//! the pane; it fans out queries to price-api.crypto.com and renders coin
//! metadata, ranges, social scores, exchange listings, news, and a price
//! chart — all stacked into sections cycled with `[` / `]`.
//!
//! Section independence matters: slow or failing endpoints only darken their
//! own panel, never the whole pane.

use std::sync::Arc;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use cdcx_core::price_api::{
    DirectoryEntry, MarketPair, PriceApiClient, RedditPost, SocialMetrics, StatisticsResponse,
    TrendingToken, VideoNews,
};

use crate::state::{AppState, PriceApiEvent};
use crate::theme::ThemeColors;
use crate::widgets::candlestick::{self, Candle};
use crate::widgets::discover::{self, NewsPanel};

const CONVERT: &str = "USD";

/// Which section of the research pane is currently visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Overview: price, ranges, social, listings stacked.
    Overview,
    /// Price chart (shares the 1h-candle store the market tab already keeps).
    Chart,
    /// News (Reddit + Video + Highlights, sub-tabbed).
    News,
}

impl Section {
    pub fn label(&self) -> &'static str {
        match self {
            Section::Overview => "Overview",
            Section::Chart => "Chart",
            Section::News => "News",
        }
    }

    pub const ORDER: &[Section] = &[Section::Overview, Section::Chart, Section::News];

    pub fn next(self) -> Self {
        let idx = Self::ORDER.iter().position(|s| *s == self).unwrap_or(0);
        Self::ORDER[(idx + 1) % Self::ORDER.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::ORDER.iter().position(|s| *s == self).unwrap_or(0);
        Self::ORDER[(idx + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }
}

/// Per-instrument snapshot. Entry is keyed by exchange-asset slug so the pane
/// can silently drop late responses from a previous selection.
#[derive(Debug, Default)]
struct Snapshot {
    entry: Option<DirectoryEntry>,
    statistics: Option<StatisticsResponse>,
    social: Option<SocialMetrics>,
    listings: Option<Vec<MarketPair>>,
    reddit: Option<Vec<RedditPost>>,
    videos: Option<Vec<VideoNews>>,
}

pub struct ResearchPane {
    instrument: Option<String>,
    snapshot: Snapshot,
    trending: Option<Vec<TrendingToken>>,
    section: Section,
    news_panel: NewsPanel,
    news_scroll: usize,
    directory_requested: bool,
    trending_requested: bool,
}

impl ResearchPane {
    pub fn new() -> Self {
        Self {
            instrument: None,
            snapshot: Snapshot::default(),
            trending: None,
            section: Section::Overview,
            news_panel: NewsPanel::Highlights,
            news_scroll: 0,
            directory_requested: false,
            trending_requested: false,
        }
    }

    pub fn active_section(&self) -> Section {
        self.section
    }

    pub fn cycle_section_forward(&mut self) {
        self.section = self.section.next();
        self.news_scroll = 0;
    }

    pub fn cycle_section_backward(&mut self) {
        self.section = self.section.prev();
        self.news_scroll = 0;
    }

    /// Scroll the news list (only meaningful when `Section::News` is active).
    pub fn scroll_news(&mut self, delta: isize) {
        if delta > 0 {
            self.news_scroll = self.news_scroll.saturating_add(delta as usize);
        } else {
            self.news_scroll = self.news_scroll.saturating_sub((-delta) as usize);
        }
    }

    /// Cycle the news sub-tab. No-op when News isn't the active section.
    pub fn cycle_news_subtab(&mut self) {
        if self.section != Section::News {
            return;
        }
        self.news_panel = match self.news_panel {
            NewsPanel::Highlights => NewsPanel::Reddit,
            NewsPanel::Reddit => NewsPanel::Video,
            NewsPanel::Video => NewsPanel::Highlights,
        };
        self.news_scroll = 0;
    }

    pub fn selected_instrument(&self) -> Option<&str> {
        self.instrument.as_deref()
    }

    /// Switch focus to a new instrument. Drops in-flight data + spawns a
    /// fresh fan-out. Called from app.rs on split-view selection changes.
    ///
    /// Resolution relies on `state.instrument_bases` (seeded from the
    /// exchange's `get-instruments` response) — naming conventions differ
    /// between CCY_PAIR (`BTC_USDT`), PERPETUAL_SWAP (`1INCHUSD-PERP`), and
    /// FUTURE (`BTCUSD-260424`), so we never parse the symbol. If the base
    /// isn't known yet (instruments list still loading), the pane just
    /// shows `Loading…` until a subsequent selection with known base.
    pub fn set_instrument(&mut self, instrument: String, state: &AppState) {
        if self.instrument.as_deref() == Some(instrument.as_str()) {
            return;
        }
        self.instrument = Some(instrument.clone());
        self.snapshot = Snapshot::default();
        self.news_scroll = 0;

        // First-time side effects: load trending + directory once per session.
        if !self.trending_requested {
            self.trending_requested = true;
            self.spawn_trending(state);
        }

        let Some(base_ccy) = state.instrument_bases.get(&instrument).cloned() else {
            // Instruments response hasn't landed yet. Leave snapshot empty;
            // the next selection change will retry.
            return;
        };

        if let Some(dir) = state.price_directory.as_ref() {
            if let Some(e) = dir.by_symbol(&base_ccy) {
                self.snapshot.entry = Some(e.clone());
            }
        }

        if self.snapshot.entry.is_some() {
            self.fanout(state);
        } else {
            self.spawn_resolve(base_ccy, state);
        }
    }

    /// Manual refresh. Called by `r` key when pane is in focus.
    pub fn refresh(&mut self, state: &AppState) {
        self.spawn_trending(state);
        if self.snapshot.entry.is_some() {
            self.fanout(state);
        }
    }

    pub fn apply_event(&mut self, event: &PriceApiEvent, state: &mut AppState) {
        match event {
            PriceApiEvent::Directory(dir) => {
                state.price_directory = Some(dir.clone());
                // Late-arriving directory: the user may have already selected
                // an instrument (or cycled through several) while we were
                // fetching. Re-resolve the current focus so the pane populates
                // rather than staying blank.
                if self.snapshot.entry.is_none() {
                    if let Some(inst) = self.instrument.clone() {
                        if let Some(base) = state.instrument_bases.get(&inst).cloned() {
                            if let Some(e) = dir.by_symbol(&base) {
                                self.snapshot.entry = Some(e.clone());
                                self.fanout(state);
                            }
                        }
                    }
                }
            }
            PriceApiEvent::Trending(list) => {
                self.trending = Some(list.clone());
            }
            PriceApiEvent::Resolved { instrument, entry } => {
                if self.instrument.as_deref() == Some(instrument.as_str()) {
                    self.snapshot.entry = entry.clone();
                    if entry.is_some() {
                        self.fanout(state);
                    }
                }
            }
            PriceApiEvent::Statistics { slug, data } => {
                if self.is_current_slug(slug) {
                    self.snapshot.statistics = Some(data.clone());
                }
            }
            PriceApiEvent::Social { slug, data } => {
                if self.is_current_slug(slug) {
                    self.snapshot.social = Some(data.clone());
                }
            }
            PriceApiEvent::MarketPairs { slug, data } => {
                if self.is_current_slug(slug) {
                    self.snapshot.listings = Some(data.clone());
                }
            }
            PriceApiEvent::SocialNews { token_id, data } => {
                if self.is_current_token_id(*token_id) {
                    self.snapshot.reddit = Some(data.clone());
                }
            }
            PriceApiEvent::VideoNews { token_id, data } => {
                if self.is_current_token_id(*token_id) {
                    self.snapshot.videos = Some(data.clone());
                }
            }
            PriceApiEvent::FetchError { kind, message } => {
                let short = if message.len() > 60 {
                    format!("{}…", &message[..57])
                } else {
                    message.clone()
                };
                state.toast(
                    format!("price-api {}: {}", kind, short),
                    crate::state::ToastStyle::Error,
                );
            }
        }
    }

    fn is_current_slug(&self, slug: &str) -> bool {
        self.snapshot
            .entry
            .as_ref()
            .map(|e| e.slug == slug)
            .unwrap_or(false)
    }

    fn is_current_token_id(&self, id: i64) -> bool {
        self.snapshot
            .entry
            .as_ref()
            .map(|e| e.id == id)
            .unwrap_or(false)
    }

    fn fanout(&mut self, state: &AppState) {
        let Some(entry) = self.snapshot.entry.clone() else {
            return;
        };
        let client = state.price_api.clone();
        let tx = state.price_api_tx.clone();
        spawn_statistics(entry.slug.clone(), client.clone(), tx.clone());
        spawn_social(entry.slug.clone(), client.clone(), tx.clone());
        spawn_pairs(entry.slug.clone(), client.clone(), tx.clone());
        spawn_social_news(entry.id, client.clone(), tx.clone());
        spawn_video_news(entry.id, client, tx);
    }

    /// Load (or refresh) the price-api directory and resolve a base-currency
    /// symbol to its directory entry. Only fires once per session — once the
    /// directory is in `state.price_directory`, subsequent resolutions use
    /// the in-memory lookup in `set_instrument`.
    fn spawn_resolve(&mut self, base_ccy: String, state: &AppState) {
        if self.directory_requested {
            return;
        }
        self.directory_requested = true;
        let client = state.price_api.clone();
        let tx = state.price_api_tx.clone();
        // Carry the current instrument through so `Resolved` can be matched
        // against the pane's focus by the stale-response guard.
        let instrument = self.instrument.clone().unwrap_or_default();
        tokio::spawn(async move {
            match cdcx_core::price_api::directory::load_or_refresh(&client).await {
                Ok(dir) => {
                    let entry = dir.by_symbol(&base_ccy).cloned();
                    let _ = tx.send(PriceApiEvent::Directory(dir));
                    let _ = tx.send(PriceApiEvent::Resolved { instrument, entry });
                }
                Err(e) => {
                    let _ = tx.send(PriceApiEvent::FetchError {
                        kind: "directory",
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    fn spawn_trending(&self, state: &AppState) {
        let client = state.price_api.clone();
        let tx = state.price_api_tx.clone();
        tokio::spawn(async move {
            match client.trending_tokens().await {
                Ok(data) => {
                    let _ = tx.send(PriceApiEvent::Trending(data));
                }
                Err(e) => {
                    let _ = tx.send(PriceApiEvent::FetchError {
                        kind: "trending",
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    /// Render the whole pane into `area`. `candles` is the 1h candle series
    /// from the Market tab — we render that directly rather than fetch our
    /// own, keeping chart data in one authoritative place.
    pub fn draw(&self, frame: &mut Frame, area: Rect, candles: &[Candle], state: &AppState) {
        // Outer block framing the whole right pane
        let outer = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(state.theme.colors.border));
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let [header_area, sub_nav_area, body_area, footer_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(inner);

        // ── Header: SYMBOL / Name / Rank / Live price / 24h change ──
        let (live_price, live_change) = self
            .instrument
            .as_deref()
            .and_then(|i| state.tickers.get(i))
            .map(|t| (Some(t.ask), Some(t.change_pct)))
            .unwrap_or((None, None));

        discover::render_header(
            frame,
            header_area,
            self.snapshot.entry.as_ref(),
            live_price,
            live_change,
            &state.theme.colors,
        );

        // ── Sub-nav strip: Overview | Chart | News ──
        let sub_spans: Vec<Span> = Section::ORDER
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                let style = if *s == self.section {
                    Style::default()
                        .fg(state.theme.colors.accent)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(state.theme.colors.muted)
                };
                let mut out = Vec::new();
                if i > 0 {
                    out.push(Span::styled(
                        "  ",
                        Style::default().fg(state.theme.colors.muted),
                    ));
                }
                out.push(Span::styled(s.label().to_string(), style));
                out
            })
            .collect();
        frame.render_widget(Paragraph::new(Line::from(sub_spans)), sub_nav_area);

        // ── Body: section-specific ──
        match self.section {
            Section::Overview => self.draw_overview(frame, body_area, &state.theme.colors),
            Section::Chart => self.draw_chart(frame, body_area, candles, state),
            Section::News => self.draw_news(frame, body_area, &state.theme.colors),
        }

        // ── Footer: shortcuts ──
        let shortcuts = match self.section {
            Section::Overview => "[/]:section  \u{2190}\u{2192}:trending  r:refresh",
            Section::Chart => "[/]:section  \u{2190}\u{2192}:trending  r:refresh",
            Section::News => "[/]:section  N:cycle news tab  j/k:scroll  r:refresh",
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                shortcuts,
                Style::default().fg(state.theme.colors.muted),
            ))),
            footer_area,
        );
    }

    fn draw_overview(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        // Grid:
        //   row1: Ranges (full width)
        //   row2: Social (full width)
        //   row3: Listings (fill)
        //   row4: Trending (fixed height)
        let [ranges_area, social_area, listings_area, trending_area] = Layout::vertical([
            Constraint::Length(11),
            Constraint::Length(5),
            Constraint::Fill(1),
            Constraint::Length(8),
        ])
        .areas(area);

        discover::render_ranges(
            frame,
            ranges_area,
            self.snapshot.statistics.as_ref(),
            CONVERT,
            colors,
        );
        discover::render_social(frame, social_area, self.snapshot.social.as_ref(), colors);
        discover::render_listings(
            frame,
            listings_area,
            self.snapshot.listings.as_deref(),
            8,
            colors,
        );
        discover::render_trending(frame, trending_area, self.trending.as_deref(), colors);
    }

    fn draw_chart(&self, frame: &mut Frame, area: Rect, candles: &[Candle], state: &AppState) {
        let instrument = self.instrument.as_deref().unwrap_or("—");
        let filled = candlestick::fill_candle_gaps(candles, 3_600_000);
        candlestick::draw_candlestick(
            frame,
            area,
            instrument,
            &filled,
            "1h",
            &state.theme.colors,
            "",
        );
    }

    fn draw_news(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        discover::render_news(
            frame,
            area,
            self.news_panel,
            self.snapshot.reddit.as_deref(),
            self.snapshot.videos.as_deref(),
            self.news_scroll,
            colors,
        );
    }
}

impl Default for ResearchPane {
    fn default() -> Self {
        Self::new()
    }
}

// Free-standing spawns so the pane's borrow on `&mut self` ends before tokio
// captures anything. Each task sends back a single event and terminates.

fn spawn_statistics(
    slug: String,
    client: Arc<PriceApiClient>,
    tx: tokio::sync::mpsc::UnboundedSender<PriceApiEvent>,
) {
    tokio::spawn(async move {
        match client.statistics(&slug, CONVERT).await {
            Ok(data) => {
                let _ = tx.send(PriceApiEvent::Statistics { slug, data });
            }
            Err(e) => {
                let _ = tx.send(PriceApiEvent::FetchError {
                    kind: "statistics",
                    message: e.to_string(),
                });
            }
        }
    });
}

fn spawn_social(
    slug: String,
    client: Arc<PriceApiClient>,
    tx: tokio::sync::mpsc::UnboundedSender<PriceApiEvent>,
) {
    tokio::spawn(async move {
        match client.social_metrics(&slug).await {
            Ok(data) => {
                let _ = tx.send(PriceApiEvent::Social { slug, data });
            }
            Err(e) => {
                let _ = tx.send(PriceApiEvent::FetchError {
                    kind: "social",
                    message: e.to_string(),
                });
            }
        }
    });
}

fn spawn_pairs(
    slug: String,
    client: Arc<PriceApiClient>,
    tx: tokio::sync::mpsc::UnboundedSender<PriceApiEvent>,
) {
    tokio::spawn(async move {
        match client.market_pairs(&slug).await {
            Ok(resp) => {
                let _ = tx.send(PriceApiEvent::MarketPairs {
                    slug,
                    data: resp.data,
                });
            }
            Err(e) => {
                let _ = tx.send(PriceApiEvent::FetchError {
                    kind: "market-pairs",
                    message: e.to_string(),
                });
            }
        }
    });
}

fn spawn_social_news(
    token_id: i64,
    client: Arc<PriceApiClient>,
    tx: tokio::sync::mpsc::UnboundedSender<PriceApiEvent>,
) {
    tokio::spawn(async move {
        match client.social_news(token_id).await {
            Ok(resp) => {
                let _ = tx.send(PriceApiEvent::SocialNews {
                    token_id,
                    data: resp.reddit_posts,
                });
            }
            Err(e) => {
                let _ = tx.send(PriceApiEvent::FetchError {
                    kind: "social-news",
                    message: e.to_string(),
                });
            }
        }
    });
}

fn spawn_video_news(
    token_id: i64,
    client: Arc<PriceApiClient>,
    tx: tokio::sync::mpsc::UnboundedSender<PriceApiEvent>,
) {
    tokio::spawn(async move {
        match client.video_news(token_id, 25).await {
            Ok(resp) => {
                let _ = tx.send(PriceApiEvent::VideoNews {
                    token_id,
                    data: resp.videos,
                });
            }
            Err(e) => {
                let _ = tx.send(PriceApiEvent::FetchError {
                    kind: "video-news",
                    message: e.to_string(),
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdcx_core::api_client::ApiClient;
    use cdcx_core::env::Environment;
    use cdcx_core::price_api::{PriceApiClient, StatisticsResponse};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn mk_state() -> AppState {
        let (rtx, _rrx) = mpsc::unbounded_channel();
        let (ptx, _prx) = mpsc::unbounded_channel();
        AppState {
            instruments: vec!["BTC_USDT".into()],
            instrument_types: HashMap::new(),
            instrument_bases: HashMap::new(),
            tickers: HashMap::new(),
            sparklines: HashMap::new(),
            alerts: vec![],
            authenticated: false,
            env: Environment::Production,
            theme: crate::theme::Theme::default(),
            terminal_size: (180, 60),
            market_connection: crate::state::ConnectionStatus::Connecting,
            user_connection: crate::state::ConnectionStatus::Error,
            api: Arc::new(ApiClient::new(None, Environment::Production)),
            rest_tx: rtx,
            price_api: Arc::new(PriceApiClient::new()),
            price_api_tx: ptx,
            price_directory: None,
            toast: None,
            session_start_value: None,
            current_portfolio_value: 0.0,
            ticker_speed_divisor: 2,
            price_flashes: HashMap::new(),
            paper_mode: false,
            paper_engine: None,
            volume_unit: crate::state::VolumeUnit::Usd,
            pending_navigation: None,
            isolated_positions: HashMap::new(),
            positions_snapshot: Vec::new(),
        }
    }

    #[test]
    fn section_cycles_forward() {
        let mut pane = ResearchPane::new();
        assert_eq!(pane.active_section(), Section::Overview);
        pane.cycle_section_forward();
        assert_eq!(pane.active_section(), Section::Chart);
        pane.cycle_section_forward();
        assert_eq!(pane.active_section(), Section::News);
        pane.cycle_section_forward();
        assert_eq!(pane.active_section(), Section::Overview);
    }

    #[test]
    fn section_cycles_backward() {
        let mut pane = ResearchPane::new();
        pane.cycle_section_backward();
        assert_eq!(pane.active_section(), Section::News);
    }

    #[test]
    fn stale_statistics_response_is_dropped() {
        let mut pane = ResearchPane::new();
        pane.snapshot.entry = Some(DirectoryEntry {
            id: 1027,
            slug: "ethereum".into(),
            symbol: "ETH".into(),
            name: "Ethereum".into(),
            rank: Some(2),
        });
        let mut state = mk_state();
        pane.apply_event(
            &PriceApiEvent::Statistics {
                slug: "bitcoin".into(),
                data: StatisticsResponse { statistics: vec![] },
            },
            &mut state,
        );
        assert!(
            pane.snapshot.statistics.is_none(),
            "late bitcoin response must not populate ethereum snapshot"
        );
    }

    #[test]
    fn matching_response_populates_snapshot() {
        let mut pane = ResearchPane::new();
        pane.snapshot.entry = Some(DirectoryEntry {
            id: 1,
            slug: "bitcoin".into(),
            symbol: "BTC".into(),
            name: "Bitcoin".into(),
            rank: Some(1),
        });
        let mut state = mk_state();
        pane.apply_event(
            &PriceApiEvent::Statistics {
                slug: "bitcoin".into(),
                data: StatisticsResponse { statistics: vec![] },
            },
            &mut state,
        );
        assert!(pane.snapshot.statistics.is_some());
    }

    #[test]
    fn news_subtab_cycle_is_noop_outside_news_section() {
        let mut pane = ResearchPane::new();
        pane.section = Section::Overview;
        let before = pane.news_panel;
        pane.cycle_news_subtab();
        assert_eq!(
            pane.news_panel, before,
            "cycling news sub-tab must not change state unless section is News"
        );
    }
}
