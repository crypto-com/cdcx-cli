use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Color;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use std::collections::{HashMap, VecDeque};

/// Maximum number of recent trades retained for the detail-view panel.
/// Sized above any realistic terminal height so the panel never visibly
/// truncates from this cap — the render path re-clips to visible rows.
const RECENT_TRADES_CAP: usize = 100;

/// Maximum number of instruments' candle series retained in-memory.
/// Chart and Compare views insert one entry per instrument viewed in a
/// session; without a cap this grows unbounded (issue #25). Eviction is
/// LRU — least-recently-viewed instrument is dropped when a new one
/// arrives and the cap is reached.
const MAX_CANDLE_INSTRUMENTS: usize = 20;

/// Order-book depth presets the user cycles through with `D` in Detail view.
/// Values come from the Crypto.com Exchange WS `book.{instrument}.{depth}`
/// channel spec — 10/50/150 are the documented buckets. Using anything else
/// (e.g. 20) works for the REST snapshot but leaves the WS stream silent
/// because the subscription would be invalid.
const BOOK_DEPTH_PRESETS: &[usize] = &[10, 50, 150];
const DEFAULT_BOOK_DEPTH: usize = 50;

use crate::format::{format_compact, format_price};
use crate::state::{AppState, RestRequest};
use crate::tabs::{DataEvent, Tab, TabKind};
use crate::widgets::candlestick::{
    draw_candlestick, draw_compare_charts, fill_candle_gaps, Candle,
};
use crate::widgets::detail_view::draw_detail;
use crate::widgets::instrument_picker::{InstrumentPicker, PickerResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Table,
    Detail,
    Chart,
    Compare,
}

/// Returns the display label for an API inst_type value.
fn category_label(inst_type: &str) -> &str {
    match inst_type {
        "CCY_PAIR" => "Spot",
        "PERPETUAL_SWAP" => "Perpetual",
        "FUTURE" => "Futures",
        other => other,
    }
}

/// Preferred display order for known categories; unknown types sort alphabetically after.
fn category_sort_key(inst_type: &str) -> (u8, &str) {
    match inst_type {
        "CCY_PAIR" => (0, ""),
        "PERPETUAL_SWAP" => (1, ""),
        "FUTURE" => (2, ""),
        other => (3, other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Volume,
    Change,
    Price,
    Name,
}

impl SortField {
    pub const ALL: &[SortField] = &[
        SortField::Volume,
        SortField::Change,
        SortField::Price,
        SortField::Name,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SortField::Volume => "Vol",
            SortField::Change => "24h",
            SortField::Price => "Price",
            SortField::Name => "A-Z",
        }
    }
}

const TIMEFRAMES: &[&str] = &["1m", "5m", "15m", "30m", "1h", "4h", "6h", "12h", "1D"];

/// Which side of the book the cursor is on, plus the zero-based index into
/// that side's visible levels (0 = best / closest to mid).
///
/// Kept as a standalone enum (rather than an `Option<(Side, usize)>` on
/// `MarketTab`) so the movement rules can be expressed and unit-tested as
/// pure functions without dragging `AppState` into the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookCursor {
    Ask(usize),
    Bid(usize),
}

/// Move the cursor down one level. `ask_len` / `bid_len` are the current
/// visible level counts on each side (so the cursor can never point past
/// what's rendered).
///
/// Rule: ↓ means "move visually down on screen". Asks render with deepest
/// at top and best at bottom, so ↓ on an ask means *toward the mid*
/// (smaller index). Crossing the best ask drops the cursor onto the best
/// bid. On bids, ↓ moves away from the mid (bigger index), clamping at
/// the deepest visible bid.
fn cursor_down(cursor: BookCursor, _ask_len: usize, bid_len: usize) -> BookCursor {
    match cursor {
        BookCursor::Ask(0) => {
            if bid_len > 0 {
                BookCursor::Bid(0)
            } else {
                BookCursor::Ask(0)
            }
        }
        BookCursor::Ask(i) => BookCursor::Ask(i - 1),
        BookCursor::Bid(i) if i + 1 < bid_len => BookCursor::Bid(i + 1),
        BookCursor::Bid(i) => BookCursor::Bid(i),
    }
}

/// Move the cursor up one level. Mirror of `cursor_down` — on asks ↑ moves
/// *away* from the mid (bigger index); on bids ↑ moves *toward* the mid
/// (smaller index); crossing the best bid lands on the best ask.
fn cursor_up(cursor: BookCursor, ask_len: usize, _bid_len: usize) -> BookCursor {
    match cursor {
        BookCursor::Bid(0) => {
            if ask_len > 0 {
                BookCursor::Ask(0)
            } else {
                BookCursor::Bid(0)
            }
        }
        BookCursor::Bid(i) => BookCursor::Bid(i - 1),
        BookCursor::Ask(i) if i + 1 < ask_len => BookCursor::Ask(i + 1),
        BookCursor::Ask(i) => BookCursor::Ask(i),
    }
}

/// After a book/depth change the previous cursor position may point past
/// the new list end. Clamp to a valid index, preserving the side. Returns
/// `None` if the target side has zero levels (caller should clear cursor).
fn clamp_cursor(cursor: BookCursor, ask_len: usize, bid_len: usize) -> Option<BookCursor> {
    match cursor {
        BookCursor::Ask(_) if ask_len == 0 && bid_len > 0 => Some(BookCursor::Bid(0)),
        BookCursor::Ask(_) if ask_len == 0 => None,
        BookCursor::Ask(i) => Some(BookCursor::Ask(i.min(ask_len - 1))),
        BookCursor::Bid(_) if bid_len == 0 && ask_len > 0 => Some(BookCursor::Ask(0)),
        BookCursor::Bid(_) if bid_len == 0 => None,
        BookCursor::Bid(i) => Some(BookCursor::Bid(i.min(bid_len - 1))),
    }
}

/// Running sums from top of book (best) down to (and including) `idx`,
/// returned as `(cumulative_qty, cumulative_notional)`. The caller supplies
/// the appropriate side's `(price, qty)` pairs already in top-down order.
/// Notional = Σ price × qty, computed level-by-level so rounding matches
/// the user's mental model (not `cum_qty * last_price`).
pub fn cumulative_at(levels: &[(f64, f64)], idx: usize) -> (f64, f64) {
    let mut cum_qty = 0.0;
    let mut cum_notional = 0.0;
    for (i, (price, qty)) in levels.iter().enumerate() {
        cum_qty += qty;
        cum_notional += price * qty;
        if i == idx {
            break;
        }
    }
    (cum_qty, cum_notional)
}

/// Distance from `mid` in basis points (1 bp = 0.01%). Positive for prices
/// above mid (asks), negative below (bids). Returns 0.0 if `mid` is 0 to
/// avoid div-by-zero during the brief moment before the ticker arrives.
pub fn bps_from_mid(price: f64, mid: f64) -> f64 {
    if mid == 0.0 {
        return 0.0;
    }
    (price - mid) / mid * 10_000.0
}

pub struct MarketTab {
    categories: Vec<String>, // dynamic inst_type values from API
    category: usize,
    sort_field: usize,
    sort_asc: bool,
    selected: usize,
    scroll_offset: usize,
    search_query: String,
    searching: bool,
    filtered_instruments: Vec<String>,
    view_mode: ViewMode,
    detail_instrument: String,
    book_data: Option<serde_json::Value>,
    /// Current depth preset for the order-book REST fetch + WS subscription.
    /// Cycled with `D` on Detail view. Session-scoped — no tui.toml round-trip.
    book_depth: usize,
    /// Cursor selecting a specific level in the order book for cumulative
    /// inspection. `None` = no cursor → pressure bar renders as before;
    /// `Some` = selected row is highlighted and the context line shows
    /// cumulative qty / notional / distance-from-mid for that level.
    /// Cleared on Esc, on leaving Detail, and on instrument switch.
    book_cursor: Option<BookCursor>,
    /// Rolling buffer of recent trades, newest at the front. Seeded from the
    /// REST `public/get-trades` response, then updated by streaming batches.
    recent_trades: VecDeque<serde_json::Value>,
    candle_data: Option<serde_json::Value>,
    // Streaming candle storage per instrument.
    // `candles` holds the data; `candle_access_order` tracks LRU eviction
    // order so the map can't grow unbounded across a long session
    // (issue #25). Back of the deque = most-recently-accessed.
    candles: HashMap<String, Vec<Candle>>,
    candle_access_order: VecDeque<String>,
    timeframe: String,
    heatmap: bool,
    // Compare view state
    compare_instruments: Vec<String>,
    compare_selected: usize,
    picker: Option<InstrumentPicker>,
    navigated_from: Option<TabKind>,
}

