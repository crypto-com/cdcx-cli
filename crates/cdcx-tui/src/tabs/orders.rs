use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::state::{AppState, RestRequest};
use crate::tabs::{DataEvent, Tab};

#[derive(Debug, Clone)]
struct Order {
    order_id: String,
    instrument: String,
    side: String,
    order_type: String,
    price: f64,
    quantity: f64,
    filled_qty: f64,
    status: String,
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status.to_ascii_uppercase().as_str(),
        "FILLED" | "CANCELED" | "CANCELLED" | "REJECTED" | "EXPIRED"
    )
}

fn parse_order_record(item: &serde_json::Value) -> Option<Order> {
    let order_id = item
        .get("order_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if order_id.is_empty() {
        return None;
    }
    Some(Order {
        order_id,
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
            .get("limit_price")
            .or_else(|| item.get("price"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        quantity: item
            .get("quantity")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        filled_qty: item
            .get("cumulative_quantity")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
        status: item
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

pub struct OrdersTab {
    orders: Vec<Order>,
    loaded: bool,
    selected: usize,
}

impl Default for OrdersTab {
    fn default() -> Self {
        Self::new()
    }
}

impl OrdersTab {
    pub fn new() -> Self {
        Self {
            orders: vec![],
            loaded: false,
            selected: 0,
        }
    }

    fn request_data(&self, state: &AppState) {
        if state.authenticated && !state.paper_mode {
            let _ = state.rest_tx.send(RestRequest {
                method: "private/get-open-orders".into(),
                params: serde_json::json!({}),
                is_private: true,
            });
        }
    }

    fn load_paper_orders(&mut self, state: &AppState) {
        if let Some(ref engine) = state.paper_engine {
            self.orders = engine
                .state
                .open_orders
                .iter()
                .map(|o| Order {
                    order_id: o.order_id.to_string(),
                    instrument: o.instrument_name.clone(),
                    side: format!("{:?}", o.side).to_uppercase(),
                    order_type: format!("{:?}", o.order_type).to_uppercase(),
                    price: o.price.unwrap_or(0.0),
                    quantity: o.quantity,
                    filled_qty: 0.0,
                    status: format!("{:?}", o.status).to_uppercase(),
                })
                .collect();
            self.loaded = true;
        }
    }

    /// Apply a batch of order deltas from the `user.order` WS channel.
    /// Terminal statuses remove the row; all others upsert by `order_id`.
    fn apply_order_updates(&mut self, records: &[serde_json::Value]) {
        for item in records {
            let Some(order) = parse_order_record(item) else {
                continue;
            };
            if is_terminal_status(&order.status) {
                self.orders.retain(|o| o.order_id != order.order_id);
                continue;
            }
            if let Some(existing) = self
                .orders
                .iter_mut()
                .find(|o| o.order_id == order.order_id)
            {
                *existing = order;
            } else {
                self.orders.push(order);
            }
        }
        if self.selected >= self.orders.len() {
            self.selected = self.orders.len().saturating_sub(1);
        }
        self.loaded = true;
    }
}

impl Tab for OrdersTab {
    fn on_key(&mut self, key: KeyEvent, _state: &mut AppState) -> bool {
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
            KeyCode::Char('r') => {
                self.loaded = false;
                true
            }
            _ => false,
        }
    }

    fn on_data(&mut self, event: &DataEvent, state: &mut AppState) {
        if state.paper_mode {
            self.load_paper_orders(state);
            return;
        }
        match event {
            DataEvent::RestResponse { method, data } if method == "private/get-open-orders" => {
                let arr_opt = data
                    .get("order_list")
                    .and_then(|d| d.as_array())
                    .or_else(|| data.get("data").and_then(|d| d.as_array()));
                if let Some(arr) = arr_opt {
                    self.orders = arr
                        .iter()
                        .filter_map(parse_order_record)
                        .filter(|o| !is_terminal_status(&o.status))
                        .collect();
                    self.loaded = true;
                }
            }
            DataEvent::OrdersUpdate(records) => {
                self.apply_order_updates(records);
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
                    "Orders \u{2014} not authenticated. Set CDC_API_KEY and CDC_API_SECRET.",
                )
                .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }
        if !self.loaded {
            frame.render_widget(
                Paragraph::new("Orders \u{2014} loading...")
                    .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        let [table_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        if self.orders.is_empty() {
            frame.render_widget(
                Paragraph::new("No open orders.")
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
                "Filled",
                "Status",
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
                Constraint::Length(12),
                Constraint::Length(16),
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
                        "ACTIVE" | "OPEN" => state.theme.colors.positive,
                        "PARTIALLY_FILLED" => state.theme.colors.accent,
                        _ => state.theme.colors.muted,
                    };
                    let fill_pct = if o.quantity > 0.0 {
                        o.filled_qty / o.quantity * 100.0
                    } else {
                        0.0
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
                            Cell::from(format!("{:.1}%", fill_pct)),
                            Cell::from(o.status.as_str()),
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
                            Cell::from(format!("{:.1}%", fill_pct)),
                            Cell::from(o.status.as_str()).style(Style::default().fg(status_color)),
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

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "r:refresh  c:cancel  \u{2191}\u{2193}:navigate",
                Style::default().fg(state.theme.colors.muted),
            ))),
            footer_area,
        );
    }

    fn subscriptions(&self, _state: &AppState) -> Vec<String> {
        vec![]
    }

    fn on_activate(&mut self) {
        // Intentionally no-op: live state is kept in sync via user.order WS upserts,
        // so we don't wipe the cache on tab switch. The initial REST prime runs
        // automatically on first activation via the on_data fallthrough.
    }

    fn selected_instrument(&self) -> Option<&str> {
        self.orders
            .get(self.selected)
            .map(|o| o.instrument.as_str())
    }
}
