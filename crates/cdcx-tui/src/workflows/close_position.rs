use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::{AppState, RestRequest};
use crate::workflows::{modal_area, Workflow, WorkflowResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Choose MARKET (immediate, slippage risk) or LIMIT (asks for price).
    OrderType,
    /// Only reached when OrderType = LIMIT.
    Price,
    Confirm,
    Submitting,
    Rejected,
}

pub struct ClosePositionWorkflow {
    instrument: String,
    /// Original position side — either "BUY"/"LONG" or "SELL"/"SHORT". The close order
    /// takes the opposite side.
    position_side: String,
    /// Raw quantity string as returned by the exchange — preserves the exact precision
    /// accepted for this instrument, avoiding qty_tick_size rounding errors that would
    /// leave dust if we reformatted from f64.
    quantity_str: String,
    entry_price: f64,
    mark_price: f64,
    is_isolated: bool,
    isolation_id: Option<String>,
    step: Step,
    order_type: usize, // 0 = MARKET, 1 = LIMIT
    price_input: String,
    error: Option<String>,
    rejection: Option<(i64, String)>,
}

impl ClosePositionWorkflow {
    /// Build the workflow from the positions snapshot. Returns None if the instrument
    /// has no open position, or if the position has zero quantity (closed).
    pub fn new(instrument: String, state: &AppState) -> Option<Self> {
        let pos = state.positions_snapshot.iter().find(|p| {
            p.get("instrument_name").and_then(|v| v.as_str()) == Some(instrument.as_str())
        })?;
        let quantity_str = pos.get("quantity").and_then(|v| v.as_str())?.to_string();
        let quantity: f64 = quantity_str.parse().ok()?;
        if quantity.abs() < 1e-12 {
            return None;
        }
        let position_side = pos
            .get("side")
            .and_then(|v| v.as_str())
            .unwrap_or(if quantity > 0.0 { "BUY" } else { "SELL" })
            .to_string();
        // API returns `open_pos_cost` (total USD) rather than per-unit price. Divide
        // by |quantity| to recover entry price; prefer `average_price` if present.
        let entry_price = pos
            .get("average_price")
            .or_else(|| pos.get("avg_entry_price"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|p| *p > 0.0)
            .or_else(|| {
                pos.get("open_pos_cost")
                    .or_else(|| pos.get("cost"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .and_then(|cost| {
                        let abs_qty = quantity.abs();
                        if abs_qty > 0.0 {
                            Some(cost.abs() / abs_qty)
                        } else {
                            None
                        }
                    })
            })
            .unwrap_or(0.0);
        let mark_price = state
            .tickers
            .get(&instrument)
            .map(|t| t.ask)
            .unwrap_or(entry_price);
        let is_isolated = pos
            .get("isolation_type")
            .and_then(|v| v.as_str())
            .map(|s| s == "ISOLATED_MARGIN")
            .unwrap_or(false);
        let isolation_id = state.isolated_positions.get(&instrument).cloned();

        let _ = quantity; // parsed only as validity check — use quantity_str for submit
        Some(Self {
            instrument,
            position_side,
            quantity_str,
            entry_price,
            mark_price,
            is_isolated,
            isolation_id,
            step: Step::OrderType,
            order_type: 0,
            price_input: String::new(),
            error: None,
            rejection: None,
        })
    }

    /// The side for the close order — opposite of the position's side.
    fn close_side(&self) -> &'static str {
        match self.position_side.as_str() {
            "BUY" | "LONG" => "SELL",
            _ => "BUY",
        }
    }

    fn type_str(&self) -> &'static str {
        if self.order_type == 0 {
            "MARKET"
        } else {
            "LIMIT"
        }
    }

    fn submit(&self, state: &AppState) {
        // Strip any leading sign from the quantity — some venue responses include it,
        // but create-order always wants a positive quantity plus an explicit side.
        let qty = self
            .quantity_str
            .trim_start_matches('-')
            .trim_start_matches('+')
            .to_string();
        let mut params = serde_json::json!({
            "instrument_name": self.instrument,
            "side": self.close_side(),
            "type": self.type_str(),
            "quantity": qty,
        });
        if self.order_type == 1 {
            params["price"] = serde_json::Value::String(self.price_input.clone());
        }
        if self.is_isolated {
            params["exec_inst"] = serde_json::json!(["ISOLATED_MARGIN"]);
            // For isolated positions, isolation_id is mandatory — otherwise we get 617.
            // REDUCE_ONLY on an isolated position tells the exchange this must reduce
            // the existing bucket rather than open a new one.
            if let Some(ref id) = self.isolation_id {
                params["isolation_id"] = serde_json::Value::String(id.clone());
            }
            if let Some(arr) = params["exec_inst"].as_array_mut() {
                arr.push(serde_json::Value::String("REDUCE_ONLY".into()));
            }
        } else {
            // Cross-margin close: REDUCE_ONLY still protects against accidentally
            // flipping from long to short if quantity math ever goes wrong.
            params["exec_inst"] = serde_json::json!(["REDUCE_ONLY"]);
        }
        // Stamp cx3- TUI origin prefix on client_oid for downstream attribution.
        let _ = cdcx_core::origin::tag_params_in_place(
            &mut params,
            cdcx_core::origin::OriginChannel::Tui,
        );
        let _ = state.rest_tx.send(RestRequest {
            method: "private/create-order".into(),
            params,
            is_private: true,
        });
    }
}

impl Workflow for ClosePositionWorkflow {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> WorkflowResult {
        if key.code == KeyCode::Esc {
            return WorkflowResult::Cancel;
        }