impl Default for MarketTab {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketTab {
    pub fn new() -> Self {
        Self {
            categories: vec!["CCY_PAIR".into(), "PERPETUAL_SWAP".into(), "FUTURE".into()],
            category: 0,
            sort_field: 0,
            sort_asc: false,
            selected: 0,
            scroll_offset: 0,
            search_query: String::new(),
            searching: false,
            filtered_instruments: vec![],
            view_mode: ViewMode::Table,
            detail_instrument: String::new(),
            book_data: None,
            book_depth: DEFAULT_BOOK_DEPTH,
            book_cursor: None,
            recent_trades: VecDeque::with_capacity(RECENT_TRADES_CAP),
            candle_data: None,
            candles: HashMap::new(),
            candle_access_order: VecDeque::with_capacity(MAX_CANDLE_INSTRUMENTS),
            timeframe: "1h".into(),
            heatmap: false,
            compare_instruments: vec![],
            compare_selected: 0,
            picker: None,
            navigated_from: None,
        }
    }

    pub fn selected_instrument(&self) -> Option<&str> {
        self.filtered_instruments
            .get(self.selected)
            .map(|s| s.as_str())
    }

    fn enter_detail(&mut self, state: &AppState) {
        if let Some(inst) = self.selected_instrument().map(String::from) {
            self.navigated_from = None;
            self.enter_detail_for(&inst, state);
        }
    }

    /// Open the detail view for a specific instrument (used by cross-tab navigation).
    pub fn enter_detail_for(&mut self, instrument: &str, state: &AppState) {
        self.detail_instrument = instrument.to_string();
        self.view_mode = ViewMode::Detail;
        self.book_data = None;
        self.book_cursor = None;
        self.recent_trades.clear();
        let _ = state.rest_tx.send(RestRequest {
            method: "public/get-book".into(),
            params: serde_json::json!({
                "instrument_name": self.detail_instrument,
                "depth": self.book_depth.to_string(),
            }),
            is_private: false,
        });
        let _ = state.rest_tx.send(RestRequest {
            method: "public/get-trades".into(),
            params: serde_json::json!({"instrument_name": self.detail_instrument}),
            is_private: false,
        });
    }

    /// Replace the recent-trades buffer with a fresh REST snapshot. The
    /// Exchange `public/get-trades` endpoint returns trades newest-first, so
    /// we copy in order and truncate to the cap.
    fn seed_recent_trades(&mut self, arr: &[serde_json::Value]) {
        self.recent_trades.clear();
        for trade in arr.iter().take(RECENT_TRADES_CAP) {
            self.recent_trades.push_back(trade.clone());
        }
    }

    /// Prepend a streaming trade batch to the buffer. Iterating in reverse so
    /// the newest trade in the batch ends up at the front (position 0).
    /// Truncates to RECENT_TRADES_CAP, dropping the oldest trades.
    fn prepend_recent_trades(&mut self, arr: &[serde_json::Value]) {
        for trade in arr.iter().rev() {
            self.recent_trades.push_front(trade.clone());
        }
        while self.recent_trades.len() > RECENT_TRADES_CAP {
            self.recent_trades.pop_back();
        }
    }

    fn enter_chart(&mut self, state: &AppState) {
        let inst = if self.view_mode == ViewMode::Detail {
            self.detail_instrument.clone()
        } else {
            self.selected_instrument().unwrap_or("BTC_USDT").to_string()
        };
        self.detail_instrument = inst.clone();
        self.view_mode = ViewMode::Chart;
        self.candle_data = None;
        self.fetch_candles(&inst, state);
    }

    fn enter_compare(&mut self, state: &AppState) {
        let inst = self.selected_instrument().unwrap_or("BTC_USDT").to_string();
        if self.compare_instruments.is_empty() {
            self.compare_instruments = vec![inst];
        }
        self.view_mode = ViewMode::Compare;
        self.compare_selected = 0;
        self.picker = None;
        for inst in &self.compare_instruments.clone() {
            self.fetch_candles(inst, state);
        }
    }

    /// Pure next-preset lookup — extracted from `cycle_book_depth` so the
    /// preset cycle can be unit-tested without constructing an `AppState`.
    /// If `current` is not in the preset list, falls back to the first preset
    /// (keeps the tab recoverable after a stale value).
    fn next_book_depth(current: usize) -> usize {
        let idx = BOOK_DEPTH_PRESETS
            .iter()
            .position(|&d| d == current)
            .map(|i| (i + 1) % BOOK_DEPTH_PRESETS.len())
            .unwrap_or(0);
        BOOK_DEPTH_PRESETS[idx]
    }

    /// Format the WS book channel for the current detail instrument. Kept as
    /// an explicit helper so subscription wiring is grep-visible and the
    /// depth-suffix contract is testable without a full `AppState`.
    fn book_channel(&self) -> String {
        format!("book.{}.{}", self.detail_instrument, self.book_depth)
    }

    /// Re-validate `book_cursor` against the current level counts. Called
    /// after each `book_data` refresh — depth changes and WS snapshots can
    /// shrink either side, and a cursor pointing past the end would render
    /// garbage + compute wrong cumulative stats.
    fn clamp_book_cursor(&mut self) {
        if let Some(cur) = self.book_cursor {
            let (ask_len, bid_len) = self.book_level_counts();
            self.book_cursor = clamp_cursor(cur, ask_len, bid_len);
        }
    }

    /// Return the current visible `(ask_count, bid_count)` from `book_data`.
    /// Called by the cursor-movement code, which needs to know the valid
    /// index range; falls back to `(0, 0)` if the book hasn't arrived yet.
    /// `book_depth` caps each side — we don't show more than that even if
    /// the exchange sends extras.
    fn book_level_counts(&self) -> (usize, usize) {
        let Some(raw) = self.book_data.as_ref() else {
            return (0, 0);
        };
        let Some(book) = raw
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
        else {
            return (0, 0);
        };
        let ask_len = book
            .get("asks")
            .and_then(|v| v.as_array())
            .map(|a| a.len().min(self.book_depth))
            .unwrap_or(0);
        let bid_len = book
            .get("bids")
            .and_then(|v| v.as_array())
            .map(|b| b.len().min(self.book_depth))
            .unwrap_or(0);
        (ask_len, bid_len)
    }

    /// Handle ↓ on the Detail view: activate the cursor at the best ask if
    /// it's not yet active, otherwise move it down one level. Returns true
    /// if the cursor did anything — i.e. whether the keystroke was
    /// consumed, so the caller can decide whether to fall through.
    fn cursor_move_down(&mut self) -> bool {
        let (ask_len, bid_len) = self.book_level_counts();
        if ask_len == 0 && bid_len == 0 {
            return false;
        }
        self.book_cursor = Some(match self.book_cursor {
            None => {
                if ask_len > 0 {
                    BookCursor::Ask(0)
                } else {
                    BookCursor::Bid(0)
                }
            }
            Some(c) => cursor_down(c, ask_len, bid_len),
        });
        true
    }

    /// Mirror of `cursor_move_down`: ↑ activates at best bid if dormant,
    /// otherwise steps one level up.
    fn cursor_move_up(&mut self) -> bool {
        let (ask_len, bid_len) = self.book_level_counts();
        if ask_len == 0 && bid_len == 0 {
            return false;
        }
        self.book_cursor = Some(match self.book_cursor {
            None => {
                if bid_len > 0 {
                    BookCursor::Bid(0)
                } else {
                    BookCursor::Ask(0)
                }
            }
            Some(c) => cursor_up(c, ask_len, bid_len),
        });
        true
    }

    /// Advance `book_depth` to the next preset and re-fetch the snapshot.
    /// The WS subscription string also changes as a side-effect —
    /// `subscriptions()` rebuilds the list on the next tick and the app's
    /// diff logic will resub the book channel to the new depth suffix.
    fn cycle_book_depth(&mut self, state: &AppState) {
        self.book_depth = Self::next_book_depth(self.book_depth);
        self.book_data = None;
        let _ = state.rest_tx.send(RestRequest {
            method: "public/get-book".into(),
            params: serde_json::json!({
                "instrument_name": self.detail_instrument,
                "depth": self.book_depth.to_string(),
            }),
            is_private: false,
        });
    }

