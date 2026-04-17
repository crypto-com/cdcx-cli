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
                // Fetch candles for split view if instrument changed
                if self.split_view {
                    let new_instrument = self.get_selected_instrument();
                    if new_instrument != prev_instrument {
                        if let Some(inst) = new_instrument {
                            let _ = self.state.rest_tx.send(crate::state::RestRequest {
                                method: "public/get-candlestick".into(),
                                params: serde_json::json!({"instrument_name": inst, "timeframe": "1h"}),
                                is_private: false,
                            });
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
                // Fetch candle data for selected instrument when entering split
                if self.split_view {
                    if let Some(inst) = self.get_selected_instrument() {
                        let _ = self.state.rest_tx.send(crate::state::RestRequest {
                            method: "public/get-candlestick".into(),
                            params: serde_json::json!({"instrument_name": inst, "timeframe": "1h"}),
                            is_private: false,
                        });
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

        // In split view, refetch candles when selected instrument changes
        if self.split_view {
            let new_instrument = self.get_selected_instrument();
            if new_instrument != prev_instrument {
                if let Some(inst) = new_instrument {
                    let _ = self.state.rest_tx.send(crate::state::RestRequest {
                        method: "public/get-candlestick".into(),
                        params: serde_json::json!({"instrument_name": inst, "timeframe": "1h"}),
                        is_private: false,
                    });
                }
            }
        }

        if consumed {
            return;
        }

        // Workflow triggers only if the tab didn't consume the key
        let tab_kind = TabKind::ALL.get(self.active_tab).copied();
        match (tab_kind, key.code) {
            (Some(TabKind::Market), KeyCode::Char('t')) => {
                let instrument = self.get_selected_instrument().unwrap_or("BTC_USDT".into());
                if self.state.paper_mode {
                    self.workflow = Some(Box::new(PaperOrderWorkflow::new(instrument)));
                } else {
                    self.workflow = Some(Box::new(PlaceOrderWorkflow::new(instrument)));
                }
                self.mode = Mode::Workflow;
            }
            (Some(TabKind::Market) | Some(TabKind::Orders), KeyCode::Char('c')) => {
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
                    let instrument = self.get_selected_instrument().unwrap_or("BTC_USDT".into());
                    self.workflow = Some(Box::new(CancelOrderWorkflow::new(instrument)));
                    self.mode = Mode::Workflow;
                }
            }
            (Some(TabKind::Market), KeyCode::Char('o')) => {
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
            (Some(TabKind::Market), KeyCode::Char('O')) => {
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

        // Track live portfolio value for status bar from any tab
        if let DataEvent::RestResponse {
            ref method,
            ref data,
        } = event
        {
            if method == "private/user-balance" && !self.state.paper_mode {
                if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                    let mut total = 0.0;
                    for item in arr {
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
                                if matches!(
                                    currency,
                                    "USDT" | "USD" | "USDC" | "DAI" | "TUSD" | "BUSD"
                                ) {
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
            }
        }

        // REST responses for workflows (create-order, cancel-all-orders)
        if let DataEvent::RestResponse {
            ref method,
            data: _,
        } = event
        {
            if self.mode == Mode::Workflow {
                if method == "private/create-order" {
                    self.workflow = None;
                    self.mode = Mode::Normal;
                    self.state
                        .toast("Order submitted", crate::state::ToastStyle::Success);
                    return;
                }
                if method == "private/cancel-all-orders" {
                    self.workflow = None;
                    self.mode = Mode::Normal;
                    self.state
                        .toast("Orders cancelled", crate::state::ToastStyle::Success);
                    return;
                }
            }
        }

        // Forward to active tab
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.on_data(&event, &mut self.state);
        }
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

            // Right panel: chart for selected instrument using Market tab's candle data
            if let Some(inst) = self.get_selected_instrument() {
                let candles = self
                    .tabs
                    .first() // Market tab (index 0)
                    .map(|tab| tab.get_candles(&inst))
                    .unwrap_or(&[]);
                crate::widgets::candlestick::draw_candlestick(
                    frame,
                    right,
                    &inst,
                    candles,
                    "1h",
                    &self.state.theme.colors,
                    "\\:close split",
                );
            }
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
    let height = 33u16;
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
        ("t", "Place order"),
        ("o", "OCO order (stop-loss + take-profit)"),
        ("O", "OTOCO order (entry + SL + TP)"),
        ("c", "Cancel orders"),
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
            toast: None,
            session_start_value: None,
            current_portfolio_value: 0.0,
            ticker_speed_divisor: 2,
            price_flashes: std::collections::HashMap::new(),
            paper_mode: false,
            paper_engine: None,
            pending_navigation: None,
            instrument_types: std::collections::HashMap::new(),
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
}
