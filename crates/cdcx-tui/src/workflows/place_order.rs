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
    Price,
    Quantity,
    Confirm,
    Submitting,
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
}

impl PlaceOrderWorkflow {
    pub fn new(instrument: String) -> Self {
        let instrument_input = instrument.clone();
        Self {
            instrument,
            instrument_input,
            step: Step::Instrument,
            side: 0,
            order_type: 0,
            price_input: String::new(),
            qty_input: String::new(),
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
                    self.step = if self.order_type == 0 {
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
                        Step::OrderType
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
                // App.on_data() closes the workflow when response arrives
            }
        }
        WorkflowResult::Continue
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        let modal = modal_area(area, 54, 18);
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
            Constraint::Length(1), // price
            Constraint::Length(1), // quantity
            Constraint::Length(1), // spacing
            Constraint::Length(1), // separator
            Constraint::Length(2), // summary / confirm
            Constraint::Length(1), // error
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
                Span::styled("Inst:  ", inst_style),
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
                Span::styled("Side:  ", side_style),
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
                Span::styled("Type:  ", type_style),
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
                Span::styled("Price: ", price_style),
                Span::styled(
                    price_display,
                    if self.step == Step::Price {
                        active_style
                    } else {
                        dim_style
                    },
                ),
            ])),
            lines[4],
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
                Span::styled("Qty:   ", qty_style),
                Span::styled(
                    qty_display,
                    if self.step == Step::Quantity {
                        active_style
                    } else {
                        dim_style
                    },
                ),
            ])),
            lines[5],
        );

        // Separator
        frame.render_widget(
            Paragraph::new("\u{2500}".repeat(inner.width as usize))
                .style(Style::default().fg(state.theme.colors.border)),
            lines[7],
        );

        // Confirm / status
        match self.step {
            Step::Confirm => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!(
                            "{} {} {} @ {}",
                            self.side_str(),
                            self.qty_input,
                            self.instrument,
                            if self.order_type == 0 {
                                &self.price_input
                            } else {
                                "MARKET"
                            }
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
            _ => {}
        }

        // Error
        if let Some(ref err) = self.error {
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
                "\u{2190}\u{2192}:select  Tab:next  Shift+Tab:back  Esc:cancel"
            }
            Step::Price | Step::Quantity => "type value  Tab:next  Shift+Tab:back  Esc:cancel",
            Step::Confirm => "Enter:submit  Shift+Tab:back  Esc:cancel",
            Step::Submitting => "waiting...",
        };
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(state.theme.colors.muted)),
            lines[10],
        );
    }
}