    fn cycle_timeframe(&mut self, direction: i32, state: &AppState) {
        let current_idx = TIMEFRAMES
            .iter()
            .position(|&t| t == self.timeframe)
            .unwrap_or(4); // default 1h
        let new_idx = if direction > 0 {
            (current_idx + 1).min(TIMEFRAMES.len() - 1)
        } else {
            current_idx.saturating_sub(1)
        };
        self.timeframe = TIMEFRAMES[new_idx].to_string();
        // Clear existing candle data and refetch — keep the two LRU
        // structures in lockstep.
        self.candles.clear();
        self.candle_access_order.clear();
        match self.view_mode {
            ViewMode::Chart => {
                self.fetch_candles(&self.detail_instrument.clone(), state);
            }
            ViewMode::Compare => {
                for inst in &self.compare_instruments.clone() {
                    self.fetch_candles(inst, state);
                }
            }
            _ => {}
        }
    }

    fn timeframe_ms(&self) -> u64 {
        match self.timeframe.as_str() {
            "1m" => 60_000,
            "5m" => 300_000,
            "15m" => 900_000,
            "30m" => 1_800_000,
            "1h" => 3_600_000,
            "4h" => 14_400_000,
            "6h" => 21_600_000,
            "12h" => 43_200_000,
            "1D" => 86_400_000,
            _ => 3_600_000, // default 1h
        }
    }

    /// Promote `instrument` to most-recently-accessed in the LRU order.
    /// If inserting it would exceed MAX_CANDLE_INSTRUMENTS, evict the
    /// oldest entry from both the order deque and the data map.
    ///
    /// Call this every time a candle entry is inserted or updated.
    fn touch_candles(&mut self, instrument: &str) {
        // Remove existing position (if any) — we'll push to MRU at the end.
        if let Some(pos) = self
            .candle_access_order
            .iter()
            .position(|i| i == instrument)
        {
            self.candle_access_order.remove(pos);
        }
        self.candle_access_order.push_back(instrument.to_string());

        // Evict oldest if we're over cap. Use `while` so we catch any
        // rare case where the cap was tightened or state was loaded stale.
        while self.candle_access_order.len() > MAX_CANDLE_INSTRUMENTS {
            if let Some(evict) = self.candle_access_order.pop_front() {
                self.candles.remove(&evict);
            }
        }
    }

    fn fetch_candles(&self, instrument: &str, state: &AppState) {
        // Request enough candles to fill the terminal width
        // Each candle is 3 chars wide, price label is 10 chars
        let max_candles = (state.terminal_size.0 as usize).saturating_sub(11) / 3;
        let count = max_candles.clamp(50, 300); // at least 50, API max 300
        let _ = state.rest_tx.send(RestRequest {
            method: "public/get-candlestick".into(),
            params: serde_json::json!({
                "instrument_name": instrument,
                "timeframe": self.timeframe,
                "count": count.to_string(),
            }),
            is_private: false,
        });
    }

    /// Fetch 1h candles for currently visible instruments to build 24h sparklines.
    /// Only fetches for instruments that don't already have sparkline data.
    fn fetch_sparkline_candles(&self, state: &AppState) {
        let vis_height = state.terminal_size.1.saturating_sub(8) as usize; // approx visible rows
        let end = (self.scroll_offset + vis_height).min(self.filtered_instruments.len());
        let visible = &self.filtered_instruments[self.scroll_offset..end];

        for inst in visible {
            if state
                .sparklines
                .get(inst)
                .map(|v| v.len() >= 2)
                .unwrap_or(false)
            {
                continue; // already have data
            }
            let _ = state.rest_tx.send(RestRequest {
                method: "sparkline-candles".into(),
                params: serde_json::json!({
                    "instrument_name": inst,
                    "timeframe": "1h",
                }),
                is_private: false,
            });
        }
    }

    fn visible_rows(&self, area_height: u16) -> usize {
        // When called from on_key with terminal_height - 4:
        //   terminal_height - 4 (ticker+tab_bar) - 4 (header+table_hdr+footer+status) = data rows
        // When called from draw with table_area.height:
        //   callers must subtract 1 for the table header themselves or use
        //   visible_rows_in_table() instead.
        (area_height as usize).saturating_sub(4)
    }

    fn visible_rows_in_table(&self, table_area_height: u16) -> usize {
        // table_area only contains the Table widget: 1 header row + N data rows
        (table_area_height as usize).saturating_sub(1)
    }

    /// Rebuild categories from instrument type data. Called after instruments are loaded.
    fn rebuild_categories(&mut self, state: &AppState) {
        let mut types: Vec<String> = state
            .instrument_types
            .values()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .cloned()
            .collect();
        types.sort_by(|a, b| category_sort_key(a).cmp(&category_sort_key(b)));
        if !types.is_empty() {
            self.categories = types;
            if self.category >= self.categories.len() {
                self.category = 0;
            }
        }
    }

    fn refilter(&mut self, state: &AppState) {
        // Rebuild dynamic categories from API inst_type data
        if !state.instrument_types.is_empty() {
            self.rebuild_categories(state);
        }
        let active_type = self
            .categories
            .get(self.category)
            .cloned()
            .unwrap_or_default();
        self.filtered_instruments = state
            .instruments
            .iter()
            .filter(|i| {
                state
                    .instrument_types
                    .get(*i)
                    .map(|t| *t == active_type)
                    .unwrap_or(false)
            })
            .filter(|i| {
                if self.search_query.is_empty() {
                    true
                } else {
                    i.to_uppercase().contains(&self.search_query.to_uppercase())
                }
            })
            .cloned()
            .collect();
        self.resort(state);
        // Fetch 24h candle data for sparklines
        self.fetch_sparkline_candles(state);
    }

