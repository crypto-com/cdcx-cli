use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Row, Table, Tabs};
use ratatui::Frame;

use crate::state::AppState;
use crate::tabs::history::HistoryTab;
use crate::tabs::market::MarketTab;
use crate::tabs::orders::OrdersTab;
use crate::tabs::portfolio::PortfolioTab;
use crate::tabs::positions::PositionsTab;
use crate::tabs::watchlist::WatchlistTab;
use crate::tabs::{DataEvent, Tab, TabKind};
use crate::widgets::settings::{SettingsAction, SettingsPanel};
use crate::widgets::status_bar::draw_status_bar;
use crate::workflows::cancel_order::CancelOrderWorkflow;
use crate::workflows::close_position::ClosePositionWorkflow;
use crate::workflows::oco_order::OcoOrderWorkflow;
use crate::workflows::otoco_order::OtocoOrderWorkflow;
use crate::workflows::paper_order::PaperOrderWorkflow;
use crate::workflows::place_order::PlaceOrderWorkflow;
use crate::workflows::{Workflow, WorkflowResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Workflow,
}

pub struct App {
    pub state: AppState,
    pub active_tab: usize,
    pub mode: Mode,
    pub should_quit: bool,
    pub show_help: bool,
    pub show_spotlight: bool,
    pub split_view: bool,
    pub tick_count: u64,
    pub tick_rate_ms: u64,
    tabs: Vec<Box<dyn Tab>>,
    workflow: Option<Box<dyn Workflow>>,
    settings: Option<SettingsPanel>,
    last_click: Option<(u16, u16, std::time::Instant)>, // (row, col, time) for double-click
    /// Bloomberg-style docked research pane shown on the right side in
    /// split view. Shared across tabs so selection state persists when
    /// switching between Market / Watchlist / Positions.
    pub research: crate::widgets::research_pane::ResearchPane,
}

impl App {
    pub fn new(state: AppState, watchlist: &[String]) -> Self {
        let tabs: Vec<Box<dyn Tab>> = vec![
            Box::new(MarketTab::new()),
            Box::new(PortfolioTab::new()),
            Box::new(OrdersTab::new()),
            Box::new(HistoryTab::new()),
            Box::new(WatchlistTab::new(&state, watchlist)),
            Box::new(PositionsTab::new()),
        ];
        Self {
            state,
            active_tab: 0,
            mode: Mode::Normal,
            should_quit: false,
            show_help: false,
            show_spotlight: false,
            split_view: false,
            tick_count: 0,
            tick_rate_ms: 250,
            tabs,
            workflow: None,
            settings: None,
            last_click: None,
            research: crate::widgets::research_pane::ResearchPane::new(),
        }
    }

    /// Notify the active tab that it was just switched to, triggering a data refresh.
    pub fn activate_current_tab(&mut self) {
        self.tabs[self.active_tab].on_activate();
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        // Settings panel — intercept all keys when open
        if let Some(ref mut panel) = self.settings {
            match panel.on_key(key) {
                SettingsAction::None => return,
                SettingsAction::ThemeChanged(theme) => {
                    self.state.theme = theme;
                    return;
                }
                SettingsAction::TickerSpeedChanged(divisor) => {
                    self.state.ticker_speed_divisor = divisor;
                    return;
                }
                SettingsAction::Save {
                    theme,
                    tick_rate_ms,
                    ticker_speed_divisor,
                } => {
                    let theme_name = theme.name.clone();
                    self.state.theme = theme;
                    self.tick_rate_ms = tick_rate_ms;
                    self.state.ticker_speed_divisor = ticker_speed_divisor;
                    let speed_key = match ticker_speed_divisor {
                        4 => "slow",
                        1 => "fast",
                        _ => "medium",
                    };
                    self.settings = None;
                    match crate::widgets::settings::save_settings(
                        &theme_name,
                        tick_rate_ms,
                        speed_key,
                    ) {
                        Ok(_) => self
                            .state
                            .toast("Settings saved", crate::state::ToastStyle::Success),
                        Err(e) => self.state.toast(
                            format!("Save failed: {}", e),
                            crate::state::ToastStyle::Error,
                        ),
                    }
                    return;
                }
                SettingsAction::Close => {
                    self.settings = None;
                    return;
                }
            }
        }

        // Help overlay — dismiss on any key
        if self.show_help {
            self.show_help = false;
            return;
        }

        // Ctrl+C: quit (always works, even during workflows)
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            self.should_quit = true;
            return;
        }

        // If a workflow is active, delegate all input to it
        if self.mode == Mode::Workflow {
            if let Some(ref mut wf) = self.workflow {
                match wf.on_key(key, &mut self.state) {
                    WorkflowResult::Continue => {}
                    WorkflowResult::Done | WorkflowResult::Cancel => {
                        self.workflow = None;
                        self.mode = Mode::Normal;
                    }
                }
            }
            return;
        }

