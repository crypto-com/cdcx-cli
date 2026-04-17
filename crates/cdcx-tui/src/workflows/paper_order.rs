use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::AppState;
use crate::workflows::{modal_area, Workflow, WorkflowResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Side,
    OrderType,
    Price,
    Quantity,
    Confirm,
    Executing,
    Result,
}

/// Paper order workflow — executes against PaperEngine, no REST calls.
pub struct PaperOrderWorkflow {
    instrument: String,
    step: Step,
    side: usize,       // 0 = BUY, 1 = SELL
    order_type: usize, // 0 = MARKET, 1 = LIMIT
    price_input: String,
    qty_input: String,
    result_msg: Option<String>,
    result_ok: bool,
    error: Option<String>,
}

impl PaperOrderWorkflow {
    pub fn new(instrument: String) -> Self {
        Self {
            instrument,
            step: Step::Side,
            side: 0,
            order_type: 0,
            price_input: String::new(),
            qty_input: String::new(),
            result_msg: None,
            result_ok: false,
            error: None,
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
            "MARKET"
        } else {
            "LIMIT"
        }
    }
}

impl Workflow for PaperOrderWorkflow {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> WorkflowResult {
        if key.code == KeyCode::Esc {
            return WorkflowResult::Cancel;
        }

        match self.step {
            Step::Side => match key.code {
                KeyCode::Left | KeyCode::Right => {
                    self.side = 1 - self.side;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.step = Step::OrderType;
                }
                _ => {}
            },
            Step::OrderType => match key.code {
                KeyCode::Left | KeyCode::Right => {
                    self.order_type = 1 - self.order_type;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    self.step = if self.order_type == 1 {
                        Step::Price
                    } else {
                        Step::Quantity
                    };
                }
                KeyCode::BackTab => {
                    self.step = Step::Side;
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
                        self.error = Some("Price required for limit orders".into());
                    } else {
                        self.error = None;
                        self.step = Step::Quantity;
                    }
                }
                KeyCode::BackTab => {
                    self.step = Step::OrderType;
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
                        self.error = Some("Quantity required".into());
                    } else {
                        self.error = None;
                        self.step = Step::Confirm;
                    }
                }
                KeyCode::BackTab => {
                    self.step = if self.order_type == 1 {
                        Step::Price
                    } else {
                        Step::OrderType
                    };
                }
                _ => {}
            },
            Step::Confirm => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.step = Step::Executing;
                    // Execute against PaperEngine directly
                    self.execute_paper_order(state);
                    self.step = Step::Result;
                }
                KeyCode::BackTab => {
                    self.step = Step::Quantity;
                }
                _ => {}
            },
            Step::Executing => {}
            Step::Result => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    return WorkflowResult::Done;
                }
                _ => {}
            },
        }
        WorkflowResult::Continue
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        let modal = modal_area(area, 54, 16);
        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(state.theme.colors.accent))
            .title(format!(" Paper Order \u{2014} {} ", self.instrument));
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let lines = Layout::vertical([
            Constraint::Length(1), // space
            Constraint::Length(1), // side
            Constraint::Length(1), // type
            Constraint::Length(1), // price
            Constraint::Length(1), // qty
            Constraint::Length(1), // space
            Constraint::Length(1), // separator
            Constraint::Length(2), // result/confirm
            Constraint::Length(1), // error
            Constraint::Length(1), // help
        ])
        .areas::<10>(inner);

        let active = Style::default()
            .fg(state.theme.colors.accent)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(state.theme.colors.muted);

        // Side
        let side_s = if self.step == Step::Side { active } else { dim };
        let buy_s = if self.side == 0 {
            Style::default()
                .fg(state.theme.colors.positive)
                .add_modifier(Modifier::BOLD)
        } else {
            dim
        };
        let sell_s = if self.side == 1 {
            Style::default()
                .fg(state.theme.colors.negative)
                .add_modifier(Modifier::BOLD)
        } else {
            dim
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Side:  ", side_s),
                Span::styled(if self.side == 0 { "[BUY]" } else { " BUY " }, buy_s),
                Span::raw("  "),
                Span::styled(if self.side == 1 { "[SELL]" } else { " SELL " }, sell_s),
            ])),
            lines[1],
        );

        // Type
        let type_s = if self.step == Step::OrderType {
            active
        } else {
            dim
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Type:  ", type_s),
                Span::styled(
                    if self.order_type == 0 {
                        "[MARKET]"
                    } else {
                        " MARKET "
                    },
                    if self.order_type == 0 { active } else { dim },
                ),
                Span::raw("  "),
                Span::styled(
                    if self.order_type == 1 {
                        "[LIMIT]"
                    } else {
                        " LIMIT "
                    },
                    if self.order_type == 1 { active } else { dim },
                ),
            ])),
            lines[2],
        );

        // Price
        let price_s = if self.step == Step::Price {
            active
        } else {
            dim
        };
        let price_val = if self.order_type == 0 {
            "MARKET".to_string()
        } else if self.step == Step::Price {
            format!("{}\u{2588}", self.price_input)
        } else {
            self.price_input.clone()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Price: ", price_s),
                Span::styled(
                    price_val,
                    if self.step == Step::Price {
                        active
                    } else {
                        dim
                    },
                ),
            ])),
            lines[3],
        );

        // Qty
        let qty_s = if self.step == Step::Quantity {
            active
        } else {
            dim
        };
        let qty_val = if self.step == Step::Quantity {
            format!("{}\u{2588}", self.qty_input)
        } else {
            self.qty_input.clone()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Qty:   ", qty_s),
                Span::styled(
                    qty_val,
                    if self.step == Step::Quantity {
                        active
                    } else {
                        dim
                    },
                ),
            ])),
            lines[4],
        );

        // Separator
        frame.render_widget(
            Paragraph::new("\u{2500}".repeat(inner.width as usize))
                .style(Style::default().fg(state.theme.colors.border)),
            lines[6],
        );

        // Result / confirm
        match self.step {
            Step::Confirm => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(
                            "PAPER {} {} {} @ {}",
                            self.side_str(),
                            self.qty_input,
                            self.instrument,
                            self.type_str()
                        ),
                        active,
                    ))),
                    lines[7],
                );
            }
            Step::Result => {
                if let Some(ref msg) = self.result_msg {
                    let color = if self.result_ok {
                        state.theme.colors.positive
                    } else {
                        state.theme.colors.negative
                    };
                    frame.render_widget(
                        Paragraph::new(msg.as_str()).style(Style::default().fg(color)),
                        lines[7],
                    );
                }
            }
            _ => {}
        }

        // Error
        if let Some(ref err) = self.error {
            frame.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(state.theme.colors.negative)),
                lines[8],
            );
        }

        // Help
        let help = match self.step {
            Step::Side | Step::OrderType => "\u{2190}\u{2192}:select  Tab:next  Esc:cancel",
            Step::Price | Step::Quantity => "type value  Tab:next  Shift+Tab:back  Esc:cancel",
            Step::Confirm => "Enter:execute  Shift+Tab:back  Esc:cancel",
            Step::Executing => "executing...",
            Step::Result => "Enter:close",
        };
        frame.render_widget(Paragraph::new(help).style(dim), lines[9]);
    }
}