    fn resort(&mut self, state: &AppState) {
        let field = SortField::ALL[self.sort_field];
        let asc = self.sort_asc;

        self.filtered_instruments.sort_by(|a, b| {
            let ta = state.tickers.get(a);
            let tb = state.tickers.get(b);

            let cmp = match field {
                SortField::Volume => {
                    let va = ta.map(|t| t.volume_usd).unwrap_or(0.0);
                    let vb = tb.map(|t| t.volume_usd).unwrap_or(0.0);
                    va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortField::Change => {
                    let va = ta.map(|t| t.change_pct).unwrap_or(0.0);
                    let vb = tb.map(|t| t.change_pct).unwrap_or(0.0);
                    va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortField::Price => {
                    let va = ta.map(|t| t.ask).unwrap_or(0.0);
                    let vb = tb.map(|t| t.ask).unwrap_or(0.0);
                    va.partial_cmp(&vb).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortField::Name => a.cmp(b),
            };
            if asc {
                cmp
            } else {
                cmp.reverse()
            }
        });
    }
}

impl Tab for MarketTab {
    fn is_editing(&self) -> bool {
        // Suppress global hotkeys whenever a text-input overlay is open —
        // otherwise a character like 'p' would toggle paper mode instead of
        // filtering the picker/search query.
        self.searching || self.picker.is_some()
    }

    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool {
        // Detail/chart/compare sub-views
        match self.view_mode {
            ViewMode::Detail => {
                return match key.code {
                    KeyCode::Esc => {
                        // If a book cursor is active, first Esc clears it
                        // (so the user isn't ejected from Detail while
                        // inspecting levels). Second Esc exits Detail.
                        if self.book_cursor.is_some() {
                            self.book_cursor = None;
                            return true;
                        }
                        self.view_mode = ViewMode::Table;
                        if let Some(origin) = self.navigated_from.take() {
                            state.pending_return_tab = Some(origin);
                        }
                        true
                    }
                    KeyCode::Down => self.cursor_move_down(),
                    KeyCode::Up => self.cursor_move_up(),
                    KeyCode::Char('k') => {
                        self.book_cursor = None;
                        self.enter_chart(state);
                        true
                    }
                    KeyCode::Char('m') => {
                        self.book_cursor = None;
                        self.enter_compare(state);
                        true
                    }
                    KeyCode::Char('D') => {
                        // Depth change invalidates the cursor's index bounds.
                        self.book_cursor = None;
                        self.cycle_book_depth(state);
                        true
                    }
                    _ => false,
                };
            }
            ViewMode::Chart => {
                return match key.code {
                    KeyCode::Esc | KeyCode::Char('k') => {
                        self.view_mode = ViewMode::Table;
                        true
                    }
                    KeyCode::Char('m') => {
                        self.enter_compare(state);
                        true
                    }
                    KeyCode::Char('[') => {
                        self.cycle_timeframe(-1, state);
                        true
                    }
                    KeyCode::Char(']') => {
                        self.cycle_timeframe(1, state);
                        true
                    }
                    _ => false,
                };
            }
            ViewMode::Compare => {
                // If picker is open, delegate to it
                if let Some(ref mut picker) = self.picker {
                    match picker.on_key(key) {
                        PickerResult::Selected(inst) => {
                            if !self.compare_instruments.contains(&inst)
                                && self.compare_instruments.len() < 4
                            {
                                self.fetch_candles(&inst, state);
                                self.compare_instruments.push(inst);
                            }
                            self.picker = None;
                        }
                        PickerResult::Cancelled => {
                            self.picker = None;
                        }
                        PickerResult::Continue => {}
                    }
                    return true;
                }
                return match key.code {
                    KeyCode::Esc | KeyCode::Char('m') => {
                        self.view_mode = ViewMode::Table;
                        true
                    }
                    KeyCode::Char('[') => {
                        self.cycle_timeframe(-1, state);
                        true
                    }
                    KeyCode::Char(']') => {
                        self.cycle_timeframe(1, state);
                        true
                    }
                    KeyCode::Char('a') => {
                        if self.compare_instruments.len() < 4 {
                            self.picker = Some(InstrumentPicker::new(&state.instruments));
                        }
                        true
                    }
                    KeyCode::Char('d') => {
                        if !self.compare_instruments.is_empty() {
                            let idx = self
                                .compare_selected
                                .min(self.compare_instruments.len() - 1);
                            self.compare_instruments.remove(idx);
                            if self.compare_selected >= self.compare_instruments.len()
                                && self.compare_selected > 0
                            {
                                self.compare_selected -= 1;
                            }
                        }
                        true
                    }
                    KeyCode::Char(c @ '1'..='4') => {
                        let idx = (c as usize) - ('1' as usize);
                        if idx < self.compare_instruments.len() {
                            self.compare_selected = idx;
                        }
                        true
                    }
                    _ => false,
                };
            }
            ViewMode::Table => {} // fall through to table handling
        }

        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.search_query.clear();
                    self.refilter(state);
                }
                KeyCode::Enter => {
                    self.searching = false;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.selected = 0;
                    self.scroll_offset = 0;
                    self.refilter(state);
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.selected = 0;
                    self.scroll_offset = 0;
                    self.refilter(state);
                }
                _ => {}
            }
            return true;
        }

        match key.code {
            KeyCode::Char('/') | KeyCode::Char('f') => {
                self.searching = true;
                return true;
            }
            KeyCode::Enter => {
                self.enter_detail(state);
                return true;
            }
            KeyCode::Char('k') => {
                self.enter_chart(state);
                return true;
            }
            KeyCode::Char('m') => {
                self.enter_compare(state);
                return true;
            }
            KeyCode::Char('h') => {
                self.heatmap = !self.heatmap;
                return true;
            }
            KeyCode::Left => {
                self.category = if self.category == 0 {
                    self.categories.len().saturating_sub(1)
                } else {
                    self.category - 1
                };
                self.selected = 0;
                self.scroll_offset = 0;
                self.refilter(state);
                return true;
            }
            KeyCode::Right => {
                self.category = (self.category + 1) % self.categories.len().max(1);
                self.selected = 0;
                self.scroll_offset = 0;
                self.refilter(state);
                return true;
            }
            KeyCode::Char('s') => {
                self.sort_field = (self.sort_field + 1) % SortField::ALL.len();
                self.resort(state);
                return true;
            }
            KeyCode::Char('S') => {
                self.sort_asc = !self.sort_asc;
                self.resort(state);
                return true;
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    if self.selected < self.scroll_offset {
                        self.scroll_offset = self.selected;
                    }
                }
                return true;
            }
            KeyCode::Down => {
                let max = self.filtered_instruments.len().saturating_sub(1);
                if self.selected < max {
                    self.selected += 1;
                    let vis = self.visible_rows(state.terminal_size.1.saturating_sub(4));
                    if self.selected >= self.scroll_offset + vis {
                        self.scroll_offset = self.selected + 1 - vis;
                        self.fetch_sparkline_candles(state);
                    }
                }
                return true;
            }
            KeyCode::PageUp => {
                let vis = self.visible_rows(state.terminal_size.1.saturating_sub(4));
                self.selected = self.selected.saturating_sub(vis);
                self.scroll_offset = self.scroll_offset.saturating_sub(vis);
                self.fetch_sparkline_candles(state);
                return true;
            }
            KeyCode::PageDown => {
                let vis = self.visible_rows(state.terminal_size.1.saturating_sub(4));
                let max = self.filtered_instruments.len().saturating_sub(1);
                self.selected = (self.selected + vis).min(max);
                let max_offset = self.filtered_instruments.len().saturating_sub(vis);
                self.scroll_offset = (self.scroll_offset + vis).min(max_offset);
                self.fetch_sparkline_candles(state);
                return true;
            }
            _ => {}
        }
        false
    }

