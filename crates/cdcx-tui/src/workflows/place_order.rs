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
    Instrument,
    Side,
    OrderType,
    Margin,
    Price,
    Quantity,
    Confirm,
    Submitting,
    Rejected,
}

pub struct PlaceOrderWorkflow {
    instrument: String,
    instrument_input: String,
    step: Step,
    side: usize,       // 0 = BUY, 1 = SELL
    order_type: usize, // 0 = LIMIT, 1 = MARKET
    price_input: String,
    qty_input: String,
    error: Option<String>,
    isolated_margin: bool,
    /// Exchange error message + code from the most recent rejection, if any.
    /// When set, the modal sits on `Step::Rejected` and offers retry paths.
    rejection: Option<(i64, String)>,
}

/// inst_types that default to requiring isolated margin on this exchange.
/// Single-stock perpetuals (e.g. SPYUSD-PERP, NVDAUSD-PERP) are the known case.
/// Crypto perpetuals are NOT in this list — they work on cross margin by default.
fn requires_isolated_margin(inst_type: &str) -> bool {
    matches!(inst_type, "EQUITY_PERP" | "EQUITY_PERPETUAL")
}

impl PlaceOrderWorkflow {
    pub fn new(instrument: String, state: &AppState) -> Self {
        let instrument_input = instrument.clone();
        let isolated_default = state
            .instrument_types
            .get(&instrument)
            .map(|t| requires_isolated_margin(t))
            .unwrap_or(false);
        Self {
            instrument,
            instrument_input,
            step: Step::Instrument,
            side: 0,
            order_type: 0,
            price_input: String::new(),
            qty_input: String::new(),
            error: None,
            isolated_margin: isolated_default,
            rejection: None,
        }
    }

