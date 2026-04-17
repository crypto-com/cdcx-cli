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
    Confirm,
    Submitting,
}

pub struct CancelOrderWorkflow {
    instrument: String,
    step: Step,
}

impl CancelOrderWorkflow {
    pub fn new(instrument: String) -> Self {
        Self {
            instrument,
            step: Step::Confirm,
        }
    }
}

impl Workflow for CancelOrderWorkflow {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> WorkflowResult {
        if key.code == KeyCode::Esc {
            return WorkflowResult::Cancel;
        }

        match self.step {
            Step::Confirm => {
                if key.code == KeyCode::Enter || key.code == KeyCode::Char('y') {
                    self.step = Step::Submitting;
                    let _ = state.rest_tx.send(RestRequest {
                        method: "private/cancel-all-orders".into(),
                        params: serde_json::json!({
                            "instrument_name": self.instrument,
                        }),
                        is_private: true,
                    });
                }
            }
            Step::Submitting => {
                // App.on_data() will close the workflow when response arrives
            }
        }
        WorkflowResult::Continue
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        let modal = modal_area(area, 46, 8);
        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(state.theme.colors.negative))
            .title(" Cancel Orders ");
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let lines = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas::<5>(inner);

        match self.step {
            Step::Confirm => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(
                            "Cancel all orders for ",
                            Style::default().fg(state.theme.colors.fg),
                        ),
                        Span::styled(
                            self.instrument.as_str(),
                            Style::default()
                                .fg(state.theme.colors.accent)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("?", Style::default().fg(state.theme.colors.fg)),
                    ])),
                    lines[1],
                );
                frame.render_widget(
                    Paragraph::new("Enter:confirm  Esc:cancel")
                        .style(Style::default().fg(state.theme.colors.muted)),
                    lines[3],
                );
            }
            Step::Submitting => {
                frame.render_widget(
                    Paragraph::new("Cancelling orders...")
                        .style(Style::default().fg(state.theme.colors.accent)),
                    lines[1],
                );
            }
        }
    }
}
