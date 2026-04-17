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
    Side,       // BUY or SELL for the entry
    EntryPrice, // Limit entry price
    StopPrice,
    TakeProfitPrice,
    Quantity,
    Confirm,
    Submitting,
}

/// OTOCO: Order-Triggers-OCO — entry order that triggers stop-loss + take-profit
pub struct OtocoOrderWorkflow {
    instrument: String,
    instrument_input: String,
    step: Step,
    side: usize, // 0 = BUY, 1 = SELL
    entry_price: String,
    stop_price: String,
    tp_price: String,
    quantity: String,
    error: Option<String>,
}

impl OtocoOrderWorkflow {
    pub fn new(instrument: String) -> Self {
        let input = instrument.clone();
        Self {
            instrument,
            instrument_input: input,
            step: Step::Instrument,
            side: 0,
            entry_price: String::new(),
            stop_price: String::new(),
            tp_price: String::new(),
            quantity: String::new(),
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
    fn exit_side(&self) -> &'static str {
        if self.side == 0 {
            "SELL"
        } else {
            "BUY"
        }
    }

    fn submit(&self, state: &AppState) {
        let params = serde_json::json!({
            "contingency_type": "OTOCO",
            "order_list": [
                {
                    "instrument_name": self.instrument,
                    "side": self.side_str(),
                    "type": "LIMIT",
                    "price": self.entry_price,
                    "quantity": self.quantity,
                },
                {
                    "instrument_name": self.instrument,
                    "side": self.exit_side(),
                    "type": "STOP_LOSS",
                    "quantity": self.quantity,
                    "trigger_price": self.stop_price,
                },
                {
                    "instrument_name": self.instrument,
                    "side": self.exit_side(),
                    "type": "TAKE_PROFIT",
                    "quantity": self.quantity,
                    "trigger_price": self.tp_price,
                }
            ]
        });
        let _ = state.rest_tx.send(RestRequest {
            method: "private/create-order-list".into(),
            params,
            is_private: true,
        });
    }
}

impl Workflow for OtocoOrderWorkflow {
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
                        self.error = Some("Instrument required".into());
                    } else if !state.instruments.contains(&trimmed) {
                        self.error = Some(format!("Unknown: {}", trimmed));
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
                    self.step = Step::EntryPrice;
                }
                KeyCode::BackTab => {
                    self.step = Step::Instrument;
                }
                _ => {}
            },
            Step::EntryPrice => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                    self.entry_price.push(c);
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.entry_price.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if self.entry_price.is_empty() {
                        self.error = Some("Entry price required".into());
                    } else {
                        self.error = None;
                        self.step = Step::StopPrice;
                    }
                }
                KeyCode::BackTab => {
                    self.step = Step::Side;
                }
                _ => {}
            },
            Step::StopPrice => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                    self.stop_price.push(c);
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.stop_price.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if self.stop_price.is_empty() {
                        self.error = Some("Stop price required".into());
                    } else {
                        self.error = None;
                        self.step = Step::TakeProfitPrice;
                    }
                }
                KeyCode::BackTab => {
                    self.step = Step::EntryPrice;
                }
                _ => {}
            },
            Step::TakeProfitPrice => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                    self.tp_price.push(c);
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.tp_price.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if self.tp_price.is_empty() {
                        self.error = Some("Take-profit price required".into());
                    } else {
                        self.error = None;
                        self.step = Step::Quantity;
                    }
                }
                KeyCode::BackTab => {
                    self.step = Step::StopPrice;
                }
                _ => {}
            },
            Step::Quantity => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' => {
                    self.quantity.push(c);
                    self.error = None;
                }
                KeyCode::Backspace => {
                    self.quantity.pop();
                }
                KeyCode::Enter | KeyCode::Tab => {
                    if self.quantity.is_empty() {
                        self.error = Some("Quantity required".into());
                    } else {
                        self.error = None;
                        self.step = Step::Confirm;
                    }
                }
                KeyCode::BackTab => {
                    self.step = Step::TakeProfitPrice;
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
            Step::Submitting => {}
        }
        WorkflowResult::Continue
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        let modal = modal_area(area, 58, 18);
        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(state.theme.colors.accent))
            .title(" OTOCO Order (Entry + Stop-Loss + Take-Profit) ");
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let lines = Layout::vertical([
            Constraint::Length(1), // space
            Constraint::Length(1), // instrument
            Constraint::Length(1), // side
            Constraint::Length(1), // entry
            Constraint::Length(1), // stop
            Constraint::Length(1), // tp
            Constraint::Length(1), // qty
            Constraint::Length(1), // separator
            Constraint::Length(2), // confirm
            Constraint::Length(1), // error
            Constraint::Length(1), // help
        ])
        .areas::<11>(inner);

        let active = Style::default()
            .fg(state.theme.colors.accent)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(state.theme.colors.muted);

        let field = |label: &str, value: &str, step: Step, current: Step| -> Line {
            let s = if step == current { active } else { dim };
            let v = if step == current {
                format!("{}\u{2588}", value)
            } else {
                value.to_string()
            };
            Line::from(vec![
                Span::styled(format!("{:<14}", label), s),
                Span::styled(
                    v,
                    if step == current {
                        active
                    } else {
                        Style::default().fg(state.theme.colors.fg)
                    },
                ),
            ])
        };

        frame.render_widget(
            Paragraph::new(field(
                "Instrument:",
                &self.instrument_input,
                Step::Instrument,
                self.step,
            )),
            lines[1],
        );

        // Side toggle
        let side_style = if self.step == Step::Side { active } else { dim };
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
                Span::styled("Side:         ", side_style),
                Span::styled(if self.side == 0 { "[BUY]" } else { " BUY " }, buy_s),
                Span::raw("  "),
                Span::styled(if self.side == 1 { "[SELL]" } else { " SELL " }, sell_s),
            ])),
            lines[2],
        );

        frame.render_widget(
            Paragraph::new(field(
                "Entry Price:",
                &self.entry_price,
                Step::EntryPrice,
                self.step,
            )),
            lines[3],
        );
        frame.render_widget(
            Paragraph::new(field(
                "Stop-Loss:",
                &self.stop_price,
                Step::StopPrice,
                self.step,
            )),
            lines[4],
        );
        frame.render_widget(
            Paragraph::new(field(
                "Take-Profit:",
                &self.tp_price,
                Step::TakeProfitPrice,
                self.step,
            )),
            lines[5],
        );
        frame.render_widget(
            Paragraph::new(field(
                "Quantity:",
                &self.quantity,
                Step::Quantity,
                self.step,
            )),
            lines[6],
        );

        frame.render_widget(
            Paragraph::new("\u{2500}".repeat(inner.width as usize))
                .style(Style::default().fg(state.theme.colors.border)),
            lines[7],
        );

        match self.step {
            Step::Confirm => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(
                            "OTOCO {} {} qty {} entry@{} SL@{} TP@{}",
                            self.side_str(),
                            self.instrument,
                            self.quantity,
                            self.entry_price,
                            self.stop_price,
                            self.tp_price
                        ),
                        active,
                    ))),
                    lines[8],
                );
            }
            Step::Submitting => {
                frame.render_widget(
                    Paragraph::new("Submitting OTOCO...")
                        .style(Style::default().fg(state.theme.colors.accent)),
                    lines[8],
                );
            }
            _ => {}
        }

        if let Some(ref err) = self.error {
            frame.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(state.theme.colors.negative)),
                lines[9],
            );
        }

        let help = match self.step {
            Step::Instrument => "type instrument  Tab:next  Esc:cancel",
            Step::Side => "\u{2190}\u{2192}:select  Tab:next  Shift+Tab:back  Esc:cancel",
            Step::EntryPrice | Step::StopPrice | Step::TakeProfitPrice | Step::Quantity => {
                "type value  Tab:next  Shift+Tab:back  Esc:cancel"
            }
            Step::Confirm => "Enter:submit  Shift+Tab:back  Esc:cancel",
            Step::Submitting => "waiting...",
        };
        frame.render_widget(Paragraph::new(help).style(dim), lines[10]);
    }
}
