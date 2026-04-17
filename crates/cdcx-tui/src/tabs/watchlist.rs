use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::state::AppState;
use crate::tabs::{DataEvent, Tab, TabKind};
use crate::widgets::instrument_picker::{InstrumentPicker, PickerResult};

pub struct WatchlistTab {
    instruments: Vec<String>,
    selected: usize,
    picker: Option<InstrumentPicker>,
}

impl WatchlistTab {
    pub fn new(_state: &AppState, watchlist: &[String]) -> Self {
        let instruments = if watchlist.is_empty() {
            vec![
                "BTC_USDT".into(),
                "ETH_USDT".into(),
                "SOL_USDT".into(),
                "CRO_USDT".into(),
            ]
        } else {
            watchlist.to_vec()
        };
        Self {
            instruments,
            selected: 0,
            picker: None,
        }
    }
}

impl Tab for WatchlistTab {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool {
        // Picker takes priority
        if let Some(ref mut picker) = self.picker {
            match picker.on_key(key) {
                PickerResult::Selected(inst) => {
                    if !self.instruments.contains(&inst) {
                        self.instruments.push(inst);
                    }
                    self.picker = None;
                }
                PickerResult::Cancelled => {
                    self.picker = None;
                }
                PickerResult::Continue => {}
            }
            return true;
        }

        match key.code {
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                true
            }
            KeyCode::Down => {
                if self.selected < self.instruments.len().saturating_sub(1) {
                    self.selected += 1;
                }
                true
            }
            KeyCode::Char('a') => {
                self.picker = Some(InstrumentPicker::new(&state.instruments));
                true
            }
            KeyCode::Enter => {
                if let Some(inst) = self.instruments.get(self.selected) {
                    state.pending_navigation = Some((TabKind::Market, inst.clone()));
                }
                true
            }
            KeyCode::Char('d') => {
                if !self.instruments.is_empty() {
                    self.instruments.remove(self.selected);
                    if self.selected >= self.instruments.len() && self.selected > 0 {
                        self.selected -= 1;
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn on_data(&mut self, _event: &DataEvent, _state: &mut AppState) {}

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        if self.instruments.is_empty() && self.picker.is_none() {
            frame.render_widget(
                Paragraph::new("No instruments in watchlist. Press a to add.")
                    .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        let [table_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        let header = Row::new(vec![
            Cell::from("Instrument"),
            Cell::from("Price"),
            Cell::from("24h"),
            Cell::from("High"),
            Cell::from("Low"),
            Cell::from("Volume"),
        ])
        .style(
            Style::default()
                .fg(state.theme.colors.header)
                .add_modifier(Modifier::BOLD),
        );

        let widths = [
            Constraint::Length(18),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(12),
        ];

        let rows: Vec<Row> = self
            .instruments
            .iter()
            .enumerate()
            .map(|(i, inst)| {
                let is_selected = i == self.selected;
                let ticker = state.tickers.get(inst);

                let row_style = if is_selected {
                    Style::default()
                        .fg(state.theme.colors.selected_fg)
                        .bg(state.theme.colors.selected_bg)
                } else {
                    Style::default().fg(state.theme.colors.fg)
                };

                if let Some(t) = ticker {
                    let change_color = if t.change_pct >= 0.0 {
                        state.theme.colors.positive
                    } else {
                        state.theme.colors.negative
                    };
                    Row::new(vec![
                        Cell::from(inst.as_str()),
                        Cell::from(format!("{:.2}", t.ask)),
                        Cell::from(format!("{:+.2}%", t.change_pct * 100.0))
                            .style(Style::default().fg(change_color)),
                        Cell::from(format!("{:.2}", t.high))
                            .style(Style::default().fg(state.theme.colors.muted)),
                        Cell::from(format!("{:.2}", t.low))
                            .style(Style::default().fg(state.theme.colors.muted)),
                        Cell::from(format!("{:.0}", t.volume))
                            .style(Style::default().fg(state.theme.colors.volume)),
                    ])
                    .style(row_style)
                } else {
                    Row::new(vec![
                        Cell::from(inst.as_str()),
                        Cell::from("\u{2026}"),
                        Cell::from(""),
                        Cell::from(""),
                        Cell::from(""),
                        Cell::from(""),
                    ])
                    .style(row_style)
                }
            })
            .collect();

        let table = Table::new(rows, widths).header(header).column_spacing(1);
        frame.render_widget(table, table_area);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "a:add  d:remove  \u{2191}\u{2193}:navigate  Enter:details",
                Style::default().fg(state.theme.colors.muted),
            ))),
            footer_area,
        );

        // Picker overlay
        if let Some(ref picker) = self.picker {
            picker.draw(frame, area, &state.theme.colors);
        }
    }

    fn subscriptions(&self, _state: &AppState) -> Vec<String> {
        self.instruments
            .iter()
            .map(|i| format!("ticker.{}", i))
            .collect()
    }

    fn selected_instrument(&self) -> Option<&str> {
        self.instruments.get(self.selected).map(|s| s.as_str())
    }
}
