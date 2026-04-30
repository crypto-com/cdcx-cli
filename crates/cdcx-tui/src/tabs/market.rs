use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Color;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use std::collections::HashMap;

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
    trades_data: Option<serde_json::Value>,
    candle_data: Option<serde_json::Value>,
    // Streaming candle storage per instrument
    candles: HashMap<String, Vec<Candle>>,
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
            trades_data: None,
            candle_data: None,
            candles: HashMap::new(),
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
        self.trades_data = None;
        let _ = state.rest_tx.send(RestRequest {
            method: "public/get-book".into(),
            params: serde_json::json!({"instrument_name": self.detail_instrument, "depth": "20"}),
            is_private: false,
        });
        let _ = state.rest_tx.send(RestRequest {
            method: "public/get-trades".into(),
            params: serde_json::json!({"instrument_name": self.detail_instrument}),
            is_private: false,
        });
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
        // Clear existing candle data and refetch
        self.candles.clear();
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
                        self.view_mode = ViewMode::Table;
                        if let Some(origin) = self.navigated_from.take() {
                            state.pending_return_tab = Some(origin);
                        }
                        true
                    }
                    KeyCode::Char('k') => {
                        self.enter_chart(state);
                        true
                    }
                    KeyCode::Char('m') => {
                        self.enter_compare(state);
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
                    }
                    return;
                }
                "public/get-trades" => {
                    if !data.is_null() {
                        self.trades_data = data.get("data").cloned();
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
                                    .unwrap_or(&self.detail_instrument);
                                self.candles.insert(inst.to_string(), parsed);
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
                }
            }
            return;
        }

        // Live trade updates from WebSocket — only accept for the detail instrument
        if let DataEvent::TradeSnapshot(data) = event {
            if self.view_mode == ViewMode::Detail {
                // Check instrument from trade data
                let is_match = data
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|t| t.get("i").and_then(|v| v.as_str()))
                    .map(|i| i == self.detail_instrument)
                    .unwrap_or(false);

                if is_match {
                    self.trades_data = Some(data.clone());
                } else if let Some(arr) = data.get("data") {
                    let is_match = arr
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|t| t.get("i").and_then(|v| v.as_str()))
                        .map(|i| i == self.detail_instrument)
                        .unwrap_or(false);
                    if is_match {
                        self.trades_data = Some(arr.clone());
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
                draw_detail(
                    frame,
                    area,
                    &self.detail_instrument,
                    state,
                    &self.book_data,
                    &self.trades_data,
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
                subs.push(format!("book.{}", self.detail_instrument));
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
}