    fn on_data(&mut self, event: &DataEvent, state: &mut AppState) {
        // Handle REST responses for detail/chart/compare views
        if let DataEvent::RestResponse { method, data } = event {
            match method.as_str() {
                "sparkline-candles" => {
                    // Extract close prices from 1h candles for sparkline
                    if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                        let closes: Vec<f64> = arr
                            .iter()
                            .filter_map(|c| {
                                c.get("c")
                                    .and_then(|v| v.as_str())
                                    .and_then(|s| s.parse().ok())
                            })
                            .collect();
                        if closes.len() >= 2 {
                            if let Some(inst) = data.get("instrument_name").and_then(|v| v.as_str())
                            {
                                state.sparklines.insert(inst.to_string(), closes);
                            }
                        }
                    }
                    return;
                }
                "public/get-book" => {
                    if !data.is_null() {
                        self.book_data = Some(data.clone());
                        self.clamp_book_cursor();
                    }
                    return;
                }
                "public/get-trades" => {
                    if !data.is_null() {
                        if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                            self.seed_recent_trades(arr);
                        }
                    }
                    return;
                }
                "public/get-candlestick" => {
                    if !data.is_null() {
                        self.candle_data = Some(data.clone());
                        // Also parse into typed candles for streaming use
                        if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                            let parsed: Vec<Candle> =
                                arr.iter().filter_map(Candle::from_json).collect();
                            if !parsed.is_empty() {
                                // Figure out the instrument from the data
                                let inst = data
                                    .get("instrument_name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&self.detail_instrument)
                                    .to_string();
                                self.candles.insert(inst.clone(), parsed);
                                self.touch_candles(&inst);
                            }
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        // Live book updates from WebSocket — only accept for the detail instrument
        if let DataEvent::BookSnapshot(data) = event {
            if self.view_mode == ViewMode::Detail {
                // Extract instrument from the WS message to filter
                let msg_instrument = data
                    .get("result")
                    .and_then(|r| r.get("instrument_name"))
                    .and_then(|v| v.as_str())
                    .or_else(|| data.get("instrument_name").and_then(|v| v.as_str()));

                if msg_instrument == Some(self.detail_instrument.as_str()) {
                    if let Some(result) = data.get("result") {
                        self.book_data = Some(result.clone());
                    } else {
                        self.book_data = Some(data.clone());
                    }
                    self.clamp_book_cursor();
                }
            }
            return;
        }

        // Live trade updates from WebSocket — only accept for the detail instrument.
        // WS frames deliver incremental batches (typically 1–4 trades), not full
        // snapshots — prepend them so the panel acts as a rolling feed instead
        // of clearing on every frame.
        if let DataEvent::TradeSnapshot(data) = event {
            if self.view_mode == ViewMode::Detail {
                // Payload may be a bare array or {data: [...]}. Try both.
                let batch = data
                    .as_array()
                    .or_else(|| data.get("data").and_then(|d| d.as_array()));

                if let Some(arr) = batch {
                    let instrument_matches = arr
                        .first()
                        .and_then(|t| t.get("i").and_then(|v| v.as_str()))
                        .map(|i| i == self.detail_instrument)
                        .unwrap_or(false);

                    if instrument_matches {
                        self.prepend_recent_trades(arr);
                    }
                }
            }
            return;
        }

        // Handle streaming candle updates
        if let DataEvent::CandleUpdate { instrument, candle } = event {
            // Compute interval before borrowing self.candles mutably
            let interval_ms = self.timeframe_ms();
            let candle_period = candle
                .timestamp
                .checked_div(interval_ms)
                .map(|q| q * interval_ms)
                .unwrap_or(candle.timestamp);
            {
                let entry = self.candles.entry(instrument.clone()).or_default();

                if let Some(last) = entry.last_mut() {
                    let last_period = last
                        .timestamp
                        .checked_div(interval_ms)
                        .map(|q| q * interval_ms)
                        .unwrap_or(last.timestamp);
                    if last_period == candle_period {
                        // Same candle period — update OHLCV in place
                        last.update_from(candle);
                    } else if candle_period > last_period {
                        // New candle period — append
                        entry.push(candle.clone());
                        if entry.len() > 200 {
                            entry.remove(0);
                        }
                    }
                    // Ignore candles older than the last one
                } else {
                    entry.push(candle.clone());
                }
            }
            // touch_candles needs a separate &mut self borrow, so we scope
            // the entry borrow above.
            self.touch_candles(instrument);
            return;
        }

        if self.filtered_instruments.is_empty() {
            self.refilter(state);
        } else {
            self.resort(state);
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        // Sub-views
        match self.view_mode {
            ViewMode::Detail => {
                // VecDeque doesn't impl AsRef<[T]>, so collect contiguous view.
                // Buffer is capped at RECENT_TRADES_CAP (100) — cheap.
                let trades: Vec<serde_json::Value> = self.recent_trades.iter().cloned().collect();
                draw_detail(
                    frame,
                    area,
                    &self.detail_instrument,
                    state,
                    &self.book_data,
                    self.book_depth,
                    self.book_cursor,
                    &trades,
                );
                return;
            }
            ViewMode::Chart => {
                let raw = self
                    .candles
                    .get(&self.detail_instrument)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let filled = fill_candle_gaps(raw, self.timeframe_ms());
                draw_candlestick(
                    frame,
                    area,
                    &self.detail_instrument,
                    &filled,
                    &self.timeframe,
                    &state.theme.colors,
                    &format!(
                        "Esc:back  [:prev ]]:next tf  k:table  m:compare  t:trade  ({})",
                        self.timeframe
                    ),
                );
                return;
            }
            ViewMode::Compare => {
                let interval_ms = self.timeframe_ms();
                let filled: Vec<(String, Vec<Candle>)> = self
                    .compare_instruments
                    .iter()
                    .map(|inst| {
                        let raw = self.candles.get(inst).map(|v| v.as_slice()).unwrap_or(&[]);
                        (inst.clone(), fill_candle_gaps(raw, interval_ms))
                    })
                    .collect();
                let charts: Vec<(&str, &[Candle])> = filled
                    .iter()
                    .map(|(inst, candles)| (inst.as_str(), candles.as_slice()))
                    .collect();

                let [chart_area, footer_area] =
                    Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

                if charts.is_empty() {
                    frame.render_widget(
                        Paragraph::new("No instruments. Press a to add one.")
                            .style(Style::default().fg(state.theme.colors.muted)),
                        chart_area,
                    );
                } else {
                    draw_compare_charts(
                        frame,
                        chart_area,
                        &charts,
                        &self.timeframe,
                        &state.theme.colors,
                    );
                }

                // Footer
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(
                            "Esc:back  a:add  d:remove  [:prev ]]:next tf  m:table  ({})",
                            self.timeframe
                        ),
                        Style::default().fg(state.theme.colors.muted),
                    ))),
                    footer_area,
                );

                // Instrument picker overlay (on top of charts)
                if let Some(ref picker) = self.picker {
                    picker.draw(frame, area, &state.theme.colors);
                }
                return;
            }
            ViewMode::Table => {} // fall through
        }

        let [header_area, table_area, footer_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);

        // === Header: category + sort + search/page info ===
        let mut spans = vec![Span::styled(
            "Market: ",
            Style::default().fg(state.theme.colors.muted),
        )];
        for (i, cat_type) in self.categories.iter().enumerate() {
            let style = if i == self.category {
                Style::default()
                    .fg(state.theme.colors.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(state.theme.colors.muted)
            };
            let label = if i == self.category {
                format!("[{}]", category_label(cat_type))
            } else {
                format!(" {} ", category_label(cat_type))
            };
            spans.push(Span::styled(label, style));
        }
        spans.push(Span::styled(
            "  Sort: ",
            Style::default().fg(state.theme.colors.muted),
        ));
        for (i, sf) in SortField::ALL.iter().enumerate() {
            let style = if i == self.sort_field {
                Style::default()
                    .fg(state.theme.colors.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(state.theme.colors.muted)
            };
            let arrow = if i == self.sort_field {
                if self.sort_asc {
                    "\u{2191}"
                } else {
                    "\u{2193}"
                }
            } else {
                ""
            };
            let label = if i == self.sort_field {
                format!("[{}{}]", sf.label(), arrow)
            } else {
                sf.label().to_string()
            };
            spans.push(Span::styled(format!(" {}", label), style));
        }

        if self.searching {
            spans.push(Span::styled(
                format!("  /{}\u{2588}", self.search_query),
                Style::default().fg(state.theme.colors.accent),
            ));
        } else {
            let total = self.filtered_instruments.len();
            let vis = self.visible_rows_in_table(table_area.height);
            let end = (self.scroll_offset + vis).min(total);
            if total > 0 {
                spans.push(Span::styled(
                    format!("  {}-{}/{}", self.scroll_offset + 1, end, total),
                    Style::default().fg(state.theme.colors.muted),
                ));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), header_area);

        // === Table ===
        let is_perp =
            self.categories.get(self.category).map(|t| t.as_str()) == Some("PERPETUAL_SWAP");
        let header = Row::new({
            let mut h = vec![
                Cell::from(" #"),
                Cell::from("Instrument"),
                Cell::from("Spark"),
                Cell::from("Price"),
                Cell::from("24h"),
                Cell::from("High"),
                Cell::from("Low"),
                Cell::from(match state.volume_unit {
                    crate::state::VolumeUnit::Usd => "Volume (USD)",
                    crate::state::VolumeUnit::Notional => "Volume",
                }),
            ];
            if is_perp {
                h.push(Cell::from("Fund"));
            }
            h
        })
        .style(
            Style::default()
                .fg(state.theme.colors.header)
                .add_modifier(Modifier::BOLD),
        );

        let widths_base = vec![
            Constraint::Length(4),
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(12),
        ];
        let widths: Vec<Constraint> = if is_perp {
            let mut w = widths_base;
            w.push(Constraint::Length(10));
            w
        } else {
            widths_base
        };

        let vis = self.visible_rows_in_table(table_area.height);
        let end = (self.scroll_offset + vis).min(self.filtered_instruments.len());
        let visible_slice = &self.filtered_instruments[self.scroll_offset..end];

        let rows: Vec<Row> = visible_slice
            .iter()
            .enumerate()
            .map(|(vi, inst)| {
                let abs_idx = self.scroll_offset + vi;
                let rank = abs_idx + 1;
                let is_selected = abs_idx == self.selected;
                let ticker = state.tickers.get(inst);

                let row_style = if is_selected {
                    Style::default()
                        .fg(state.theme.colors.selected_fg)
                        .bg(state.theme.colors.selected_bg)
                } else if self.heatmap {
                    // Heatmap: background color intensity based on 24h change
                    let change = ticker.map(|t| t.change_pct).unwrap_or(0.0);
                    let bg = heatmap_color(change);
                    Style::default().fg(Color::White).bg(bg)
                } else {
                    Style::default().fg(state.theme.colors.fg)
                };

                // Sparkline: 24h price profile from REST candle close prices
                let spark = render_sparkline_24h(
                    state
                        .sparklines
                        .get(inst)
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]),
                    10,
                );
                let spark_color = if ticker.map(|t| t.change_pct >= 0.0).unwrap_or(true) {
                    state.theme.colors.positive
                } else {
                    state.theme.colors.negative
                };

                if let Some(t) = ticker {
                    let change_color = if t.change_pct >= 0.0 {
                        state.theme.colors.positive
                    } else {
                        state.theme.colors.negative
                    };

                    // Price flash: briefly color the price cell on price changes
                    let price_style = if let Some(&(up, when)) = state.price_flashes.get(inst) {
                        let elapsed = when.elapsed().as_millis();
                        if elapsed < 600 {
                            // Fade: full intensity for first 300ms, then fade
                            let intensity = if elapsed < 300 {
                                60
                            } else {
                                60 - ((elapsed - 300) * 60 / 300) as u8
                            };
                            let bg = if up {
                                Color::Rgb(0, intensity, 0)
                            } else {
                                Color::Rgb(intensity, 0, 0)
                            };
                            Style::default().fg(Color::White).bg(bg)
                        } else {
                            Style::default().fg(state.theme.colors.fg)
                        }
                    } else {
                        Style::default().fg(state.theme.colors.fg)
                    };

                    let mut cells = vec![
                        Cell::from(format!("{:>3}", rank)),
                        Cell::from(inst.as_str()),
                        Cell::from(spark).style(Style::default().fg(spark_color)),
                        Cell::from(format_price(t.ask)).style(price_style),
                        Cell::from(format!("{:+.2}%", t.change_pct * 100.0))
                            .style(Style::default().fg(change_color)),
                        Cell::from(format_price(t.high))
                            .style(Style::default().fg(state.theme.colors.muted)),
                        Cell::from(format_price(t.low))
                            .style(Style::default().fg(state.theme.colors.muted)),
                        Cell::from(match state.volume_unit {
                            crate::state::VolumeUnit::Usd => format_compact(t.volume_usd),
                            crate::state::VolumeUnit::Notional => format_compact(t.volume),
                        })
                        .style(Style::default().fg(state.theme.colors.volume)),
                    ];
                    if is_perp {
                        let fr_color = if t.funding_rate >= 0.0 {
                            state.theme.colors.positive
                        } else {
                            state.theme.colors.negative
                        };
                        cells.push(
                            Cell::from(if t.funding_rate != 0.0 {
                                format!("{:+.4}%", t.funding_rate * 100.0)
                            } else {
                                "\u{2014}".into()
                            })
                            .style(Style::default().fg(fr_color)),
                        );
                    }
                    Row::new(cells).style(row_style)
                } else {
                    let mut cells = vec![
                        Cell::from(format!("{:>3}", rank)),
                        Cell::from(inst.as_str()),
                        Cell::from(""),
                        Cell::from("       \u{2026}"),
                        Cell::from(""),
                        Cell::from(""),
                        Cell::from(""),
                        Cell::from(""),
                    ];
                    if is_perp {
                        cells.push(Cell::from(""));
                    }
                    Row::new(cells).style(row_style)
                }
            })
            .collect();

        let table = Table::new(rows, widths).header(header).column_spacing(1);
        frame.render_widget(table, table_area);

        // === Footer ===
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "\u{2190}\u{2192}:category  s:sort  S:reverse  /:search  h:heatmap  Enter:detail  t:trade",
                Style::default().fg(state.theme.colors.muted),
            ))),
            footer_area,
        );
    }

    fn subscriptions(&self, _state: &AppState) -> Vec<String> {
        let mut subs: Vec<String> = self
            .filtered_instruments
            .iter()
            .take(30)
            .map(|i| format!("ticker.{}", i))
            .collect();

        // Add view-specific streaming channels
        match self.view_mode {
            ViewMode::Detail => {
                // Live order book updates
                // `book.{instrument}` (no depth suffix) is being deprecated
                // by the exchange — the WS docs mandate
                // `book.{instrument}.{depth}` with depth ∈ {10, 50, 150}.
                subs.push(self.book_channel());
                subs.push(format!("trade.{}", self.detail_instrument));
            }
            ViewMode::Chart => {
                subs.push(format!(
                    "candlestick.{}.{}",
                    self.timeframe, self.detail_instrument
                ));
            }
            ViewMode::Compare => {
                for inst in &self.compare_instruments {
                    subs.push(format!("candlestick.{}.{}", self.timeframe, inst));
                }
            }
            _ => {}
        }
        subs
    }

    fn selected_instrument(&self) -> Option<&str> {
        self.filtered_instruments
            .get(self.selected)
            .map(|s| s.as_str())
    }

    fn on_click(&mut self, row: u16, _col: u16, state: &mut AppState) -> bool {
        if self.view_mode != ViewMode::Table {
            return false;
        }
        // Content area layout: row 0 = category header, row 1 = table header, row 2+ = data
        if row >= 2 {
            let visual_row = (row - 2) as usize;
            let vis = self.visible_rows(state.terminal_size.1.saturating_sub(4));
            if visual_row >= vis {
                return false;
            }
            let table_row = visual_row + self.scroll_offset;
            if table_row < self.filtered_instruments.len() {
                self.selected = table_row;
                return true;
            }
        }
        false
    }

    fn on_double_click(&mut self, row: u16, _col: u16, state: &mut AppState) -> bool {
        if self.view_mode != ViewMode::Table {
            return false;
        }
        if row >= 2 {
            let visual_row = (row - 2) as usize;
            let vis = self.visible_rows(state.terminal_size.1.saturating_sub(4));
            if visual_row >= vis {
                return false;
            }
            let table_row = visual_row + self.scroll_offset;
            if table_row < self.filtered_instruments.len() {
                self.selected = table_row;
                self.enter_detail(state);
                return true;
            }
        }
        false
    }

    fn export_csv(&self, state: &AppState) -> Option<String> {
        let mut csv = String::from("Instrument,Price,24h%,High,Low,Volume\n");
        for inst in &self.filtered_instruments {
            if let Some(t) = state.tickers.get(inst) {
                let vol = match state.volume_unit {
                    crate::state::VolumeUnit::Usd => t.volume_usd,
                    crate::state::VolumeUnit::Notional => t.volume,
                };
                csv.push_str(&format!(
                    "{},{:.8},{:+.2},{:.8},{:.8},{:.2}\n",
                    inst, t.ask, t.change_pct, t.high, t.low, vol
                ));
            }
        }
        Some(csv)
    }

    fn get_candles(&self, instrument: &str) -> &[crate::widgets::candlestick::Candle] {
        self.candles
            .get(instrument)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn navigate_to_instrument(&mut self, instrument: &str, state: &AppState) {
        self.navigated_from = state.pending_return_tab;
        self.enter_detail_for(instrument, state);
    }
}