        // Dismiss spotlight on any key; let workflow keys propagate
        if self.show_spotlight {
            self.show_spotlight = false;
            match key.code {
                KeyCode::Char('t')
                | KeyCode::Char('o')
                | KeyCode::Char('O')
                | KeyCode::Char('c') => {} // fall through to normal handling
                _ => return,
            }
        }

        // When a tab has active text input (e.g. search bar), skip global single-key
        // bindings and delegate directly to the tab so keystrokes reach the input.
        let tab_editing = self
            .tabs
            .get(self.active_tab)
            .map(|t| t.is_editing())
            .unwrap_or(false);

        if tab_editing {
            // Esc still works (tab handles it to exit edit mode)
            let prev_instrument = self.get_selected_instrument();
            let consumed = self
                .tabs
                .get_mut(self.active_tab)
                .map(|tab| tab.on_key(key, &mut self.state))
                .unwrap_or(false);
            if consumed {
                // Fetch candles + update research pane on selection change
                if self.split_view {
                    let new_instrument = self.get_selected_instrument();
                    if new_instrument != prev_instrument {
                        if let Some(inst) = new_instrument {
                            let _ = self.state.rest_tx.send(crate::state::RestRequest {
                                method: "public/get-candlestick".into(),
                                params: serde_json::json!({"instrument_name": inst, "timeframe": "1h"}),
                                is_private: false,
                            });
                            self.research.set_instrument(inst, &self.state);
                        }
                    }
                }
            }
            return;
        }