    fn side_str(&self) -> &'static str {
        if self.side == 0 {
            "BUY"
        } else {
            "SELL"
        }
    }

    fn type_str(&self) -> &'static str {
        if self.order_type == 0 {
            "LIMIT"
        } else {
            "MARKET"
        }
    }

    fn submit(&self, state: &AppState) {
        let mut params = serde_json::json!({
            "instrument_name": self.instrument,
            "side": self.side_str(),
            "type": self.type_str(),
            "quantity": self.qty_input,
        });
        if self.order_type == 0 {
            params["price"] = serde_json::Value::String(self.price_input.clone());
        }
        if self.isolated_margin {
            params["exec_inst"] = serde_json::json!(["ISOLATED_MARGIN"]);
            // Attach isolation_id if we already have an open isolated position on this
            // instrument. The exchange rejects a second bare-isolated order on the same
            // instrument with error 617 (DUPLICATED_INSTRUMENT_ORDER_FOR_ISOLATED_MARGIN);
            // referencing the existing bucket topping-up / trimming it instead.
            if let Some(id) = state.isolated_positions.get(&self.instrument) {
                params["isolation_id"] = serde_json::Value::String(id.clone());
            }
        }
        // Stamp cx3- TUI origin prefix on client_oid so orders placed from the dashboard
        // are attributable downstream. See cdcx-core::origin for the scheme.
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

impl Workflow for PlaceOrderWorkflow {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> WorkflowResult {
        if key.code == KeyCode::Esc {
            return WorkflowResult::Cancel;
        }

        // Global: M toggles isolated margin from any editable step.
        // Not available while typing free-text (Instrument/Price/Quantity) to avoid
        // swallowing the user's keystroke. From Side/OrderType/Margin/Confirm it's safe.
        if let KeyCode::Char('m' | 'M') = key.code {
            if matches!(
                self.step,
                Step::Side | Step::OrderType | Step::Margin | Step::Confirm
            ) {
                self.isolated_margin = !self.isolated_margin;
                return WorkflowResult::Continue;
            }
        }

        match self.step {
            Step::Instrument => match key.code {
                KeyCode::Char(c) => {
                    self.instrument_input.push(c.to_ascii_uppercase());
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.instrument_input.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    let trimmed = self.instrument_input.trim().to_uppercase();
                    if trimmed.is_empty() {
                        self.error = Some("Instrument name is required".into());
                    } else if !state.instruments.contains(&trimmed) {
                        self.error = Some(format!("Unknown instrument: {}", trimmed));
                    } else {
                        self.instrument = trimmed;
                        // Re-evaluate isolated-margin default for the picked instrument.
                        if let Some(t) = state.instrument_types.get(&self.instrument) {
                            if requires_isolated_margin(t) {
                                self.isolated_margin = true;
                            }
                        }
                        self.error = None;
                        self.step = Step::Side;
                    }
                }
                _ => {}
            },
            Step::Side => match key.code {
                KeyCode::Left | KeyCode::Right => {
                    self.side = 1 - self.side;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.step = Step::OrderType;
                }
                KeyCode::BackTab => {
                    self.step = Step::Instrument;
                }
                _ => {}
            },
            Step::OrderType => match key.code {
                KeyCode::Left | KeyCode::Right => {
                    self.order_type = 1 - self.order_type;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.step = Step::Margin;
                }
                KeyCode::BackTab => {
                    self.step = Step::Side;
                }
                _ => {}
            },
            Step::Margin => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                    self.isolated_margin = !self.isolated_margin;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.step = if self.order_type == 0 {
                        Step::Price
                    } else {
                        Step::Quantity
                    };
                }
                KeyCode::BackTab => {
                    self.step = Step::OrderType;
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
                        self.error = Some("Price is required for LIMIT orders".into());
                    } else if self.price_input.parse::<f64>().is_err()
                        || self.price_input.parse::<f64>().unwrap() <= 0.0
                    {
                        self.error = Some("Invalid price — enter a positive number".into());
                    } else {
                        self.error = None;
                        self.step = Step::Quantity;
                    }
                }
                KeyCode::BackTab => {
                    self.step = Step::Margin;
                }
                _ => {}
            },
            Step::Quantity => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                    self.qty_input.push(c);
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.qty_input.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if self.qty_input.is_empty() {
                        self.error = Some("Quantity is required".into());
                    } else if self.qty_input.parse::<f64>().is_err()
                        || self.qty_input.parse::<f64>().unwrap() <= 0.0
                    {
                        self.error = Some("Invalid quantity — enter a positive number".into());
                    } else {
                        self.error = None;
                        self.step = Step::Confirm;
                    }
                }
                KeyCode::BackTab => {
                    self.step = if self.order_type == 0 {
                        Step::Price
                    } else {
                        Step::Margin
                    };
                }
                _ => {}
            },
            Step::Confirm => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.step = Step::Submitting;
                    self.submit(state);
                }
                KeyCode::BackTab => {
                    self.step = Step::Quantity;
                }
                _ => {}
            },
            Step::Submitting => {
                // on_response() will advance to Rejected or WorkflowResult::Done.
            }
            Step::Rejected => match key.code {
                KeyCode::Char('r' | 'R') | KeyCode::Enter => {
                    // Retry with current settings (user may have pressed M to flip margin).
                    self.rejection = None;
                    self.step = Step::Submitting;
                    self.submit(state);
                }
                KeyCode::Char('e' | 'E') => {
                    // Back to edit — land on Confirm so user can Shift+Tab back through fields.
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
        // The API client returns Ok(...) with embedded code+message for business-logic
        // rejections (see cdcx-core/src/api_client.rs:183-193). Code 0 or missing = success.
        let code = data
            .get("data")
            .and_then(|d| d.get("code"))
            .and_then(|v| v.as_i64())
            .or_else(|| data.get("code").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        if code == 0 {
            state.toast(
                if self.isolated_margin {
                    "Order placed (isolated margin)".to_string()
                } else {
                    "Order placed".to_string()
                },
                crate::state::ToastStyle::Success,
            );
            return WorkflowResult::Done;
        }
        let message = data
            .get("data")
            .and_then(|d| d.get("message"))
            .or_else(|| data.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("Order rejected")
            .to_string();
        // Error 617 means an isolated position for this instrument already exists —
        // we need the isolation_id to add to / trim it. Kick off a positions refresh
        // so the retry path has it even if the WS snapshot hasn't landed yet.
        if code == 617 && !state.isolated_positions.contains_key(&self.instrument) {
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
        let modal = modal_area(area, 60, 20);
        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(state.theme.colors.accent))
            .title(" Place Order ");
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let lines = Layout::vertical([
            Constraint::Length(1), // spacing
            Constraint::Length(1), // instrument
            Constraint::Length(1), // side
            Constraint::Length(1), // type
            Constraint::Length(1), // margin
            Constraint::Length(1), // price
            Constraint::Length(1), // quantity
            Constraint::Length(1), // separator
            Constraint::Length(2), // summary / confirm / rejected
            Constraint::Length(2), // error / rejection-message
            Constraint::Length(1), // help
        ])
        .areas::<11>(inner);

        let active_style = Style::default()
            .fg(state.theme.colors.accent)
            .add_modifier(Modifier::BOLD);
        let dim_style = Style::default().fg(state.theme.colors.muted);

        // Instrument
        let inst_style = if self.step == Step::Instrument {
            active_style
        } else {
            dim_style
        };
        let inst_display = if self.step == Step::Instrument {
            format!("{}\u{2588}", self.instrument_input)
        } else {
            self.instrument.clone()
        };
        let inst_value_style = if self.step == Step::Instrument {
            active_style
        } else {
            Style::default()
                .fg(state.theme.colors.accent)
                .add_modifier(Modifier::BOLD)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Inst:   ", inst_style),
                Span::styled(inst_display, inst_value_style),
            ])),
            lines[1],
        );

        // Side
        let side_style = if self.step == Step::Side {
            active_style
        } else {
            dim_style
        };
        let buy_style = if self.side == 0 {
            Style::default()
                .fg(state.theme.colors.positive)
                .add_modifier(Modifier::BOLD)
        } else {
            dim_style
        };
        let sell_style = if self.side == 1 {
            Style::default()
                .fg(state.theme.colors.negative)
                .add_modifier(Modifier::BOLD)
        } else {
            dim_style
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Side:   ", side_style),
                Span::styled(if self.side == 0 { "[BUY]" } else { " BUY " }, buy_style),
                Span::raw("  "),
                Span::styled(if self.side == 1 { "[SELL]" } else { " SELL " }, sell_style),
            ])),
            lines[2],
        );

        // Type
        let type_style = if self.step == Step::OrderType {
            active_style
        } else {
            dim_style
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Type:   ", type_style),
                Span::styled(
                    if self.order_type == 0 {
                        "[LIMIT]"
                    } else {
                        " LIMIT "
                    },
                    if self.order_type == 0 {
                        active_style
                    } else {
                        dim_style
                    },
                ),
                Span::raw("  "),
                Span::styled(
                    if self.order_type == 1 {
                        "[MARKET]"
                    } else {
                        " MARKET "
                    },
                    if self.order_type == 1 {
                        active_style
                    } else {
                        dim_style
                    },
                ),
            ])),
            lines[3],
        );

        // Margin
        let margin_label_style = if self.step == Step::Margin {
            active_style
        } else {
            dim_style
        };
        let iso_label = if self.isolated_margin {
            "[ISOLATED]"
        } else {
            " ISOLATED "
        };
        let cross_label = if self.isolated_margin {
            " CROSS "
        } else {
            "[CROSS]"
        };
        let iso_style = if self.isolated_margin {
            Style::default()
                .fg(state.theme.colors.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            dim_style
        };
        let cross_style = if !self.isolated_margin {
            Style::default()
                .fg(state.theme.colors.fg)
                .add_modifier(Modifier::BOLD)
        } else {
            dim_style
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Margin: ", margin_label_style),
                Span::styled(iso_label, iso_style),
                Span::raw("  "),
                Span::styled(cross_label, cross_style),
                Span::styled("   (M to toggle)", dim_style),
            ])),
            lines[4],
        );

        // Price
        let price_style = if self.step == Step::Price {
            active_style
        } else {
            dim_style
        };
        let price_display = if self.order_type == 1 {
            "MARKET".to_string()
        } else if self.price_input.is_empty() && self.step == Step::Price {
            "\u{2588}".to_string()
        } else if self.step == Step::Price {
            format!("{}\u{2588}", self.price_input)
        } else {
            self.price_input.clone()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Price:  ", price_style),
                Span::styled(
                    price_display,
                    if self.step == Step::Price {
                        active_style
                    } else {
                        dim_style
                    },
                ),
            ])),
            lines[5],
        );

        // Quantity
        let qty_style = if self.step == Step::Quantity {
            active_style
        } else {
            dim_style
        };
        let qty_display = if self.qty_input.is_empty() && self.step == Step::Quantity {
            "\u{2588}".to_string()
        } else if self.step == Step::Quantity {
            format!("{}\u{2588}", self.qty_input)
        } else {
            self.qty_input.clone()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Qty:    ", qty_style),
                Span::styled(
                    qty_display,
                    if self.step == Step::Quantity {
                        active_style
                    } else {
                        dim_style
                    },
                ),
            ])),
            lines[6],
        );

        // Separator
        frame.render_widget(
            Paragraph::new("\u{2500}".repeat(inner.width as usize))
                .style(Style::default().fg(state.theme.colors.border)),
            lines[7],
        );

        // Confirm / status / rejected
        match self.step {
            Step::Confirm => {
                let margin_tag = if self.isolated_margin {
                    "  [ISOLATED]"
                } else {
                    ""
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!(
                            "{} {} {} @ {}{}",
                            self.side_str(),
                            self.qty_input,
                            self.instrument,
                            if self.order_type == 0 {
                                &self.price_input
                            } else {
                                "MARKET"
                            },
                            margin_tag,
                        ),
                        Style::default()
                            .fg(state.theme.colors.accent)
                            .add_modifier(Modifier::BOLD),
                    )])),
                    lines[8],
                );
            }
            Step::Submitting => {
                frame.render_widget(
                    Paragraph::new("Submitting order...")
                        .style(Style::default().fg(state.theme.colors.accent)),
                    lines[8],
                );
            }
            Step::Rejected => {
                frame.render_widget(
                    Paragraph::new("Order rejected by exchange")
                        .style(Style::default().fg(state.theme.colors.negative)),
                    lines[8],
                );
            }
            _ => {}
        }

        // Error / rejection detail
        if let Some((code, ref message)) = self.rejection {
            let has_isolation_id = state.isolated_positions.contains_key(&self.instrument);
            let hint = match code {
                623 if !self.isolated_margin => {
                    "  (press M then R to retry with isolated margin)".to_string()
                }
                617 if has_isolation_id => {
                    "  (R to retry — will attach your open isolation_id)".to_string()
                }
                617 => "  (waiting on positions stream; check `cdcx account positions` then R)"
                    .to_string(),
                _ => String::new(),
            };
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("[{}] {}", code, message),
                        Style::default().fg(state.theme.colors.negative),
                    ),
                    Span::styled(hint, Style::default().fg(state.theme.colors.muted)),
                ])),
                lines[9],
            );
        } else if let Some(ref err) = self.error {
            frame.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(state.theme.colors.negative)),
                lines[9],
            );
        }

        // Help
        let help = match self.step {
            Step::Instrument => "type instrument  Tab:next  Esc:cancel",
            Step::Side | Step::OrderType => {
                "\u{2190}\u{2192}:select  Tab:next  Shift+Tab:back  M:margin  Esc:cancel"
            }
            Step::Margin => "\u{2190}\u{2192}/Space:toggle  Tab:next  Shift+Tab:back  Esc:cancel",
            Step::Price | Step::Quantity => "type value  Tab:next  Shift+Tab:back  Esc:cancel",
            Step::Confirm => "Enter:submit  M:toggle margin  Shift+Tab:back  Esc:cancel",
            Step::Submitting => "waiting...",
            Step::Rejected => "R/Enter:retry  M:toggle margin  E:edit  Esc:cancel",
        };
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(state.theme.colors.muted)),
            lines[10],
        );
    }
}