// format helpers moved to crate::format
/// Sparkline using Braille dot patterns for smooth line rendering.
/// Each character cell is 2 dots wide × 4 dots high, giving much higher
/// resolution than block characters.
///
/// Braille encoding: U+2800 base, dots are:
///   col0: row0=0x01, row1=0x02, row2=0x04, row3=0x40
///   col1: row0=0x08, row1=0x10, row2=0x20, row3=0x80
fn render_sparkline_24h(prices: &[f64], width: usize) -> String {
    if prices.len() < 2 {
        return "\u{2800}".repeat(width); // empty braille (waiting for data)
    }

    // Each braille char covers 2 data points horizontally
    let num_points = width * 2;

    // Resample to num_points
    let data: Vec<f64> = (0..num_points)
        .map(|i| {
            let pos = i as f64 * (prices.len() - 1) as f64 / (num_points - 1).max(1) as f64;
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(prices.len() - 1);
            let frac = pos - lo as f64;
            prices[lo] * (1.0 - frac) + prices[hi] * frac
        })
        .collect();

    let min = data.iter().copied().fold(f64::MAX, f64::min);
    let max = data.iter().copied().fold(f64::MIN, f64::max);
    let range = max - min;

    // Map each point to a row (0=bottom, 3=top) in the 4-row braille grid
    let rows: Vec<usize> = data
        .iter()
        .map(|&p| {
            if range > 0.0 {
                let normalized = (p - min) / range; // 0.0 to 1.0
                (normalized * 3.0).round() as usize // 0 to 3
            } else {
                2 // mid-height
            }
        })
        .collect();

    // Build braille characters: each char covers 2 consecutive points
    // Braille dot positions (row 0 = top visually, but row 3 in braille = top):
    //   We invert: data row 3 (highest) → braille row 0 (top dot)
    let dot_bits: [[u8; 2]; 4] = [
        [0x01, 0x08], // row 0 (top)
        [0x02, 0x10], // row 1
        [0x04, 0x20], // row 2
        [0x40, 0x80], // row 3 (bottom)
    ];

    let mut result = String::with_capacity(width);
    for ch_idx in 0..width {
        let left = ch_idx * 2;
        let right = left + 1;

        let mut bits: u8 = 0;

        // Left column dot
        if left < rows.len() {
            let braille_row = 3 - rows[left].min(3); // invert: high value = top dot
            bits |= dot_bits[braille_row][0];
        }

        // Right column dot
        if right < rows.len() {
            let braille_row = 3 - rows[right].min(3);
            bits |= dot_bits[braille_row][1];
        }

        // Also fill dots between left and right if they span multiple rows
        // This creates a connected line effect
        if left < rows.len() && right < rows.len() {
            let r1 = 3 - rows[left].min(3);
            let r2 = 3 - rows[right].min(3);
            let lo_r = r1.min(r2);
            let hi_r = r1.max(r2);
            for (r_idx, dot_row) in dot_bits.iter().enumerate().take(hi_r + 1).skip(lo_r) {
                if r_idx == r1 {
                    bits |= dot_row[0];
                }
                if r_idx == r2 {
                    bits |= dot_row[1];
                }
                // Fill intermediate rows on the closer column
                if r_idx > lo_r && r_idx < hi_r {
                    bits |= dot_row[0];
                    bits |= dot_row[1];
                }
            }
        }

        result.push(char::from_u32(0x2800 + bits as u32).unwrap_or(' '));
    }
    result
}