        // Global: quit, help, spotlight, split
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                return;
            }
            KeyCode::Char('i') => {
                self.show_spotlight = true;
                return;
            }
            KeyCode::Char('\\') => {
                self.split_view = !self.split_view;
                if self.split_view {
                    // Seed both the chart data and the research pane off the
                    // currently-selected instrument so the right pane isn't
                    // blank on first open.
                    if let Some(inst) = self.get_selected_instrument() {
                        let _ = self.state.rest_tx.send(crate::state::RestRequest {
                            method: "public/get-candlestick".into(),
                            params: serde_json::json!({"instrument_name": inst, "timeframe": "1h"}),
                            is_private: false,
                        });
                        self.research.set_instrument(inst, &self.state);
                    }
                }
                return;
            }
            KeyCode::Char(',') => {
                self.settings = Some(SettingsPanel::new(
                    &self.state.theme.name,
                    self.tick_rate_ms,
                    self.state.ticker_speed_divisor,
                ));
                return;
            }
            // Research pane controls only work in split view. Chart mode
            // inside the Market tab also binds `[` / `]` for timeframe
            // cycling, so we check `split_view` first and let the tab claim
            // them otherwise.
            KeyCode::Char(']') if self.split_view => {
                self.research.cycle_section_forward();
                return;
            }
            KeyCode::Char('[') if self.split_view => {
                self.research.cycle_section_backward();
                return;
            }
            KeyCode::Char('N') if self.split_view => {
                self.research.cycle_news_subtab();
                return;
            }
            _ => {}
        }

        // Paper mode toggle
        if key.code == KeyCode::Char('p') {
            self.state.paper_mode = !self.state.paper_mode;
            let mode = if self.state.paper_mode {
                "PAPER"
            } else {
                "LIVE"
            };
            self.state.toast(
                format!("Switched to {} mode", mode),
                crate::state::ToastStyle::Info,
            );
            self.activate_current_tab();
            return;
        }

        // Volume unit toggle (v)
        if key.code == KeyCode::Char('v') {
            self.state.volume_unit = match self.state.volume_unit {
                crate::state::VolumeUnit::Usd => crate::state::VolumeUnit::Notional,
                crate::state::VolumeUnit::Notional => crate::state::VolumeUnit::Usd,
            };
            let label = match self.state.volume_unit {
                crate::state::VolumeUnit::Usd => "USD",
                crate::state::VolumeUnit::Notional => "Notional",
            };
            self.state.toast(
                format!("Volume unit: {}", label),
                crate::state::ToastStyle::Info,
            );
            self.activate_current_tab();
            return;
        }

        // Tab switching (1-6, Tab, BackTab)
        match key.code {
            KeyCode::Char(c @ '1'..='6') => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.tabs.len() {
                    self.active_tab = idx;
                }
                return;
            }
            KeyCode::Tab => {
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                return;
            }
            KeyCode::BackTab => {
                self.active_tab = if self.active_tab == 0 {
                    self.tabs.len() - 1
                } else {
                    self.active_tab - 1
                };
                return;
            }
            _ => {}
        }

        // Quick alert on selected instrument (! key)
        // Sets alert above current price (simple — a full alert UI would be better)
        if key.code == KeyCode::Char('!') {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                if let Some(inst) = tab.selected_instrument() {
                    if let Some(ticker) = self.state.tickers.get(inst) {
                        let price = ticker.ask;
                        // Toggle: if no alert exists, add above +1%; if one exists, remove it
                        let existing = self
                            .state
                            .alerts
                            .iter()
                            .position(|a| a.instrument == inst && !a.triggered);
                        if let Some(idx) = existing {
                            self.state.alerts.remove(idx);
                            self.state.toast(
                                format!("Alert removed: {}", inst),
                                crate::state::ToastStyle::Info,
                            );
                        } else {
                            let target = price * 1.01; // 1% above current
                            self.state.alerts.push(crate::state::PriceAlert {
                                instrument: inst.to_string(),
                                target_price: target,
                                direction: crate::state::AlertDirection::Above,
                                triggered: false,
                            });
                            self.state.toast(
                                format!("Alert set: {} > {:.2}", inst, target),
                                crate::state::ToastStyle::Success,
                            );
                        }
                    }
                }
            }
            return;
        }

        // Export to clipboard (y = yank)
        if key.code == KeyCode::Char('y') {
            if let Some(tab) = self.tabs.get(self.active_tab) {
                if let Some(csv) = tab.export_csv(&self.state) {
                    if copy_to_clipboard(&csv) {
                        self.state
                            .toast("Copied to clipboard", crate::state::ToastStyle::Success);
                    } else {
                        self.state
                            .toast("Clipboard copy failed", crate::state::ToastStyle::Error);
                    }
                }
            }
            return;
        }

        // Delegate to active tab FIRST — if the tab consumes the key
        // (e.g. typing in a search/input field), don't trigger workflows
        let prev_instrument = self.get_selected_instrument();
        let consumed = self
            .tabs
            .get_mut(self.active_tab)
            .map(|tab| tab.on_key(key, &mut self.state))
            .unwrap_or(false);

        // Handle cross-tab navigation requests (e.g. watchlist Enter → market detail)
        if let Some((target_tab, instrument)) = self.state.pending_navigation.take() {
            if let Some(idx) = TabKind::ALL.iter().position(|t| *t == target_tab) {
                self.active_tab = idx;
                if let Some(tab) = self.tabs.get_mut(idx) {
                    tab.navigate_to_instrument(&instrument, &self.state);
                }
            }
            return;
        }

        // In split view, refetch candles + re-target research pane when the
        // selected instrument changes.
        if self.split_view {
            let new_instrument = self.get_selected_instrument();
            if new_instrument != prev_instrument {
                if let Some(inst) = new_instrument {
                    let _ = self.state.rest_tx.send(crate::state::RestRequest {
                        method: "public/get-candlestick".into(),
                        params: serde_json::json!({"instrument_name": inst, "timeframe": "1h"}),
                        is_private: false,
                    });
                    self.research.set_instrument(inst, &self.state);
                }
            }
        }

        if consumed {
            return;
        }

        // Workflow triggers only if the tab didn't consume the key
        let tab_kind = TabKind::ALL.get(self.active_tab).copied();
        match (tab_kind, key.code) {
            (
                Some(TabKind::Market) | Some(TabKind::Positions) | Some(TabKind::Watchlist),
                KeyCode::Char('t'),
            ) => {
                let instrument = self.get_selected_instrument().unwrap_or("BTC_USDT".into());
                if self.state.paper_mode {
                    self.workflow = Some(Box::new(PaperOrderWorkflow::new(instrument)));
                } else {
                    self.workflow =
                        Some(Box::new(PlaceOrderWorkflow::new(instrument, &self.state)));
                }
                self.mode = Mode::Workflow;
            }
            (Some(TabKind::Positions), KeyCode::Char('x')) => {
                // Close position — only on the Positions tab. Requires an open
                // position on the selected row; if the snapshot is empty we surface
                // a toast rather than spawning a half-formed modal.
                if self.state.paper_mode {
                    self.state.toast(
                        "Close-position workflow not available in paper mode",
                        crate::state::ToastStyle::Info,
                    );
                } else if let Some(instrument) = self.get_selected_instrument() {
                    match ClosePositionWorkflow::new(instrument.clone(), &self.state) {
                        Some(wf) => {
                            self.workflow = Some(Box::new(wf));
                            self.mode = Mode::Workflow;
                        }
                        None => {
                            self.state.toast(
                                format!("No open position for {}", instrument),
                                crate::state::ToastStyle::Info,
                            );
                        }
                    }
                }
            }
            (
                Some(TabKind::Market)
                | Some(TabKind::Orders)
                | Some(TabKind::Positions)
                | Some(TabKind::Watchlist),
                KeyCode::Char('c'),
            ) => {
                if self.state.paper_mode {
                    // Cancel first open paper order for selected instrument
                    let inst = self.get_selected_instrument().unwrap_or("BTC_USDT".into());
                    let cancel_result = if let Some(ref mut engine) = self.state.paper_engine {
                        if let Some(order) = engine
                            .state
                            .open_orders
                            .iter()
                            .find(|o| o.instrument_name == inst)
                        {
                            let oid = order.order_id;
                            engine.cancel_order(oid).ok().map(|_| oid)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(oid) = cancel_result {
                        self.state.toast(
                            format!("Paper order #{} cancelled", oid),
                            crate::state::ToastStyle::Success,
                        );
                    } else {
                        self.state.toast(
                            "No open paper orders for this instrument",
                            crate::state::ToastStyle::Info,
                        );
                    }
                } else {
                    // No BTC_USDT fallback — cancelling the wrong instrument is worse
                    // than refusing. If the tab has no selection (empty list, or a tab
                    // without instrument semantics), tell the user rather than guessing.
                    match self.get_selected_instrument() {
                        Some(instrument) => {
                            self.workflow = Some(Box::new(CancelOrderWorkflow::new(instrument)));
                            self.mode = Mode::Workflow;
                        }
                        None => {
                            self.state.toast(
                                "No instrument selected — pick a row first",
                                crate::state::ToastStyle::Info,
                            );
                        }
                    }
                }
            }
            (
                Some(TabKind::Market) | Some(TabKind::Positions) | Some(TabKind::Watchlist),
                KeyCode::Char('o'),
            ) => {
                if self.state.paper_mode {
                    self.state.toast(
                        "OCO orders not available in paper mode",
                        crate::state::ToastStyle::Error,
                    );
                } else {
                    let instrument = self.get_selected_instrument().unwrap_or("BTC_USDT".into());
                    self.workflow = Some(Box::new(OcoOrderWorkflow::new(instrument)));
                    self.mode = Mode::Workflow;
                }
            }
            (
                Some(TabKind::Market) | Some(TabKind::Positions) | Some(TabKind::Watchlist),
                KeyCode::Char('O'),
            ) => {
                if self.state.paper_mode {
                    self.state.toast(
                        "OTOCO orders not available in paper mode",
                        crate::state::ToastStyle::Error,
                    );
                } else {
                    let instrument = self.get_selected_instrument().unwrap_or("BTC_USDT".into());
                    self.workflow = Some(Box::new(OtocoOrderWorkflow::new(instrument)));
                    self.mode = Mode::Workflow;
                }
            }
            _ => {}
        }
    }

    /// Recalculate portfolio total from a balance records array (WS or REST shape).
    /// Mirrors the logic used by `private/user-balance` REST handling so WS pushes keep
    /// the status-bar portfolio value in sync without waiting for the 30s REST tick.
    fn apply_balance_records(&mut self, items: &[serde_json::Value]) {
        if self.state.paper_mode {
            return;
        }
        let mut total = 0.0;
        for item in items {
            let currency = item
                .get("instrument_name")
                .or_else(|| item.get("currency"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let amount: f64 = item
                .get("total_cash_balance")
                .or_else(|| item.get("quantity"))
                .or_else(|| item.get("balance"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            if amount <= 0.0 {
                continue;
            }
            let value: f64 = item
                .get("market_value")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| {
                    if matches!(currency, "USDT" | "USD" | "USDC" | "DAI" | "TUSD" | "BUSD") {
                        amount
                    } else {
                        let pair = format!("{}_USDT", currency);
                        self.state
                            .tickers
                            .get(&pair)
                            .map(|t| amount * t.ask)
                            .unwrap_or(0.0)
                    }
                });
            total += value;
        }
        if total > 0.0 {
            if self.state.session_start_value.is_none() {
                self.state.session_start_value = Some(total);
            }
            self.state.current_portfolio_value = total;
        }
    }

    pub fn on_data(&mut self, event: DataEvent) {
        // Ticker updates go to shared state (sparklines come from REST candles only)
        if let DataEvent::TickerUpdate(ref ticker) = event {
            // Detect price change for flash animation
            if let Some(prev) = self.state.tickers.get(&ticker.instrument) {
                if (ticker.ask - prev.ask).abs() > f64::EPSILON {
                    let up = ticker.ask > prev.ask;
                    self.state
                        .price_flashes
                        .insert(ticker.instrument.clone(), (up, std::time::Instant::now()));
                }
            }
            self.state
                .tickers
                .insert(ticker.instrument.clone(), ticker.clone());
        }

        // Keep isolated_positions + portfolio value fresh from WS user channels.
        if let DataEvent::PositionsSnapshot(ref positions) = event {
            self.state.update_positions(positions);
        }
        if let DataEvent::BalanceSnapshot(ref balances) = event {
            self.apply_balance_records(balances);
        }

        // Track live portfolio value for status bar from REST responses too.
        if let DataEvent::RestResponse {
            ref method,
            ref data,
        } = event
        {
            if method == "private/user-balance" {
                if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                    self.apply_balance_records(arr);
                }
            }
            if method == "private/get-positions" {
                if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                    self.state.update_positions(arr);
                }
            }
        }

        // REST responses for workflows (create-order, cancel-all-orders).
        // The API client returns Ok(...) with embedded code+message for business-logic
        // rejections (api_client.rs parse_response), so we MUST inspect the body —
        // blindly closing the modal would silently swallow errors like
        // INSTRUMENT_MUST_USE_ISOLATED_MARGIN.
        if let DataEvent::RestResponse {
            ref method,
            ref data,
        } = event
        {
            if self.mode == Mode::Workflow {
                if method == "private/create-order" {
                    if let Some(ref mut wf) = self.workflow {
                        match wf.on_response(method, data, &mut self.state) {
                            WorkflowResult::Done | WorkflowResult::Cancel => {
                                self.workflow = None;
                                self.mode = Mode::Normal;
                            }
                            WorkflowResult::Continue => {
                                // Workflow keeps the modal open (e.g. to show rejection).
                            }
                        }
                    }
                    return;
                }
                if method == "private/cancel-all-orders" {
                    let code = data
                        .get("data")
                        .and_then(|d| d.get("code"))
                        .and_then(|v| v.as_i64())
                        .or_else(|| data.get("code").and_then(|v| v.as_i64()))
                        .unwrap_or(0);
                    self.workflow = None;
                    self.mode = Mode::Normal;
                    if code == 0 {
                        self.state
                            .toast("Orders cancelled", crate::state::ToastStyle::Success);
                    } else {
                        let message = data
                            .get("data")
                            .and_then(|d| d.get("message"))
                            .or_else(|| data.get("message"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Cancel rejected")
                            .to_string();
                        self.state.toast(
                            format!("[{}] {}", code, message),
                            crate::state::ToastStyle::Error,
                        );
                    }
                    return;
                }
            }
        }

        // User-channel events (positions, orders, balance) must reach their
        // owning tabs even when a different tab is active — beta testers hit
        // "refresh required" because we previously only delivered to the
        // active tab. Route by TabKind so the tables stay live cross-tab.
        let broadcast_kinds: &[TabKind] = match event {
            DataEvent::OrdersUpdate(_) => &[TabKind::Orders],
            DataEvent::PositionsSnapshot(_) => &[TabKind::Positions],
            DataEvent::BalanceSnapshot(_) => &[TabKind::Portfolio],
            _ => &[],
        };
        if !broadcast_kinds.is_empty() {
            for kind in broadcast_kinds {
                let idx = TabKind::ALL.iter().position(|k| k == kind);
                if let Some(i) = idx {
                    if i != self.active_tab {
                        if let Some(tab) = self.tabs.get_mut(i) {
                            tab.on_data(&event, &mut self.state);
                        }
                    }
                }
            }
        }

        // Forward to active tab
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.on_data(&event, &mut self.state);
        }
    }

    /// Dispatch a price-api event to the research pane, regardless of which
    /// tab is currently active — background fetches can finish after the
    /// user has moved away, and the pane's stale-response guard handles that.
    pub fn on_price_api(&mut self, event: crate::state::PriceApiEvent) {
        self.research.apply_event(&event, &mut self.state);
    }

    pub fn on_tick(&mut self) {
        self.tick_count += 1;

        // Check price alerts
        let triggered = self.state.check_alerts();
        for msg in triggered {
            self.state
                .toast(format!("ALERT: {}", msg), crate::state::ToastStyle::Info);
            // Terminal bell for audible alert
            print!("\x07");
        }

        // Periodic portfolio balance fetch for status bar (every ~30s at 250ms tick)
        if !self.state.paper_mode && self.state.authenticated && self.tick_count % 120 == 1 {
            let _ = self.state.rest_tx.send(crate::state::RestRequest {
                method: "private/user-balance".into(),
                params: serde_json::json!({}),
                is_private: true,
            });
        }

        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.on_data(
                &DataEvent::RestResponse {
                    method: "tick".into(),
                    data: serde_json::Value::Null,
                },
                &mut self.state,
            );
        }
    }

    pub fn on_resize(&mut self, w: u16, h: u16) {
        self.state.terminal_size = (w, h);
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        // Layout: row 0 = ticker tape, rows 1-3 = tab bar, rows 4..h-1 = content, row h-1 = status
        let content_start = 4u16;
        let content_end = self.state.terminal_size.1.saturating_sub(1);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let key = KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE);
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.on_key(key, &mut self.state);
                }
            }
            MouseEventKind::ScrollDown => {
                let key = KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE);
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.on_key(key, &mut self.state);
                }
            }
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                // Dismiss overlays on click
                if self.show_help {
                    self.show_help = false;
                    return;
                }
                if self.show_spotlight {
                    self.show_spotlight = false;
                    return;
                }

                // Click in tab bar area (rows 1-3) → switch tabs
                if mouse.row >= 1 && mouse.row <= 3 {
                    let tab_idx = (mouse.column as usize).saturating_sub(2) / 14;
                    if tab_idx < self.tabs.len() {
                        self.active_tab = tab_idx;
                    }
                }
                // Click in content area → delegate to active tab
                else if mouse.row >= content_start && mouse.row < content_end {
                    let content_row = mouse.row - content_start;
                    let now = std::time::Instant::now();

                    // Detect double-click: same row within 400ms
                    let is_double = self
                        .last_click
                        .map(|(r, _, t)| r == mouse.row && now.duration_since(t).as_millis() < 400)
                        .unwrap_or(false);

                    if is_double {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                            tab.on_double_click(content_row, mouse.column, &mut self.state);
                        }
                        self.last_click = None;
                    } else {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                            tab.on_click(content_row, mouse.column, &mut self.state);
                        }
                        self.last_click = Some((mouse.row, mouse.column, now));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn active_subscriptions(&self) -> Vec<String> {
        self.tabs
            .get(self.active_tab)
            .map(|t| t.subscriptions(&self.state))
            .unwrap_or_default()
    }

    fn get_selected_instrument(&self) -> Option<String> {
        self.tabs
            .get(self.active_tab)
            .and_then(|t| t.selected_instrument())
            .map(|s| s.to_string())
    }

    pub fn draw(&self, frame: &mut Frame) {
        let [ticker_tape_area, tab_bar_area, content_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        // Scrolling ticker tape
        crate::widgets::ticker_tape::draw_ticker_tape(
            frame,
            ticker_tape_area,
            &self.state,
            self.tick_count,
        );

        // Tab bar
        let titles: Vec<String> = TabKind::ALL
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{}:{}", i + 1, t.title()))
            .collect();
        let tabs_widget = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.state.theme.colors.border))
                    .title(" cdcx "),
            )
            .select(self.active_tab)
            .style(Style::default().fg(self.state.theme.colors.muted))
            .highlight_style(
                Style::default()
                    .fg(self.state.theme.colors.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .divider(" \u{2502} ");

        frame.render_widget(tabs_widget, tab_bar_area);

        // Active tab content — split view shows chart on right
        if self.split_view {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(content_area);

            if let Some(tab) = self.tabs.get(self.active_tab) {
                tab.draw(frame, left, &self.state);
            }

            // Right panel: Bloomberg-style research pane (Overview / Chart /
            // News sections). Candles are sourced from the Market tab — one
            // authoritative store for OHLC across the app.
            let candles: &[crate::widgets::candlestick::Candle] = self
                .research
                .selected_instrument()
                .and_then(|inst| self.tabs.first().map(|tab| tab.get_candles(inst)))
                .unwrap_or(&[]);
            self.research.draw(frame, right, candles, &self.state);
        } else if let Some(tab) = self.tabs.get(self.active_tab) {
            tab.draw(frame, content_area, &self.state);
        }

        // Workflow overlay (renders on top of tab content)
        if let Some(ref wf) = self.workflow {
            wf.draw(frame, content_area, &self.state);
        }

        // Settings overlay
        if let Some(ref panel) = self.settings {
            panel.draw(frame, content_area, &self.state.theme.colors);
        }

        // Spotlight overlay
        if self.show_spotlight {
            if let Some(inst) = self.get_selected_instrument() {
                crate::widgets::spotlight::draw_spotlight(frame, content_area, &inst, &self.state);
            }
        }

        // Help overlay
        if self.show_help {
            draw_help_overlay(frame, content_area, &self.state);
        }

        // Status bar
        draw_status_bar(frame, status_area, &self.state);
    }
}