        match self.step {
            Step::OrderType => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                    self.order_type = 1 - self.order_type;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.step = if self.order_type == 0 {
                        Step::Confirm
                    } else {
                        // Pre-fill LIMIT price with mark so the user sees a sensible
                        // anchor; they can edit/clear it with Backspace.
                        if self.price_input.is_empty() && self.mark_price > 0.0 {
                            self.price_input = format!("{:.2}", self.mark_price);
                        }
                        Step::Price
                    };
                }
                _ => {}
            },
            Step::Price => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                    self.price_input.push(c);
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.price_input.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if self.price_input.is_empty() {
                        self.error = Some("Price is required for LIMIT close".into());
                    } else if self.price_input.parse::<f64>().is_err()
                        || self.price_input.parse::<f64>().unwrap() <= 0.0
                    {
                        self.error = Some("Invalid price — enter a positive number".into());
                    } else {
                        self.error = None;
                        self.step = Step::Confirm;
                    }
                }
                KeyCode::BackTab => {
                    self.step = Step::OrderType;
                }
                _ => {}
            },
            Step::Confirm => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.step = Step::Submitting;
                    self.submit(state);
                }
                KeyCode::BackTab => {
                    self.step = if self.order_type == 0 {
                        Step::OrderType
                    } else {
                        Step::Price
                    };
                }
                _ => {}
            },
            Step::Submitting => {
                // on_response() transitions to Rejected or returns Done.
            }
            Step::Rejected => match key.code {
                KeyCode::Char('r' | 'R') | KeyCode::Enter => {
                    self.rejection = None;
                    self.step = Step::Submitting;
                    self.submit(state);
                }
                KeyCode::Char('e' | 'E') => {
                    self.rejection = None;
                    self.step = Step::Confirm;
                }
                _ => {}
            },
        }
        WorkflowResult::Continue
    }

    fn on_response(
        &mut self,
        method: &str,
        data: &serde_json::Value,
        state: &mut AppState,
    ) -> WorkflowResult {
        if method != "private/create-order" || self.step != Step::Submitting {
            return WorkflowResult::Continue;
        }
        let code = data
            .get("data")
            .and_then(|d| d.get("code"))
            .and_then(|v| v.as_i64())
            .or_else(|| data.get("code").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        if code == 0 {
            state.toast(
                format!("Close order submitted — {}", self.instrument),
                crate::state::ToastStyle::Success,
            );
            return WorkflowResult::Done;
        }
        let message = data
            .get("data")
            .and_then(|d| d.get("message"))
            .or_else(|| data.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Close rejected")
            .to_string();
        if code == 617 && self.isolation_id.is_none() {
            // Cache missed — refresh positions so the retry can auto-attach the ID.
            let _ = state.rest_tx.send(RestRequest {
                method: "private/get-positions".into(),
                params: serde_json::json!({}),
                is_private: true,
            });
        }
        self.rejection = Some((code, message));
        self.step = Step::Rejected;
        WorkflowResult::Continue
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        let modal = modal_area(area, 60, 16);
        frame.render_widget(Clear, modal);

        let title = if self.is_isolated {
            " Close Position (Isolated) "
        } else {
            " Close Position "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(state.theme.colors.negative))
            .title(title);
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let lines = Layout::vertical([
            Constraint::Length(1), // spacing
            Constraint::Length(1), // position summary 1
            Constraint::Length(1), // position summary 2
            Constraint::Length(1), // spacing
            Constraint::Length(1), // order type
            Constraint::Length(1), // price (LIMIT only — blank otherwise)
            Constraint::Length(1), // separator
            Constraint::Length(2), // confirm / status
            Constraint::Length(2), // error / rejection
            Constraint::Length(1), // help
        ])
        .areas::<10>(inner);

        let muted = Style::default().fg(state.theme.colors.muted);
        let active = Style::default()
            .fg(state.theme.colors.accent)
            .add_modifier(Modifier::BOLD);

        let side_color = if matches!(self.position_side.as_str(), "BUY" | "LONG") {
            state.theme.colors.positive
        } else {
            state.theme.colors.negative
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Position: ", muted),
                Span::styled(
                    self.position_side.as_str(),
                    Style::default().fg(side_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(self.quantity_str.as_str(), active),
                Span::raw(" "),
                Span::styled(
                    self.instrument.as_str(),
                    Style::default()
                        .fg(state.theme.colors.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            lines[1],
        );

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Entry: ", muted),
                Span::raw(format!("{:.2}", self.entry_price)),
                Span::raw("   "),
                Span::styled("Mark: ", muted),
                Span::raw(format!("{:.2}", self.mark_price)),
                Span::raw("   "),
                Span::styled(
                    if self.is_isolated {
                        "[ISOLATED]"
                    } else {
                        "[CROSS]"
                    },
                    muted,
                ),
            ])),
            lines[2],
        );

        // Order type row
        let type_label_style = if self.step == Step::OrderType {
            active
        } else {
            muted
        };
        let market_style = if self.order_type == 0 { active } else { muted };
        let limit_style = if self.order_type == 1 { active } else { muted };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Close via: ", type_label_style),
                Span::styled(
                    if self.order_type == 0 {
                        "[MARKET]"
                    } else {
                        " MARKET "
                    },
                    market_style,
                ),
                Span::raw("  "),
                Span::styled(
                    if self.order_type == 1 {
                        "[LIMIT]"
                    } else {
                        " LIMIT "
                    },
                    limit_style,
                ),
            ])),
            lines[4],
        );

        // Price row (only when LIMIT chosen)
        if self.order_type == 1 {
            let price_style = if self.step == Step::Price {
                active
            } else {
                muted
            };
            let price_display = if self.price_input.is_empty() && self.step == Step::Price {
                "\u{2588}".to_string()
            } else if self.step == Step::Price {
                format!("{}\u{2588}", self.price_input)
            } else {
                self.price_input.clone()
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("Price:     ", price_style),
                    Span::styled(
                        price_display,
                        if self.step == Step::Price {
                            active
                        } else {
                            muted
                        },
                    ),
                ])),
                lines[5],
            );
        }

        frame.render_widget(
            Paragraph::new("\u{2500}".repeat(inner.width as usize))
                .style(Style::default().fg(state.theme.colors.border)),
            lines[6],
        );

        match self.step {
            Step::Confirm => {
                let price_segment = if self.order_type == 0 {
                    "@ MARKET".to_string()
                } else {
                    format!("@ {}", self.price_input)
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!(
                            "Close: {} {} {} {}",
                            self.close_side(),
                            self.quantity_str,
                            self.instrument,
                            price_segment,
                        ),
                        Style::default()
                            .fg(state.theme.colors.negative)
                            .add_modifier(Modifier::BOLD),
                    )])),
                    lines[7],
                );
            }
            Step::Submitting => {
                frame.render_widget(
                    Paragraph::new("Submitting close order...")
                        .style(Style::default().fg(state.theme.colors.accent)),
                    lines[7],
                );
            }
            Step::Rejected => {
                frame.render_widget(
                    Paragraph::new("Close rejected by exchange")
                        .style(Style::default().fg(state.theme.colors.negative)),
                    lines[7],
                );
            }
            _ => {}
        }

        if let Some((code, ref message)) = self.rejection {
            let has_isolation_id = self.isolation_id.is_some()
                || state.isolated_positions.contains_key(&self.instrument);
            let hint = match code {
                617 if has_isolation_id => "  (R to retry with isolation_id)".to_string(),
                617 => "  (waiting on positions refresh; try R again shortly)".to_string(),
                _ => String::new(),
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("[{}] {}", code, message),
                        Style::default().fg(state.theme.colors.negative),
                    ),
                    Span::styled(hint, muted),
                ])),
                lines[8],
            );
        } else if let Some(ref err) = self.error {
            frame.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(state.theme.colors.negative)),
                lines[8],
            );
        }

        let help = match self.step {
            Step::OrderType => "\u{2190}\u{2192}/Space:toggle  Enter:next  Esc:cancel",
            Step::Price => "type price  Enter:next  Shift+Tab:back  Esc:cancel",
            Step::Confirm => "Enter/y:close position  Shift+Tab:back  Esc:cancel",
            Step::Submitting => "waiting...",
            Step::Rejected => "R/Enter:retry  E:edit  Esc:cancel",
        };
        frame.render_widget(Paragraph::new(help).style(muted), lines[9]);
    }
}