/// Map a 24h change percentage to a background color for heatmap mode.
/// Ranges from deep red (-10%+) through dark bg (0%) to deep green (+10%+).
fn heatmap_color(change_pct: f64) -> Color {
    // change_pct is a ratio (0.05 = 5%), cap at ±10% (0.10)
    let intensity = (change_pct.abs() / 0.10).min(1.0);
    let i = (intensity * 80.0) as u8; // max 80 to keep text readable

    if change_pct >= 0.0 {
        Color::Rgb(0, 20 + i, 0) // green gradient
    } else {
        Color::Rgb(20 + i, 0, 0) // red gradient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_label() {
        assert_eq!(category_label("CCY_PAIR"), "Spot");
        assert_eq!(category_label("PERPETUAL_SWAP"), "Perpetual");
        assert_eq!(category_label("FUTURE"), "Futures");
        assert_eq!(category_label("RWA"), "RWA"); // unknown types pass through
    }

    #[test]
    fn is_editing_tracks_search_and_picker() {
        let mut tab = MarketTab::new();
        assert!(!tab.is_editing(), "fresh tab must not be editing");

        tab.searching = true;
        assert!(tab.is_editing(), "search bar open must be editing");
        tab.searching = false;

        // Picker open (compare view) must also suppress global hotkeys —
        // otherwise characters like 'p' toggle paper mode instead of
        // filtering the picker query.
        tab.picker = Some(InstrumentPicker::new(&["BTC_USDT".into()]));
        assert!(tab.is_editing(), "picker open must be editing");
    }

    #[test]
    fn test_category_sort_key() {
        let mut types = vec!["FUTURE", "RWA", "CCY_PAIR", "PERPETUAL_SWAP"];
        types.sort_by(|a, b| category_sort_key(a).cmp(&category_sort_key(b)));
        assert_eq!(types, vec!["CCY_PAIR", "PERPETUAL_SWAP", "FUTURE", "RWA"]);
    }

    #[test]
    fn test_format_price() {
        assert_eq!(format_price(67234.5), "67,234.50");
        assert_eq!(format_price(3.456), "3.4560");
        assert_eq!(format_price(0.00208), "0.00208000");
        assert_eq!(format_price(0.0), "\u{2014}");
    }

    #[test]
    fn test_format_compact() {
        assert_eq!(format_compact(1_200_000_000.0), "1.2B");
        assert_eq!(format_compact(892_300_000.0), "892.3M");
        assert_eq!(format_compact(445_100.0), "445.1K");
        assert_eq!(format_compact(0.0), "\u{2014}");
    }

    // ---- Recent trades rolling buffer (Issue #21) ----

    fn trade(price: &str, qty: &str, side: &str) -> serde_json::Value {
        serde_json::json!({"p": price, "q": qty, "s": side, "i": "BTC_USDT"})
    }

    /// REST seed must replace the buffer in insertion order and cap at
    /// RECENT_TRADES_CAP. The Exchange `public/get-trades` endpoint returns
    /// newest-first, so the first element of the input must end up at the
    /// front of the deque.
    #[test]
    fn seed_recent_trades_replaces_and_caps() {
        let mut tab = MarketTab::new();
        tab.recent_trades.push_back(trade("stale", "1", "BUY"));
        tab.recent_trades.push_back(trade("stale", "2", "SELL"));

        let seed: Vec<serde_json::Value> = (0..150)
            .map(|i| trade(&format!("{}", i), "1", "BUY"))
            .collect();
        tab.seed_recent_trades(&seed);

        assert_eq!(
            tab.recent_trades.len(),
            RECENT_TRADES_CAP,
            "seed must truncate to cap"
        );
        assert_eq!(
            tab.recent_trades.front().and_then(|t| t.get("p")),
            Some(&serde_json::Value::String("0".into())),
            "newest (first) trade in input must land at front"
        );
        assert!(
            !tab.recent_trades
                .iter()
                .any(|t| t.get("p") == Some(&serde_json::Value::String("stale".into()))),
            "prior contents must be cleared"
        );
    }

    /// WS streaming batches are incremental — they must prepend, not replace.
    /// This is the core regression test for Issue #21: before the fix,
    /// each WS frame overwrote the buffer so only the latest frame's 1–4
    /// trades were visible.
    #[test]
    fn prepend_recent_trades_builds_rolling_feed() {
        let mut tab = MarketTab::new();
        tab.seed_recent_trades(&[
            trade("100", "1", "BUY"),
            trade("99", "1", "SELL"),
            trade("98", "1", "BUY"),
        ]);
        assert_eq!(tab.recent_trades.len(), 3);

        // First WS batch: 2 new trades.
        tab.prepend_recent_trades(&[trade("102", "1", "BUY"), trade("101", "1", "SELL")]);
        // Previous 3 trades must still be present (this is the bug).
        assert_eq!(
            tab.recent_trades.len(),
            5,
            "WS frames must accumulate — previous trades must not be discarded"
        );

        // Newest trade of the incoming batch must be at the front.
        assert_eq!(
            tab.recent_trades[0].get("p"),
            Some(&serde_json::Value::String("102".into())),
            "newest trade in batch must be at the front"
        );
        assert_eq!(
            tab.recent_trades[1].get("p"),
            Some(&serde_json::Value::String("101".into())),
            "batch order preserved: index 1 = second-newest of batch"
        );
        // Pre-existing trades slide down.
        assert_eq!(
            tab.recent_trades[2].get("p"),
            Some(&serde_json::Value::String("100".into()))
        );
    }

    /// Buffer must never exceed RECENT_TRADES_CAP even under sustained WS
    /// traffic — oldest trades drop off the back.
    #[test]
    fn prepend_recent_trades_evicts_oldest_at_cap() {
        let mut tab = MarketTab::new();
        let initial: Vec<serde_json::Value> = (0..RECENT_TRADES_CAP)
            .map(|i| trade(&format!("seed-{}", i), "1", "BUY"))
            .collect();
        tab.seed_recent_trades(&initial);
        assert_eq!(tab.recent_trades.len(), RECENT_TRADES_CAP);

        // Push in a batch of 3 new trades — the 3 oldest must evict.
        tab.prepend_recent_trades(&[
            trade("new-c", "1", "BUY"),
            trade("new-b", "1", "BUY"),
            trade("new-a", "1", "BUY"),
        ]);
        assert_eq!(
            tab.recent_trades.len(),
            RECENT_TRADES_CAP,
            "cap must not be exceeded"
        );
        assert_eq!(
            tab.recent_trades.front().and_then(|t| t.get("p")),
            Some(&serde_json::Value::String("new-c".into())),
            "newest stays at front"
        );
        // Oldest 3 seeds gone — the back of the buffer now holds seed-0..seed-96.
        assert_eq!(
            tab.recent_trades.back().and_then(|t| t.get("p")),
            Some(&serde_json::Value::String(format!(
                "seed-{}",
                RECENT_TRADES_CAP - 1 - 3
            ))),
            "oldest 3 seeds must be evicted from the back"
        );
    }

    // ---- Candles LRU eviction (Issue #25) ----

    /// Seed `candles` + access-order with one entry per instrument so
    /// tests exercise the touch/evict logic without needing REST I/O.
    /// We don't care about the Vec<Candle> contents here — only the
    /// map bookkeeping.
    fn insert_candle(tab: &mut MarketTab, inst: &str) {
        tab.candles.insert(inst.to_string(), vec![]);
        tab.touch_candles(inst);
    }

    /// Touching the same instrument multiple times must not grow the
    /// access-order deque — it promotes, not appends.
    #[test]
    fn touch_candles_promotes_existing_entry() {
        let mut tab = MarketTab::new();
        insert_candle(&mut tab, "BTC_USDT");
        insert_candle(&mut tab, "ETH_USDT");

        // Re-touch BTC — ETH should now be oldest, BTC newest.
        tab.touch_candles("BTC_USDT");

        assert_eq!(tab.candle_access_order.len(), 2, "no duplicate entry");
        assert_eq!(
            tab.candle_access_order.front().map(|s| s.as_str()),
            Some("ETH_USDT"),
            "older entry must be at the front"
        );
        assert_eq!(
            tab.candle_access_order.back().map(|s| s.as_str()),
            Some("BTC_USDT"),
            "re-touched entry must move to the back (MRU)"
        );
    }

    /// Issue #25 regression: when the cap is exceeded, the LRU entry
    /// must be evicted from BOTH the access-order deque AND the
    /// candles HashMap. Without the evict from `candles`, the
    /// HashMap would grow unbounded.
    #[test]
    fn touch_candles_evicts_lru_from_both_maps() {
        let mut tab = MarketTab::new();
        // Fill to cap.
        for i in 0..MAX_CANDLE_INSTRUMENTS {
            insert_candle(&mut tab, &format!("INST_{}", i));
        }
        assert_eq!(tab.candles.len(), MAX_CANDLE_INSTRUMENTS);
        assert_eq!(tab.candle_access_order.len(), MAX_CANDLE_INSTRUMENTS);

        // Adding one more must evict INST_0 from both structures.
        insert_candle(&mut tab, "NEW_INST");

        assert_eq!(
            tab.candles.len(),
            MAX_CANDLE_INSTRUMENTS,
            "candles HashMap must not exceed cap"
        );
        assert_eq!(
            tab.candle_access_order.len(),
            MAX_CANDLE_INSTRUMENTS,
            "access order must not exceed cap"
        );
        assert!(
            !tab.candles.contains_key("INST_0"),
            "oldest entry must be evicted from the HashMap, not just the order deque"
        );
        assert!(tab.candles.contains_key("NEW_INST"));
        assert!(tab
            .candles
            .contains_key(&format!("INST_{}", MAX_CANDLE_INSTRUMENTS - 1)));
    }

    /// Sustained traffic: the HashMap size must stay bounded even
    /// after many more insertions than the cap.
    #[test]
    fn touch_candles_holds_bound_under_sustained_traffic() {
        let mut tab = MarketTab::new();
        for i in 0..(MAX_CANDLE_INSTRUMENTS * 5) {
            insert_candle(&mut tab, &format!("INST_{}", i));
            assert!(
                tab.candles.len() <= MAX_CANDLE_INSTRUMENTS,
                "candles map exceeded cap after {} insertions: {}",
                i + 1,
                tab.candles.len()
            );
        }
        assert_eq!(tab.candles.len(), MAX_CANDLE_INSTRUMENTS);
    }

    // ---- Order-book depth preset cycle (Issue #26) ----

    /// The `D` keybinding cycles through WS-valid depths in order and wraps
    /// back to the first preset. 20 is intentionally excluded — it is a valid
    /// REST value but the WS `book.{instrument}.{depth}` channel only accepts
    /// 10/50/150, so letting the user land on 20 would leave the live stream
    /// silent.
    #[test]
    fn next_book_depth_cycles_through_presets_in_order() {
        assert_eq!(MarketTab::next_book_depth(10), 50);
        assert_eq!(MarketTab::next_book_depth(50), 150);
        assert_eq!(MarketTab::next_book_depth(150), 10, "wraps back to first");
    }

    /// Defense in depth: if `book_depth` somehow falls outside the preset
    /// list (manual state mutation, future refactor, corrupted restore),
    /// the next cycle must snap back to a valid preset instead of hanging
    /// on an un-resubscribable value.
    #[test]
    fn next_book_depth_recovers_from_out_of_band_value() {
        assert_eq!(
            MarketTab::next_book_depth(20),
            BOOK_DEPTH_PRESETS[0],
            "unknown depth must fall through to first preset"
        );
    }

    /// The WS channel must include the depth suffix. Without it the
    /// subscription silently targets the deprecated `book.{instrument}` path
    /// that the exchange is retiring — live updates would stop arriving.
    /// Exercises the real production helper, not `format!`, so a refactor
    /// that changes the channel shape would break this test.
    #[test]
    fn book_channel_includes_depth_suffix() {
        let mut tab = MarketTab::new();
        tab.detail_instrument = "BTC_USDT".into();

        tab.book_depth = 50;
        assert_eq!(tab.book_channel(), "book.BTC_USDT.50");

        tab.book_depth = 150;
        assert_eq!(tab.book_channel(), "book.BTC_USDT.150");

        // Regression guard: the deprecated no-suffix form must never appear.
        assert_ne!(
            tab.book_channel(),
            format!("book.{}", tab.detail_instrument)
        );
    }

    // ---- Book level cursor (#26 Piece 2) ----

    /// ↓ within asks walks toward the mid (smaller index). The deepest ask
    /// is `Ask(len-1)`; pressing ↓ from there moves to `Ask(len-2)` and
    /// eventually `Ask(0)`, the best ask.
    #[test]
    fn cursor_down_on_asks_walks_toward_mid() {
        let c = BookCursor::Ask(3);
        assert_eq!(cursor_down(c, 5, 5), BookCursor::Ask(2));
        assert_eq!(cursor_down(BookCursor::Ask(1), 5, 5), BookCursor::Ask(0));
    }

    /// ↓ from the best ask crosses the mid and lands on the best bid.
    /// This is the key UX guarantee: arrows move *visually* down through
    /// the panel, and the panel's vertical layout puts bids below asks.
    #[test]
    fn cursor_down_from_best_ask_crosses_to_best_bid() {
        assert_eq!(cursor_down(BookCursor::Ask(0), 5, 5), BookCursor::Bid(0));
    }

    /// ↓ through bids moves deeper (bigger index) and clamps at the
    /// deepest visible bid instead of wrapping — the user should never
    /// see the cursor jump from the bottom back up to the top.
    #[test]
    fn cursor_down_on_bids_clamps_at_deepest() {
        assert_eq!(cursor_down(BookCursor::Bid(4), 5, 5), BookCursor::Bid(4));
    }

    /// Mirror of the ask-crossing test: ↑ from best bid lands on best ask.
    #[test]
    fn cursor_up_from_best_bid_crosses_to_best_ask() {
        assert_eq!(cursor_up(BookCursor::Bid(0), 5, 5), BookCursor::Ask(0));
    }

    /// ↑ on asks moves away from mid (deeper). At the deepest visible ask
    /// the cursor clamps — no wrap.
    #[test]
    fn cursor_up_on_asks_clamps_at_deepest() {
        assert_eq!(cursor_up(BookCursor::Ask(4), 5, 5), BookCursor::Ask(4));
    }

    /// Clamp a now-out-of-bounds cursor to the new last valid index on
    /// the same side. Simulates depth-shrink (150 → 50 → 10) where the
    /// cursor was sitting on a level that no longer exists.
    #[test]
    fn clamp_cursor_preserves_side_and_shrinks_index() {
        assert_eq!(
            clamp_cursor(BookCursor::Ask(80), 10, 10),
            Some(BookCursor::Ask(9))
        );
        assert_eq!(
            clamp_cursor(BookCursor::Bid(80), 10, 10),
            Some(BookCursor::Bid(9))
        );
    }

    /// When the cursor's side has zero levels (extreme edge — one-sided
    /// book), fall back to the opposite side's best level rather than
    /// returning `None` — the user was inspecting *something*, keep them
    /// inside the book.
    #[test]
    fn clamp_cursor_falls_through_to_opposite_side_when_empty() {
        assert_eq!(
            clamp_cursor(BookCursor::Ask(3), 0, 5),
            Some(BookCursor::Bid(0))
        );
        assert_eq!(
            clamp_cursor(BookCursor::Bid(3), 5, 0),
            Some(BookCursor::Ask(0))
        );
    }

    /// Book with no levels at all → cursor must be cleared, not clamped.
    #[test]
    fn clamp_cursor_returns_none_on_empty_book() {
        assert_eq!(clamp_cursor(BookCursor::Ask(0), 0, 0), None);
    }

    /// Cumulative stats must accumulate per-level (Σ price × qty), not
    /// shortcut as `cum_qty * last_price`. Construct a book where the
    /// difference matters: 1 @ 100 + 1 @ 200 → cum_notional = 300, not 400.
    #[test]
    fn cumulative_at_sums_notional_level_by_level() {
        let levels = vec![(100.0, 1.0), (200.0, 1.0), (300.0, 1.0)];
        assert_eq!(cumulative_at(&levels, 0), (1.0, 100.0));
        assert_eq!(cumulative_at(&levels, 1), (2.0, 300.0));
        assert_eq!(cumulative_at(&levels, 2), (3.0, 600.0));
    }

    /// Basis-points distance — positive above mid, negative below, and
    /// zero when mid is zero (defensive div-by-zero guard).
    #[test]
    fn bps_from_mid_is_signed_and_guards_zero() {
        assert_eq!(bps_from_mid(101.0, 100.0), 100.0); // +1% = +100 bps
        assert_eq!(bps_from_mid(99.0, 100.0), -100.0);
        assert_eq!(bps_from_mid(100.0, 0.0), 0.0);
    }
}