impl PaperOrderWorkflow {
    fn execute_paper_order(&mut self, state: &mut AppState) {
        use cdcx_core::paper::types::OrderSide;

        let qty: f64 = match self.qty_input.parse() {
            Ok(q) => q,
            Err(_) => {
                self.result_msg = Some("Invalid quantity".into());
                self.result_ok = false;
                return;
            }
        };

        let price: Option<f64> = if self.order_type == 1 {
            match self.price_input.parse() {
                Ok(p) => Some(p),
                Err(_) => {
                    self.result_msg = Some("Invalid price".into());
                    self.result_ok = false;
                    return;
                }
            }
        } else {
            None
        };

        let side = if self.side == 0 {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };

        let Some(ref mut engine) = state.paper_engine else {
            self.result_msg = Some("Paper engine not initialized".into());
            self.result_ok = false;
            return;
        };

        if self.order_type == 0 {
            // Market order — get fill price from live ticker data
            let ticker = state.tickers.get(&self.instrument);
            let fill_price = match (ticker, side) {
                (Some(t), OrderSide::Buy) if t.ask > 0.0 => t.ask,
                (Some(t), OrderSide::Sell) if t.bid > 0.0 => t.bid,
                (Some(t), _) if t.ask > 0.0 => t.ask,
                _ => {
                    self.result_msg = Some(format!("No price data for {}", self.instrument));
                    self.result_ok = false;
                    return;
                }
            };

            match engine.execute_market_order_at_price(&self.instrument, side, qty, fill_price) {
                Ok(trade) => {
                    self.result_msg = Some(format!(
                        "FILLED: {} {} {} @ {:.2} (fee: {:.4})\nBalance: ${:.2}",
                        self.side_str(),
                        qty,
                        self.instrument,
                        trade.price,
                        trade.fee,
                        engine.state.balance
                    ));
                    self.result_ok = true;
                }
                Err(e) => {
                    self.result_msg = Some(e.to_string());
                    self.result_ok = false;
                }
            }
        } else {
            // Limit order
            let limit_price = price.unwrap_or(0.0);
            match engine.place_limit_order(&self.instrument, side, qty, limit_price) {
                Ok(order) => {
                    self.result_msg = Some(format!(
                        "LIMIT ORDER #{}: {} {} {} @ {:.2}",
                        order.order_id,
                        self.side_str(),
                        qty,
                        self.instrument,
                        limit_price
                    ));
                    self.result_ok = true;
                }
                Err(e) => {
                    self.result_msg = Some(e.to_string());
                    self.result_ok = false;
                }
            }
        }
    }
}
