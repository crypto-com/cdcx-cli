use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::format::format_price;
use crate::state::{AppState, RestRequest};
use crate::tabs::{DataEvent, Tab, TabKind};

const PAGE_SIZE: usize = 100;

fn days_to_ymd(days_since_epoch: i64) -> (i64, u32, u32) {
    // Civil days to Y/M/D (algorithm from http://howardhinnant.github.io/date_algorithms.html)
    let z = days_since_epoch + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[derive(Debug, Clone)]
struct HistoryOrder {
    instrument: String,
    side: String,
    order_type: String,
    price: String,
    quantity: f64,
    status: String,
    time: String,
    create_time_ns: u64,
}

pub struct HistoryTab {
    orders: Vec<HistoryOrder>,
    loaded: bool,
    requesting: bool,
    selected: usize,
    scroll_offset: usize,
    /// Stack of end_time cursors for previous pages (enables going back).
    cursor_stack: Vec<u64>,
    /// The end_time cursor for the next page (earliest create_time of current batch).
    next_cursor: Option<u64>,
    has_next: bool,
}

impl Default for HistoryTab {
    fn default() -> Self {
        Self::new()
    }
}

impl HistoryTab {
    pub fn new() -> Self {
        Self {
            orders: vec![],
            loaded: false,
            requesting: false,
            selected: 0,
            scroll_offset: 0,
            cursor_stack: vec![],
            next_cursor: None,
            has_next: true,
        }
    }

    fn request_data(&mut self, state: &AppState) {
        if state.authenticated && !state.paper_mode && !self.requesting {
            self.requesting = true;
            let mut params = serde_json::json!({"limit": PAGE_SIZE});
            if let Some(cursor) = self.next_cursor {
                params["end_time"] = serde_json::json!(cursor);
            }
            let _ = state.rest_tx.send(RestRequest {
                method: "private/get-order-history".into(),
                params,
                is_private: true,
            });
        }
    }

    fn visible_rows(&self, terminal_height: u16) -> usize {
        // terminal_height - ticker(1) - tab_bar(3) - status(1) - footer(1) - table_header(1) = data rows
        (terminal_height as usize).saturating_sub(7)
    }

    fn load_paper_history(&mut self, state: &AppState) {
        if let Some(ref engine) = state.paper_engine {
            self.orders = engine
                .state
                .trade_history
                .iter()
                .rev()
                .map(|t| {
                    HistoryOrder {
                        instrument: t.instrument_name.clone(),
                        side: format!("{:?}", t.side).to_uppercase(),
                        order_type: "MARKET".into(),
                        price: format_price(t.price),
                        quantity: t.quantity,
                        status: "FILLED".into(),
                        time: t.timestamp.chars().take(19).collect(),
                        create_time_ns: 0,
                    }
                })
                .collect();
            self.loaded = true;
        }
    }
}

impl Tab for HistoryTab {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool {
        match key.code {
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    if self.selected < self.scroll_offset {
                        self.scroll_offset = self.selected;
                    }
                }
                true
            }
            KeyCode::Down => {
                if self.selected < self.orders.len().saturating_sub(1) {
                    self.selected += 1;
                    let vis = self.visible_rows(state.terminal_size.1);
                    if self.selected >= self.scroll_offset + vis {
                        self.scroll_offset = self.selected + 1 - vis;
                    }
                }
                true
            }
            KeyCode::Right if !state.paper_mode && self.loaded && self.has_next => {
                // Push current cursor so Left can restore it
                if let Some(first_ts) = self.orders.first().map(|o| o.create_time_ns) {
                    self.cursor_stack.push(first_ts);
                }
                self.loaded = false;
                self.selected = 0;
                self.scroll_offset = 0;
                self.request_data(state);
                true
            }
            KeyCode::Left if !state.paper_mode && self.loaded && !self.cursor_stack.is_empty() => {
                // Pop previous page's start_time and use it as the new end_time + 1ns
                // to re-fetch that page. If stack is empty after pop, fetch first page.
                let prev_cursor = self.cursor_stack.pop();
                // The cursor we popped was the first (newest) item's create_time of that page.
                // To re-fetch that page, we need end_time > that timestamp (or no end_time for first page).
                if self.cursor_stack.is_empty() {
                    self.next_cursor = None;
                } else {
                    self.next_cursor = prev_cursor;
                }
                self.loaded = false;
                self.selected = 0;
                self.scroll_offset = 0;
                self.request_data(state);
                true
            }
            KeyCode::Enter => {
                if let Some(order) = self.orders.get(self.selected) {
                    state.pending_navigation =
                        Some((TabKind::Market, order.instrument.clone()));
                }
                true
            }
            KeyCode::Char('r') => {
                self.loaded = false;
                self.requesting = false;
                true
            }
            _ => false,
        }
    }

    fn on_data(&mut self, event: &DataEvent, state: &mut AppState) {
        if state.paper_mode {
            self.load_paper_history(state);
            return;
        }
        match event {
            DataEvent::RestResponse { method, data } if method == "private/get-order-history" => {
                self.requesting = false;
                let arr_opt = data
                    .get("order_list")
                    .and_then(|d| d.as_array())
                    .or_else(|| data.get("data").and_then(|d| d.as_array()));
                if let Some(arr) = arr_opt {
                    self.has_next = arr.len() >= PAGE_SIZE;
                    self.orders = arr
                        .iter()
                        .map(|item| {
                            let create_time_ns = item
                                .get("create_time_ns")
                                .and_then(|v| v.as_u64())
                                .or_else(|| {
                                    item.get("create_time")
                                        .and_then(|v| v.as_u64())
                                        .map(|ms| ms * 1_000_000)
                                })
                                .unwrap_or(0);
                            HistoryOrder {
                                instrument: item
                                    .get("instrument_name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                side: item
                                    .get("side")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                order_type: item
                                    .get("order_type")
                                    .or_else(|| item.get("type"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                price: format_price(
                                    item.get("avg_price")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| *s != "0")
                                        .or_else(|| {
                                            item.get("limit_price")
                                                .and_then(|v| v.as_str())
                                                .filter(|s| *s != "0")
                                        })
                                        .or_else(|| {
                                            item.get("price")
                                                .and_then(|v| v.as_str())
                                                .filter(|s| *s != "0")
                                        })
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0.0),
                                ),
                                quantity: item
                                    .get("quantity")
                                    .and_then(|v| v.as_str())
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0.0),
                                status: item
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                time: item
                                    .get("create_time")
                                    .and_then(|v| v.as_u64())
                                    .map(|ms| {
                                        let secs = (ms / 1000) as i64;
                                        let days = secs / 86400;
                                        let time_of_day = secs % 86400;
                                        let hours = time_of_day / 3600;
                                        let minutes = (time_of_day % 3600) / 60;
                                        // Days since Unix epoch to Y-M-D
                                        let (y, m, d) = days_to_ymd(days);
                                        format!(
                                            "{:04}-{:02}-{:02} {:02}:{:02}",
                                            y, m, d, hours, minutes
                                        )
                                    })
                                    .unwrap_or_default(),
                                create_time_ns,
                            }
                        })
                        .collect();
                    // Use the earliest create_time as cursor for next page
                    self.next_cursor = self
                        .orders
                        .iter()
                        .map(|o| o.create_time_ns)
                        .filter(|t| *t > 0)
                        .min();
                    self.loaded = true;
                }
            }
            _ => {
                if !self.loaded {
                    self.request_data(state);
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        if !state.paper_mode && !state.authenticated {
            frame.render_widget(
                Paragraph::new(
                    "History \u{2014} not authenticated. Set CDC_API_KEY and CDC_API_SECRET.",
                )
                .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }
        if !self.loaded {
            frame.render_widget(
                Paragraph::new("History \u{2014} loading...")
                    .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        let [table_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        if self.orders.is_empty() {
            frame.render_widget(
                Paragraph::new(if state.paper_mode {
                    "No paper trades yet."
                } else if !self.cursor_stack.is_empty() {
                    "No more orders."
                } else {
                    "No order history."
                })
                .style(Style::default().fg(state.theme.colors.muted)),
                table_area,
            );
        } else {
            let header = Row::new(vec![
                "Instrument",
                "Side",
                "Type",
                "Price",
                "Qty",
                "Status",
                "Time",
            ])
            .style(
                Style::default()
                    .fg(state.theme.colors.header)
                    .add_modifier(Modifier::BOLD),
            );
            let widths = [
                Constraint::Length(16),
                Constraint::Length(6),
                Constraint::Length(8),
                Constraint::Length(14),
                Constraint::Length(12),
                Constraint::Length(16),
                Constraint::Length(16),
            ];

            let vis = (table_area.height as usize).saturating_sub(1);
            let end = (self.scroll_offset + vis).min(self.orders.len());
            let visible_slice = &self.orders[self.scroll_offset..end];

            let rows: Vec<Row> = visible_slice
                .iter()
                .enumerate()
                .map(|(vi, o)| {
                    let abs_idx = self.scroll_offset + vi;
                    let is_selected = abs_idx == self.selected;
                    let side_color = if o.side == "BUY" {
                        state.theme.colors.positive
                    } else {
                        state.theme.colors.negative
                    };
                    let status_color = match o.status.as_str() {
                        "FILLED" => state.theme.colors.positive,
                        "CANCELED" | "CANCELLED" => state.theme.colors.negative,
                        "EXPIRED" => state.theme.colors.muted,
                        "REJECTED" => state.theme.colors.negative,
                        _ => state.theme.colors.fg,
                    };

                    let row_style = if is_selected {
                        Style::default()
                            .fg(state.theme.colors.selected_fg)
                            .bg(state.theme.colors.selected_bg)
                    } else {
                        Style::default().fg(state.theme.colors.fg)
                    };

                    if is_selected {
                        Row::new(vec![
                            Cell::from(o.instrument.as_str()),
                            Cell::from(o.side.as_str()),
                            Cell::from(o.order_type.as_str()),
                            Cell::from(o.price.as_str()),
                            Cell::from(format!("{:.4}", o.quantity)),
                            Cell::from(o.status.as_str()),
                            Cell::from(o.time.as_str()),
                        ])
                        .style(row_style)
                    } else {
                        Row::new(vec![
                            Cell::from(o.instrument.as_str()),
                            Cell::from(o.side.as_str()).style(Style::default().fg(side_color)),
                            Cell::from(o.order_type.as_str()),
                            Cell::from(o.price.as_str()),
                            Cell::from(format!("{:.4}", o.quantity)),
                            Cell::from(o.status.as_str()).style(Style::default().fg(status_color)),
                            Cell::from(o.time.as_str())
                                .style(Style::default().fg(state.theme.colors.muted)),
                        ])
                        .style(row_style)
                    }
                })
                .collect();

            frame.render_widget(
                Table::new(rows, widths).header(header).column_spacing(1),
                table_area,
            );
        }

        let page_info = if state.paper_mode {
            format!("{} trades", self.orders.len())
        } else {
            format!("Page {}", self.cursor_stack.len() + 1)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{} ", page_info),
                    Style::default().fg(state.theme.colors.accent),
                ),
                Span::styled(
                    if state.paper_mode {
                        "r:refresh  \u{2191}\u{2193}:navigate"
                    } else {
                        "\u{2190}\u{2192}:page  r:refresh  \u{2191}\u{2193}:navigate"
                    },
                    Style::default().fg(state.theme.colors.muted),
                ),
            ])),
            footer_area,
        );
    }

    fn subscriptions(&self, _state: &AppState) -> Vec<String> {
        vec![]
    }

    fn on_click(&mut self, row: u16, _col: u16, _state: &mut AppState) -> bool {
        // Layout: row 0 = table header, row 1+ = data rows, last row = footer
        if row >= 1 {
            let data_row = (row - 1) as usize + self.scroll_offset;
            if data_row < self.orders.len() {
                self.selected = data_row;
                return true;
            }
        }
        false
    }

    fn on_double_click(&mut self, row: u16, _col: u16, state: &mut AppState) -> bool {
        if row >= 1 {
            let data_row = (row - 1) as usize + self.scroll_offset;
            if data_row < self.orders.len() {
                self.selected = data_row;
                if let Some(order) = self.orders.get(self.selected) {
                    state.pending_navigation =
                        Some((TabKind::Market, order.instrument.clone()));
                }
                return true;
            }
        }
        false
    }

    fn on_activate(&mut self) {
        self.loaded = false;
        self.requesting = false;
        self.has_next = true;
        self.cursor_stack.clear();
        self.next_cursor = None;
        self.selected = 0;
        self.scroll_offset = 0;
    }
}
