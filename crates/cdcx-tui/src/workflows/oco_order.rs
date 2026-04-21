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
    StopPrice,
    TakeProfitPrice,
    Quantity,
    Confirm,
    Submitting,
}

/// OCO: One-Cancels-the-Other — stop-loss + take-profit
pub struct OcoOrderWorkflow {
    instrument: String,
    instrument_input: String,
    step: Step,
    stop_price: String,
    tp_price: String,
    quantity: String,
    error: Option<String>,
}

impl OcoOrderWorkflow {
    pub fn new(instrument: String) -> Self {
        let input = instrument.clone();
        Self {
            instrument,
            instrument_input: input,
            step: Step::Instrument,
            stop_price: String::new(),
            tp_price: String::new(),
            quantity: String::new(),
            error: None,
        }
    }

    fn submit(&self, state: &AppState) {
        let mut params = serde_json::json!({
            "contingency_type": "OCO",
            "order_list": [
                {
                    "instrument_name": self.instrument,
                    "side": "SELL",
                    "type": "STOP_LOSS",
                    "quantity": self.quantity,
                    "trigger_price": self.stop_price,
                },
                {
                    "instrument_name": self.instrument,
                    "side": "SELL",
                    "type": "TAKE_PROFIT",
                    "quantity": self.quantity,
                    "trigger_price": self.tp_price,
                }
            ]
        });
        // Stamp cx3- TUI origin prefix on each leg's client_oid.
        cdcx_core::origin::tag_order_list_legs(&mut params, cdcx_core::origin::OriginChannel::Tui);
        let _ = state.rest_tx.send(RestRequest {
            method: "private/create-order-list".into(),
            params,
            is_private: true,
        });
    }
}

impl Workflow for OcoOrderWorkflow {
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
                        self.step = Step::StopPrice;
                    }
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
                    self.step = Step::Instrument;
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
        let modal = modal_area(area, 54, 16);
        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(state.theme.colors.accent))
            .title(" OCO Order (Stop-Loss + Take-Profit) ");
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let lines = Layout::vertical([
            Constraint::Length(1), // space
            Constraint::Length(1), // instrument
            Constraint::Length(1), // stop
            Constraint::Length(1), // tp
            Constraint::Length(1), // qty
            Constraint::Length(1), // space
            Constraint::Length(1), // separator
            Constraint::Length(2), // confirm
            Constraint::Length(1), // error
            Constraint::Length(1), // help
        ])
        .areas::<10>(inner);

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
                Span::styled(format!("{:<12}", label), s),
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
        frame.render_widget(
            Paragraph::new(field(
                "Stop-Loss:",
                &self.stop_price,
                Step::StopPrice,
                self.step,
            )),
            lines[2],
        );
        frame.render_widget(
            Paragraph::new(field(
                "Take-Profit:",
                &self.tp_price,
                Step::TakeProfitPrice,
                self.step,
            )),
            lines[3],
        );
        frame.render_widget(
            Paragraph::new(field(
                "Quantity:",
                &self.quantity,
                Step::Quantity,
                self.step,
            )),
            lines[4],
        );

        frame.render_widget(
            Paragraph::new("\u{2500}".repeat(inner.width as usize))
                .style(Style::default().fg(state.theme.colors.border)),
            lines[6],
        );

        match self.step {
            Step::Confirm => {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(
                            "OCO {} qty {} SL@{} TP@{}",
                            self.instrument, self.quantity, self.stop_price, self.tp_price
                        ),
                        active,
                    ))),
                    lines[7],
                );
            }
            Step::Submitting => {
                frame.render_widget(
                    Paragraph::new("Submitting OCO...")
                        .style(Style::default().fg(state.theme.colors.accent)),
                    lines[7],
                );
            }
            _ => {}
        }

        if let Some(ref err) = self.error {
            frame.render_widget(
                Paragraph::new(err.as_str())
                    .style(Style::default().fg(state.theme.colors.negative)),
                lines[8],
            );
        }

        let help = match self.step {
            Step::Instrument => "type instrument  Tab:next  Esc:cancel",
            Step::StopPrice | Step::TakeProfitPrice | Step::Quantity => {
                "type value  Tab:next  Shift+Tab:back  Esc:cancel"
            }
            Step::Confirm => "Enter:submit  Shift+Tab:back  Esc:cancel",
            Step::Submitting => "waiting...",
        };
        frame.render_widget(Paragraph::new(help).style(dim), lines[9]);
    }
}
