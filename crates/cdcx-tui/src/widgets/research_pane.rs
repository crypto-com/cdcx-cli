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
    /// Index of the currently-highlighted news item within the active sub-tab.
    /// Reset on sub-tab switch or snapshot change; clamped against the list
    /// length at render time (the list can shrink between refreshes).
    news_selected: usize,
    /// When expanded, the selected item's body / description / URL renders
    /// inline below the row rather than just its title.
    news_expanded: bool,
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
            news_selected: 0,
            news_expanded: false,
            directory_requested: false,
            trending_requested: false,
        }
    }

    pub fn active_section(&self) -> Section {
        self.section
    }

    pub fn cycle_section_forward(&mut self) {
        self.section = self.section.next();
        self.reset_news_view();
    }

    pub fn cycle_section_backward(&mut self) {
        self.section = self.section.prev();
        self.reset_news_view();
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
        self.reset_news_view();
    }

    fn reset_news_view(&mut self) {
        self.news_selected = 0;
        self.news_expanded = false;
    }

    /// Move the news selection. Only active when Section::News is visible;
    /// otherwise the keystroke should fall through to the tab.
    pub fn select_news_next(&mut self) -> bool {
        if self.section != Section::News {
            return false;
        }
        let len = self.current_news_len();
        if len == 0 {
            return true;
        }
        self.news_selected = (self.news_selected + 1).min(len.saturating_sub(1));
        self.news_expanded = false;
        true
    }

    pub fn select_news_prev(&mut self) -> bool {
        if self.section != Section::News {
            return false;
        }
        self.news_selected = self.news_selected.saturating_sub(1);
        self.news_expanded = false;
        true
    }

    /// Toggle inline expansion of the selected news item. No-op outside
    /// News section. Returns true if the key was consumed.
    pub fn toggle_news_expand(&mut self) -> bool {
        if self.section != Section::News {
            return false;
        }
        if self.current_news_len() == 0 {
            return true;
        }
        self.news_expanded = !self.news_expanded;
        true
    }

    /// Collapse (only if currently expanded). Returns true if the key was
    /// consumed — so callers can distinguish "pane swallowed Esc" from
    /// "nothing to do, let the global handler run".
    pub fn collapse_news(&mut self) -> bool {
        if self.section != Section::News || !self.news_expanded {
            return false;
        }
        self.news_expanded = false;
        true
    }

    /// Returns the URL to open for the currently-selected news item, if any.
    /// Prefers external `link` when the post is a link-share; falls back to
    /// the reddit comment thread URL. For videos, builds a YouTube watch URL.
    pub fn selected_news_url(&self) -> Option<String> {
        if self.section != Section::News {
            return None;
        }
        match self.news_panel {
            NewsPanel::Reddit => {
                let post = self.snapshot.reddit.as_ref()?.get(self.news_selected)?;
                // Prefer the external link if it's a link-type post.
                post.link.clone().or_else(|| {
                    post.url
                        .as_ref()
                        .map(|u| format!("https://reddit.com{}", u))
                })
            }
            NewsPanel::Video => {
                let v = self.snapshot.videos.as_ref()?.get(self.news_selected)?;
                Some(format!("https://youtube.com/watch?v={}", v.id))
            }
            NewsPanel::Highlights => {
                // Highlights is a merged view — we'd need to know which list
                // the selected item came from, which is fragile. Punt: no URL
                // opening from highlights. Users press N to jump into Reddit
                // or Video first.
                None
            }
        }
    }

    fn current_news_len(&self) -> usize {
        match self.news_panel {
            NewsPanel::Reddit => self.snapshot.reddit.as_deref().map_or(0, |v| v.len()),
            NewsPanel::Video => self.snapshot.videos.as_deref().map_or(0, |v| v.len()),
            // Highlights pulls top-5 reddit + top-3 videos.
            NewsPanel::Highlights => {
                let r = self
                    .snapshot
                    .reddit
                    .as_deref()
                    .map_or(0, |v| v.len().min(5));
                let v = self
                    .snapshot
                    .videos
                    .as_deref()
                    .map_or(0, |v| v.len().min(3));
                r + v
            }
        }
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
        self.reset_news_view();

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

        // ── Footer: shortcuts, plus a beta badge so users remember this
        //    surface calls a consumer-web API that may change or break.
        let shortcuts = match self.section {
            Section::Overview => "[/]:section  r:refresh",
            Section::Chart => "[/]:section  r:refresh",
            Section::News => "[/]:section  N:subtab  J/K:nav  Enter:expand  b:open  r:refresh",
        };
        let footer = Line::from(vec![
            Span::styled(
                " BETA ",
                Style::default()
                    .fg(state.theme.colors.selected_fg)
                    .bg(state.theme.colors.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(shortcuts, Style::default().fg(state.theme.colors.muted)),
        ]);
        frame.render_widget(Paragraph::new(footer), footer_area);
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
        // Clamp selection against current list length at render time so a
        // new snapshot with fewer items can't leave the cursor past the end.
        let len = self.current_news_len();
        let selected = if len == 0 {
            0
        } else {
            self.news_selected.min(len - 1)
        };
        discover::render_news(
            frame,
            area,
            self.news_panel,
            self.snapshot.reddit.as_deref(),
            self.snapshot.videos.as_deref(),
            selected,
            self.news_expanded,
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

    fn post(id: &str, upvotes: i64, title: &str) -> RedditPost {
        RedditPost {
            id: id.into(),
            username: Some("alice".into()),
            upvotes,
            create_time: Some("2026-04-24T09:00:00".into()),
            title: title.into(),
            url: Some(format!("/r/Bitcoin/comments/{}/", id)),
            text: Some("body text".into()),
            link: None,
        }
    }

    #[test]
    fn news_selection_falls_through_when_section_not_news() {
        let mut pane = ResearchPane::new();
        // Default section is Overview.
        assert!(
            !pane.select_news_next(),
            "must return false so app.rs falls through to the tab"
        );
        assert!(!pane.select_news_prev());
    }

    #[test]
    fn news_selection_clamps_at_end_of_list() {
        let mut pane = ResearchPane::new();
        pane.section = Section::News;
        pane.news_panel = NewsPanel::Reddit;
        pane.snapshot.reddit = Some(vec![post("a", 10, "A"), post("b", 5, "B")]);
        assert!(pane.select_news_next());
        assert_eq!(pane.news_selected, 1);
        // Already at end — further next is a no-op, not a wraparound.
        assert!(pane.select_news_next());
        assert_eq!(pane.news_selected, 1);
    }

    #[test]
    fn news_moving_selection_collapses_expansion() {
        let mut pane = ResearchPane::new();
        pane.section = Section::News;
        pane.news_panel = NewsPanel::Reddit;
        pane.snapshot.reddit = Some(vec![post("a", 10, "A"), post("b", 5, "B")]);
        pane.news_expanded = true;
        pane.select_news_next();
        assert!(
            !pane.news_expanded,
            "moving selection must collapse expansion so the reader isn't stuck on stale body"
        );
    }

    #[test]
    fn collapse_returns_false_when_nothing_expanded() {
        let mut pane = ResearchPane::new();
        pane.section = Section::News;
        assert!(
            !pane.collapse_news(),
            "Esc must fall through when nothing to collapse — tab may want to handle it"
        );
    }

    #[test]
    fn reddit_self_post_resolves_to_thread_url() {
        let mut pane = ResearchPane::new();
        pane.section = Section::News;
        pane.news_panel = NewsPanel::Reddit;
        pane.snapshot.reddit = Some(vec![post("abc123", 50, "hello")]);
        let url = pane.selected_news_url().expect("url");
        assert_eq!(url, "https://reddit.com/r/Bitcoin/comments/abc123/");
    }

    #[test]
    fn reddit_link_post_prefers_external_link() {
        let mut pane = ResearchPane::new();
        pane.section = Section::News;
        pane.news_panel = NewsPanel::Reddit;
        let mut p = post("abc", 10, "t");
        p.link = Some("https://example.com/article".into());
        pane.snapshot.reddit = Some(vec![p]);
        let url = pane.selected_news_url().expect("url");
        assert_eq!(
            url, "https://example.com/article",
            "link-share posts should open the external article, not the reddit thread"
        );
    }

    #[test]
    fn video_resolves_to_youtube_watch_url() {
        let mut pane = ResearchPane::new();
        pane.section = Section::News;
        pane.news_panel = NewsPanel::Video;
        pane.snapshot.videos = Some(vec![VideoNews {
            id: "xYz123".into(),
            title: "t".into(),
            create_time: None,
            description: None,
        }]);
        let url = pane.selected_news_url().expect("url");
        assert_eq!(url, "https://youtube.com/watch?v=xYz123");
    }

    #[test]
    fn highlights_refuses_url_resolution() {
        // Merged list makes URL resolution ambiguous; the pane returns None
        // and app.rs surfaces a toast directing the user to the sub-tab.
        let mut pane = ResearchPane::new();
        pane.section = Section::News;
        pane.news_panel = NewsPanel::Highlights;
        pane.snapshot.reddit = Some(vec![post("a", 10, "t")]);
        assert!(pane.selected_news_url().is_none());
    }
}
