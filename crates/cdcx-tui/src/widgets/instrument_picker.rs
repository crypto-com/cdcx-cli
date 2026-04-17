use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::theme::ThemeColors;

/// Search-as-you-type instrument picker overlay.
pub struct InstrumentPicker {
    pub query: String,
    pub selected: usize,
    pub scroll_offset: usize,
    filtered: Vec<String>,
    all_instruments: Vec<String>,
}

pub enum PickerResult {
    Continue,
    Selected(String),
    Cancelled,
}

impl InstrumentPicker {
    pub fn new(instruments: &[String]) -> Self {
        let all = instruments.to_vec();
        let filtered = all.clone();
        Self {
            query: String::new(),
            selected: 0,
            scroll_offset: 0,
            filtered,
            all_instruments: all,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PickerResult {
        match key.code {
            KeyCode::Esc => PickerResult::Cancelled,
            KeyCode::Enter => {
                if let Some(inst) = self.filtered.get(self.selected) {
                    PickerResult::Selected(inst.clone())
                } else {
                    PickerResult::Cancelled
                }
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    if self.selected < self.scroll_offset {
                        self.scroll_offset = self.selected;
                    }
                }
                PickerResult::Continue
            }
            KeyCode::Down => {
                let max = self.filtered.len().saturating_sub(1);
                if self.selected < max {
                    self.selected += 1;
                    // visible height handled in draw, assume ~15 rows
                    if self.selected >= self.scroll_offset + 15 {
                        self.scroll_offset = self.selected.saturating_sub(14);
                    }
                }
                PickerResult::Continue
            }
            KeyCode::Char(c) => {
                self.query.push(c.to_ascii_uppercase());
                self.refilter();
                PickerResult::Continue
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
                PickerResult::Continue
            }
            _ => PickerResult::Continue,
        }
    }

    fn refilter(&mut self) {
        if self.query.is_empty() {
            self.filtered = self.all_instruments.clone();
        } else {
            self.filtered = self
                .all_instruments
                .iter()
                .filter(|i| i.contains(&self.query))
                .cloned()
                .collect();
        }
        self.selected = 0;
        self.scroll_offset = 0;
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        // Centered modal
        let width = 40u16.min(area.width.saturating_sub(4));
        let height = 20u16.min(area.height.saturating_sub(4));
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let modal = Rect::new(x, y, width, height);

        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.accent))
            .title(" Select Instrument ");
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let [search_area, count_area, list_area, help_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(inner);

        // Search input
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" / ", Style::default().fg(colors.accent)),
                Span::styled(
                    format!("{}\u{2588}", self.query),
                    Style::default().fg(colors.fg),
                ),
            ])),
            search_area,
        );

        // Count
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {} matches", self.filtered.len()),
                Style::default().fg(colors.muted),
            ))),
            count_area,
        );

        // Instrument list
        let visible_height = list_area.height as usize;
        let end = (self.scroll_offset + visible_height).min(self.filtered.len());
        let visible = &self.filtered[self.scroll_offset..end];

        let lines: Vec<Line> = visible
            .iter()
            .enumerate()
            .map(|(vi, inst)| {
                let abs_idx = self.scroll_offset + vi;
                let is_selected = abs_idx == self.selected;

                if is_selected {
                    Line::from(Span::styled(
                        format!(" > {}", inst),
                        Style::default()
                            .fg(colors.selected_fg)
                            .bg(colors.selected_bg)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("   {}", inst),
                        Style::default().fg(colors.fg),
                    ))
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), list_area);

        // Help
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " type:filter  \u{2191}\u{2193}:select  Enter:pick  Esc:cancel",
                Style::default().fg(colors.muted),
            ))),
            help_area,
        );
    }
}
