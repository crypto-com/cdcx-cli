use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::state::{AppState, RestRequest};
use crate::tabs::{DataEvent, Tab};

const PAGE_SIZE: usize = 50;

#[derive(Debug, Clone)]
struct HistoryOrder {
    instrument: String,
    side: String,
    order_type: String,
    price: f64,
    quantity: f64,
    status: String,
    time: String,
}

pub struct HistoryTab {
    orders: Vec<HistoryOrder>,
    loaded: bool,
    selected: usize,
    page: usize,
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
            selected: 0,
            page: 0,
            has_next: true,
        }
    }

    fn request_data(&self, state: &AppState) {
        if state.authenticated && !state.paper_mode {
            let _ = state.rest_tx.send(RestRequest {
                method: "private/get-order-history".into(),
                params: serde_json::json!({"page_size": PAGE_SIZE.to_string(), "page": self.page.to_string()}),
                is_private: true,
            });
        }
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
                        order_type: "MARKET".into(), // paper trades are always filled
                        price: t.price,
                        quantity: t.quantity,
                        status: "FILLED".into(),
                        time: t.timestamp.chars().take(19).collect(), // trim to datetime
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
                }
                true
            }
            KeyCode::Down => {
                if self.selected < self.orders.len().saturating_sub(1) {
                    self.selected += 1;
                }
                true
            }
            KeyCode::Right if !state.paper_mode && self.has_next => {
                self.page += 1;
                self.loaded = false;
                self.selected = 0;
                self.request_data(state);
                true
            }
            KeyCode::Left if !state.paper_mode && self.page > 0 => {
                self.page -= 1;
                self.loaded = false;
                self.selected = 0;
                self.request_data(state);
                true
            }
            KeyCode::Char('r') => {
                self.loaded = false;
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
                let arr_opt = data
                    .get("order_list")
                    .and_then(|d| d.as_array())
                    .or_else(|| data.get("data").and_then(|d| d.as_array()));
                if let Some(arr) = arr_opt {
                    self.has_next = arr.len() >= PAGE_SIZE;
                    self.orders = arr
                        .iter()
                        .map(|item| HistoryOrder {
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
                            price: item
                                .get("avg_price")
                                .or_else(|| item.get("limit_price"))
                                .or_else(|| item.get("price"))
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0.0),
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
                                .map(|ts| {
                                    let s = ts / 1000;
                                    format!("{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60)
                                })
                                .unwrap_or_default(),
                        })
                        .collect();
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
                } else if self.page > 0 {
                    "No more orders on this page."
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
                Constraint::Length(10),
            ];

            let rows: Vec<Row> = self
                .orders
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let is_selected = i == self.selected;
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
                            Cell::from(if o.price > 0.0 {
                                format!("{:.2}", o.price)
                            } else {
                                "MARKET".into()
                            }),
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
                            Cell::from(if o.price > 0.0 {
                                format!("{:.2}", o.price)
                            } else {
                                "MARKET".into()
                            }),
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
            format!("Page {}", self.page + 1)
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

    fn on_activate(&mut self) {
        self.loaded = false;
        self.has_next = true;
        self.page = 0;
        self.selected = 0;
    }
}