fn draw_help_overlay(frame: &mut Frame, area: Rect, state: &AppState) {
    let width = 58u16;
    let height = 40u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let modal = Rect::new(x, y, width.min(area.width), height.min(area.height));

    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(state.theme.colors.accent))
        .title(" Keyboard Shortcuts ");
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let bindings: Vec<(&str, &str)> = vec![
        ("Global", ""),
        ("1-6", "Switch to tab"),
        ("Tab / Shift+Tab", "Next / previous tab"),
        ("q", "Quit"),
        ("?", "Toggle this help"),
        ("v", "Toggle volume unit (USD / Notional)"),
        ("r", "Refresh current tab"),
        ("y", "Copy table data to clipboard (CSV)"),
        ("!", "Toggle price alert (+1% above current)"),
        ("i", "Instrument spotlight popup"),
        ("\\", "Toggle split screen (table + chart)"),
        (",", "Settings"),
        ("Mouse scroll", "Navigate rows up/down"),
        ("", ""),
        ("Market Table", ""),
        ("\u{2190}\u{2192}", "Switch category (Spot/Perp/Futures)"),
        ("s / S", "Cycle sort field / reverse sort"),
        ("/", "Search instruments"),
        ("\u{2191}\u{2193}", "Navigate rows"),
        ("PgUp / PgDn", "Page scroll"),
        ("Enter", "Instrument detail (book + trades)"),
        ("h", "Toggle heatmap mode"),
        ("k", "Candlestick chart (streaming)"),
        ("m", "Compare charts (up to 4)"),
        ("t", "Place order (Market/Positions/Watchlist)"),
        ("o", "OCO order (stop-loss + take-profit)"),
        ("O", "OTOCO order (entry + SL + TP)"),
        ("c", "Cancel orders"),
        ("x", "Close position (Positions tab only)"),
        ("p", "Toggle LIVE / PAPER mode"),
        ("", ""),
        ("Detail View", ""),
        ("Esc", "Back to table"),
        ("k", "Switch to candlestick chart"),
        ("m", "Switch to compare view"),
        ("", ""),
        ("Chart / Compare", ""),
        ("Esc / k / m", "Back to table"),
        ("[ / ]", "Previous / next timeframe"),
        ("a", "Add instrument (compare, max 4)"),
        ("d", "Remove instrument (compare)"),
        ("1-4", "Select chart panel (compare)"),
        ("", ""),
        ("Watchlist", ""),
        ("a / d", "Add / remove instrument"),
        ("", ""),
        ("Orders / History", ""),
        ("r", "Refresh data"),
        ("n / p", "Next / previous page (history)"),
        ("", ""),
        ("Research Pane (split view)", ""),
        ("\\", "Toggle split view"),
        ("[ / ]", "Cycle section (Overview/Chart/News)"),
        ("N", "Cycle news sub-tab (News section)"),
        ("r", "Refresh research panels"),
        ("", ""),
        ("Press any key to close", ""),
    ];

    let rows: Vec<Row> = bindings
        .iter()
        .map(|(key, desc)| {
            if desc.is_empty() && !key.is_empty() {
                // Section header
                Row::new(vec![
                    Cell::from(Line::from(Span::styled(
                        *key,
                        Style::default()
                            .fg(state.theme.colors.accent)
                            .add_modifier(Modifier::BOLD),
                    ))),
                    Cell::from(""),
                ])
            } else if key.is_empty() && desc.is_empty() {
                // Blank row
                Row::new(vec![Cell::from(""), Cell::from("")])
            } else if key.is_empty() {
                // Footer text
                Row::new(vec![
                    Cell::from(Line::from(Span::styled(
                        *desc,
                        Style::default().fg(state.theme.colors.muted),
                    ))),
                    Cell::from(""),
                ])
            } else {
                Row::new(vec![
                    Cell::from(Line::from(Span::styled(
                        format!("  {}", key),
                        Style::default().fg(state.theme.colors.header),
                    ))),
                    Cell::from(Line::from(Span::styled(
                        *desc,
                        Style::default().fg(state.theme.colors.fg),
                    ))),
                ])
            }
        })
        .collect();

    let table = Table::new(rows, [Constraint::Length(20), Constraint::Fill(1)]);
    frame.render_widget(table, inner);
}

fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Try pbcopy (macOS), then xclip (Linux), then xsel (Linux)
    let commands = [
        ("pbcopy", vec![]),
        ("xclip", vec!["-selection", "clipboard"]),
        ("xsel", vec!["--clipboard", "--input"]),
    ];

    for (cmd, args) in &commands {
        if let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                if stdin.write_all(text.as_bytes()).is_ok() {
                    drop(child.stdin.take());
                    return child.wait().map(|s| s.success()).unwrap_or(false);
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdcx_core::api_client::ApiClient;
    use cdcx_core::env::Environment;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn make_app() -> App {
        let (tx, _rx) = mpsc::unbounded_channel();
        let state = AppState {
            instruments: vec!["BTC_USDT".into()],
            tickers: std::collections::HashMap::new(),
            sparklines: std::collections::HashMap::new(),
            alerts: vec![],
            authenticated: false,
            env: Environment::Production,
            theme: crate::theme::Theme::default(),
            terminal_size: (120, 40),
            market_connection: crate::state::ConnectionStatus::Connecting,
            api: Arc::new(ApiClient::new(None, Environment::Production)),
            rest_tx: tx,
            price_api: Arc::new(cdcx_core::price_api::PriceApiClient::new()),
            price_api_tx: {
                let (ptx, _prx) = mpsc::unbounded_channel();
                ptx
            },
            price_directory: None,
            toast: None,
            session_start_value: None,
            current_portfolio_value: 0.0,
            ticker_speed_divisor: 2,
            price_flashes: std::collections::HashMap::new(),
            paper_mode: false,
            paper_engine: None,
            volume_unit: crate::state::VolumeUnit::Usd,
            pending_navigation: None,
            instrument_types: std::collections::HashMap::new(),
            instrument_bases: std::collections::HashMap::new(),
            user_connection: crate::state::ConnectionStatus::Error,
            isolated_positions: std::collections::HashMap::new(),
            positions_snapshot: Vec::new(),
        };
        let mut app = App::new(state, &[]);
        // Seed market tab with an instrument so workflow triggers have something to select
        app.state.instruments = vec!["BTC_USDT".into()];
        app
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn key_shift(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    #[test]
    fn test_spotlight_dismiss_does_not_swallow_trade_key() {
        let mut app = make_app();
        app.show_spotlight = true;
        app.on_key(key('t'));
        assert!(!app.show_spotlight, "spotlight should be dismissed");
        assert_eq!(
            app.mode,
            Mode::Workflow,
            "trade workflow should have started"
        );
    }

    #[test]
    fn test_spotlight_dismiss_does_not_swallow_oco_key() {
        let mut app = make_app();
        app.show_spotlight = true;
        app.on_key(key('o'));
        assert!(!app.show_spotlight, "spotlight should be dismissed");
        assert_eq!(app.mode, Mode::Workflow, "OCO workflow should have started");
    }

    #[test]
    fn test_spotlight_dismiss_does_not_swallow_otoco_key() {
        let mut app = make_app();
        app.show_spotlight = true;
        app.on_key(key_shift('O'));
        assert!(!app.show_spotlight, "spotlight should be dismissed");
        assert_eq!(
            app.mode,
            Mode::Workflow,
            "OTOCO workflow should have started"
        );
    }

    #[test]
    fn test_spotlight_dismiss_consumes_other_keys() {
        let mut app = make_app();
        app.show_spotlight = true;
        app.on_key(key('x'));
        assert!(!app.show_spotlight, "spotlight should be dismissed");
        assert_eq!(app.mode, Mode::Normal, "no workflow should start for 'x'");
    }

    #[test]
    fn test_spotlight_dismiss_does_not_quit_on_q() {
        let mut app = make_app();
        app.show_spotlight = true;
        app.on_key(key('q'));
        assert!(!app.show_spotlight, "spotlight should be dismissed");
        assert!(
            !app.should_quit,
            "'q' should be consumed by spotlight dismiss, not quit"
        );
    }

    #[test]
    fn test_volume_toggle_preserves_market_order() {
        let mut app = make_app();
        app.state.instruments = vec!["AAA_USDT".into(), "BBB_USDT".into(), "CCC_USDT".into()];
        app.state
            .instrument_types
            .insert("AAA_USDT".into(), "CCY_PAIR".into());
        app.state
            .instrument_types
            .insert("BBB_USDT".into(), "CCY_PAIR".into());
        app.state
            .instrument_types
            .insert("CCC_USDT".into(), "CCY_PAIR".into());

        app.state.tickers.insert(
            "AAA_USDT".into(),
            crate::state::TickerData {
                instrument: "AAA_USDT".into(),
                ask: 100.0,
                bid: 99.0,
                change_pct: 0.0,
                high: 110.0,
                low: 90.0,
                volume: 1_000.0,
                volume_usd: 100_000.0,
                funding_rate: 0.0,
            },
        );
        app.state.tickers.insert(
            "BBB_USDT".into(),
            crate::state::TickerData {
                instrument: "BBB_USDT".into(),
                ask: 200.0,
                bid: 199.0,
                change_pct: 0.0,
                high: 210.0,
                low: 190.0,
                volume: 2_000.0,
                volume_usd: 200_000.0,
                funding_rate: 0.0,
            },
        );
        app.state.tickers.insert(
            "CCC_USDT".into(),
            crate::state::TickerData {
                instrument: "CCC_USDT".into(),
                ask: 150.0,
                bid: 149.0,
                change_pct: 0.0,
                high: 160.0,
                low: 140.0,
                volume: 1_500.0,
                volume_usd: 150_000.0,
                funding_rate: 0.0,
            },
        );

        // Trigger refilter: enter search mode then backspace (refilter), then exit
        app.on_key(key('/'));
        let backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        app.on_key(backspace);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        app.on_key(enter);

        let csv_before = app
            .tabs
            .get(0)
            .and_then(|t| t.export_csv(&app.state))
            .expect("market csv");
        let rows_before: Vec<&str> = csv_before.lines().skip(1).collect();

        app.on_key(key('v'));

        let csv_after = app
            .tabs
            .get(0)
            .and_then(|t| t.export_csv(&app.state))
            .expect("market csv");
        let rows_after: Vec<&str> = csv_after.lines().skip(1).collect();

        let insts_before: Vec<&str> = rows_before
            .iter()
            .map(|r| r.split(',').next().unwrap())
            .collect();
        let insts_after: Vec<&str> = rows_after
            .iter()
            .map(|r| r.split(',').next().unwrap())
            .collect();

        assert_eq!(
            insts_before, insts_after,
            "instrument order should not change when toggling volume unit"
        );

        // prices should remain unchanged
        for (r_before, r_after) in rows_before.iter().zip(rows_after.iter()) {
            let cols_b: Vec<&str> = r_before.split(',').collect();
            let cols_a: Vec<&str> = r_after.split(',').collect();
            assert_eq!(cols_b[1], cols_a[1], "price should not change");
            assert_eq!(cols_b[3], cols_a[3], "high should not change");
            assert_eq!(cols_b[4], cols_a[4], "low should not change");
        }
    }
}
