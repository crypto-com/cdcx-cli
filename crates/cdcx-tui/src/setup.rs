use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::event::{Event, EventHandler};
use crate::theme::{Theme, ThemeColors};

const DEFAULT_WATCHLIST: &[&str] = &["BTC_USDT", "ETH_USDT", "SOL_USDT", "CRO_USDT"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Theme,
    Watchlist,
    TickRate,
    Confirm,
    Writing,
    Done,
}

struct SetupState {
    step: Step,
    theme_idx: usize,
    themes: Vec<(&'static str, Theme)>,
    watchlist: Vec<String>,
    watchlist_input: String,
    editing_watchlist: bool,
    tick_rate_idx: usize,
    tick_rates: Vec<(u64, &'static str)>,
    error: Option<String>,
}

impl SetupState {
    fn new() -> Self {
        let themes: Vec<(&'static str, Theme)> = Theme::builtin_names()
            .iter()
            .map(|name| (*name, Theme::builtin(name).unwrap()))
            .collect();
        Self {
            step: Step::Welcome,
            theme_idx: 0,
            themes,
            watchlist: DEFAULT_WATCHLIST.iter().map(|s| s.to_string()).collect(),
            watchlist_input: String::new(),
            editing_watchlist: false,
            tick_rate_idx: 1, // 250ms default
            tick_rates: vec![
                (100, "100ms (fast, higher CPU)"),
                (250, "250ms (recommended)"),
                (500, "500ms (relaxed)"),
                (1000, "1000ms (slow, minimal CPU)"),
            ],
            error: None,
        }
    }

    fn selected_theme(&self) -> &Theme {
        &self.themes[self.theme_idx].1
    }

    fn selected_theme_name(&self) -> &str {
        self.themes[self.theme_idx].0
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        match self.step {
            Step::Welcome => {
                if key.code == KeyCode::Enter {
                    self.step = Step::Theme;
                } else if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                    return true; // quit
                }
            }
            Step::Theme => match key.code {
                KeyCode::Up if self.theme_idx > 0 => {
                    self.theme_idx -= 1;
                }
                KeyCode::Down if self.theme_idx < self.themes.len() - 1 => {
                    self.theme_idx += 1;
                }
                KeyCode::Enter => self.step = Step::Watchlist,
                KeyCode::Esc => self.step = Step::Welcome,
                _ => {}
            },
            Step::Watchlist => {
                if self.editing_watchlist {
                    match key.code {
                        KeyCode::Enter => {
                            let trimmed = self.watchlist_input.trim().to_uppercase();
                            if !trimmed.is_empty() && !self.watchlist.contains(&trimmed) {
                                self.watchlist.push(trimmed);
                            }
                            self.watchlist_input.clear();
                            self.editing_watchlist = false;
                        }
                        KeyCode::Esc => {
                            self.watchlist_input.clear();
                            self.editing_watchlist = false;
                        }
                        KeyCode::Backspace => {
                            self.watchlist_input.pop();
                        }
                        KeyCode::Char(c) => {
                            self.watchlist_input.push(c);
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('a') => {
                            self.editing_watchlist = true;
                        }
                        KeyCode::Char('d') if !self.watchlist.is_empty() => {
                            self.watchlist.pop();
                        }
                        KeyCode::Enter => self.step = Step::TickRate,
                        KeyCode::Esc => self.step = Step::Theme,
                        _ => {}
                    }
                }
            }
            Step::TickRate => match key.code {
                KeyCode::Up if self.tick_rate_idx > 0 => {
                    self.tick_rate_idx -= 1;
                }
                KeyCode::Down if self.tick_rate_idx < self.tick_rates.len() - 1 => {
                    self.tick_rate_idx += 1;
                }
                KeyCode::Enter => self.step = Step::Confirm,
                KeyCode::Esc => self.step = Step::Watchlist,
                _ => {}
            },
            Step::Confirm => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    self.step = Step::Writing;
                }
                KeyCode::Esc => self.step = Step::TickRate,
                _ => {}
            },
            Step::Writing => {} // handled externally
            Step::Done => {
                if key.code == KeyCode::Enter {
                    return true; // signal: proceed to dashboard
                }
            }
        }
        false
    }

    fn draw(&self, frame: &mut Frame) {
        let theme = self.selected_theme();
        let area = frame.area();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.colors.border))
            .title(" cdcx tui setup ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);

        match self.step {
            Step::Welcome => self.draw_welcome(frame, content_area, &theme.colors),
            Step::Theme => self.draw_theme(frame, content_area, &theme.colors),
            Step::Watchlist => self.draw_watchlist(frame, content_area, &theme.colors),
            Step::TickRate => self.draw_tick_rate(frame, content_area, &theme.colors),
            Step::Confirm => self.draw_confirm(frame, content_area, &theme.colors),
            Step::Writing => {
                frame.render_widget(
                    Paragraph::new("Writing config...")
                        .style(Style::default().fg(theme.colors.accent)),
                    content_area,
                );
            }
            Step::Done => self.draw_done(frame, content_area, &theme.colors),
        }

        // Footer
        let help = match self.step {
            Step::Welcome => "Enter:continue  Esc:quit",
            Step::Theme => "Up/Down:select  Enter:next  Esc:back",
            Step::Watchlist if self.editing_watchlist => "type instrument  Enter:add  Esc:cancel",
            Step::Watchlist => "a:add  d:remove last  Enter:next  Esc:back",
            Step::TickRate => "Up/Down:select  Enter:next  Esc:back",
            Step::Confirm => "Enter:save  Esc:back",
            Step::Writing => "",
            Step::Done => "Enter:launch dashboard",
        };
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(theme.colors.muted)),
            footer_area,
        );
    }

    fn draw_welcome(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let [_, content, _] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(6),
            Constraint::Fill(1),
        ])
        .areas(area);

        let lines = vec![
            Line::from(Span::styled(
                "  cdcx tui setup",
                Style::default()
                    .fg(colors.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Configure your terminal dashboard: theme, watchlist, and refresh rate.",
                Style::default().fg(colors.fg),
            )),
            Line::from(Span::styled(
                "  Settings are saved to ~/.config/cdcx/tui.toml",
                Style::default().fg(colors.muted),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press Enter to begin...",
                Style::default().fg(colors.muted),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), content);
    }

    fn draw_theme(&self, frame: &mut Frame, area: Rect, _colors: &ThemeColors) {
        let [title_area, _, list_area, _, preview_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(self.themes.len() as u16 + 1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(area);

        let active = self.selected_theme();

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  Step 1: ", Style::default().fg(active.colors.muted)),
                Span::styled(
                    "Choose a theme",
                    Style::default()
                        .fg(active.colors.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            title_area,
        );

        // Theme list with selection
        let rows: Vec<Row> = self
            .themes
            .iter()
            .enumerate()
            .map(|(i, (name, theme))| {
                let is_selected = i == self.theme_idx;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    Style::default()
                        .fg(theme.colors.selected_fg)
                        .bg(theme.colors.selected_bg)
                } else {
                    Style::default().fg(active.colors.fg)
                };
                Row::new(vec![Cell::from(format!("{}{}", prefix, name))]).style(style)
            })
            .collect();

        let table = Table::new(rows, [Constraint::Fill(1)]);
        frame.render_widget(table, list_area);

        // Live preview with the selected theme's colors
        let preview_theme = &self.themes[self.theme_idx].1;
        let c = &preview_theme.colors;

        let preview_lines = vec![
            Line::from(Span::styled("  Preview:", Style::default().fg(c.muted))),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  BTC_USDT  ",
                    Style::default().fg(c.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled("67,234.50  ", Style::default().fg(c.fg)),
                Span::styled("+2.34%  ", Style::default().fg(c.positive)),
                Span::styled("Vol: 1.2B  ", Style::default().fg(c.volume)),
            ]),
            Line::from(vec![
                Span::styled("  ETH_USDT  ", Style::default().fg(c.fg)),
                Span::styled(" 3,456.78  ", Style::default().fg(c.fg)),
                Span::styled("-0.87%  ", Style::default().fg(c.negative)),
                Span::styled("Vol: 892M  ", Style::default().fg(c.volume)),
            ]),
            Line::from(vec![
                Span::styled("  SOL_USDT  ", Style::default().fg(c.fg)),
                Span::styled("   178.92  ", Style::default().fg(c.fg)),
                Span::styled("+5.12%  ", Style::default().fg(c.positive)),
                Span::styled("Vol: 445M  ", Style::default().fg(c.volume)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Status: ", Style::default().fg(c.muted)),
                Span::styled("LIVE", Style::default().fg(c.positive)),
                Span::styled("  |  ", Style::default().fg(c.border)),
                Span::styled("PROD", Style::default().fg(c.status_bar_fg)),
            ]),
        ];
        frame.render_widget(Paragraph::new(preview_lines), preview_area);
    }

    fn draw_watchlist(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let [title_area, _, list_area, _, input_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .areas(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  Step 2: ", Style::default().fg(colors.muted)),
                Span::styled(
                    "Dashboard watchlist",
                    Style::default()
                        .fg(colors.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            title_area,
        );

        // Current watchlist
        if self.watchlist.is_empty() {
            frame.render_widget(
                Paragraph::new("  (empty — press a to add instruments)")
                    .style(Style::default().fg(colors.muted)),
                list_area,
            );
        } else {
            let lines: Vec<Line> = self
                .watchlist
                .iter()
                .enumerate()
                .map(|(i, inst)| {
                    Line::from(Span::styled(
                        format!("  {}. {}", i + 1, inst),
                        Style::default().fg(colors.fg),
                    ))
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), list_area);
        }

        // Input area
        if self.editing_watchlist {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("  Add: ", Style::default().fg(colors.accent)),
                    Span::styled(
                        format!("{}\u{2588}", self.watchlist_input),
                        Style::default().fg(colors.fg),
                    ),
                ])),
                input_area,
            );
        }
    }

    fn draw_tick_rate(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let [title_area, _, list_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  Step 3: ", Style::default().fg(colors.muted)),
                Span::styled(
                    "Refresh rate",
                    Style::default()
                        .fg(colors.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            title_area,
        );

        let rows: Vec<Row> = self
            .tick_rates
            .iter()
            .enumerate()
            .map(|(i, (_ms, label))| {
                let is_selected = i == self.tick_rate_idx;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    Style::default()
                        .fg(colors.selected_fg)
                        .bg(colors.selected_bg)
                } else {
                    Style::default().fg(colors.fg)
                };
                Row::new(vec![Cell::from(format!("{}{}", prefix, label))]).style(style)
            })
            .collect();

        let table = Table::new(rows, [Constraint::Fill(1)]);
        frame.render_widget(table, list_area);
    }

    fn draw_confirm(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let [title_area, _, summary_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(area);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  Review and save",
                Style::default()
                    .fg(colors.accent)
                    .add_modifier(Modifier::BOLD),
            ))),
            title_area,
        );

        let tick_label = self.tick_rates[self.tick_rate_idx].1;
        let lines = vec![
            Line::from(vec![
                Span::styled("  Theme:      ", Style::default().fg(colors.muted)),
                Span::styled(
                    self.selected_theme_name(),
                    Style::default().fg(colors.accent),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Watchlist:  ", Style::default().fg(colors.muted)),
                Span::styled(self.watchlist.join(", "), Style::default().fg(colors.fg)),
            ]),
            Line::from(vec![
                Span::styled("  Tick rate:  ", Style::default().fg(colors.muted)),
                Span::styled(tick_label, Style::default().fg(colors.fg)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  File:       ", Style::default().fg(colors.muted)),
                Span::styled("~/.config/cdcx/tui.toml", Style::default().fg(colors.fg)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  Press Enter to save, Esc to go back.",
                Style::default().fg(colors.muted),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), summary_area);
    }

    fn draw_done(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Setup complete!",
                Style::default()
                    .fg(colors.positive)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Config saved to ", Style::default().fg(colors.fg)),
                Span::styled(
                    "~/.config/cdcx/tui.toml",
                    Style::default().fg(colors.accent),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Theme: ", Style::default().fg(colors.muted)),
                Span::styled(
                    self.selected_theme_name(),
                    Style::default().fg(colors.accent),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Watchlist: ", Style::default().fg(colors.muted)),
                Span::styled(self.watchlist.join(", "), Style::default().fg(colors.fg)),
            ]),
            Line::from(""),
            if let Some(ref err) = self.error {
                Line::from(Span::styled(
                    format!("  Warning: {}", err),
                    Style::default().fg(colors.negative),
                ))
            } else {
                Line::from("")
            },
            Line::from(Span::styled(
                "  Press Enter to launch the dashboard...",
                Style::default().fg(colors.muted),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn build_toml(&self) -> String {
        let watchlist_str = self
            .watchlist
            .iter()
            .map(|w| format!("\"{}\"", w))
            .collect::<Vec<_>>()
            .join(", ");
        let tick_ms = self.tick_rates[self.tick_rate_idx].0;

        format!(
            "theme = \"{}\"\ntick_rate_ms = {}\nwatchlist = [{}]\n",
            self.selected_theme_name(),
            tick_ms,
            watchlist_str,
        )
    }

    fn write_config(&mut self) {
        let toml = self.build_toml();
        let Some(home) = dirs::home_dir() else {
            self.error = Some("Could not determine home directory".into());
            self.step = Step::Done;
            return;
        };
        let dir = home.join(".config").join("cdcx");
        #[cfg(unix)]
        let create_result = {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)
        };
        #[cfg(not(unix))]
        let create_result = std::fs::create_dir_all(&dir);

        if let Err(e) = create_result {
            self.error = Some(format!("Failed to create config directory: {}", e));
            self.step = Step::Done;
            return;
        }
        if let Err(e) = cdcx_core::config::set_dir_owner_only(&dir) {
            self.error = Some(format!("Failed to secure config directory: {}", e));
            self.step = Step::Done;
            return;
        }
        let path = dir.join("tui.toml");
        if let Err(e) = std::fs::write(&path, &toml) {
            self.error = Some(format!("Failed to write config: {}", e));
            self.step = Step::Done;
            return;
        }
        if let Err(e) = cdcx_core::config::set_file_owner_only(&path) {
            self.error = Some(format!("Failed to secure config file: {}", e));
            self.step = Step::Done;
            return;
        }
        self.error = None;
        self.step = Step::Done;
    }
}

/// Run the setup wizard. Returns true if the user completed it (proceed to dashboard),
/// false if they quit.
pub async fn run_setup(events: &mut EventHandler, terminal: &mut ratatui::DefaultTerminal) -> bool {
    let mut state = SetupState::new();

    loop {
        terminal.draw(|f| state.draw(f)).ok();

        // Handle the Writing step synchronously between frames
        if state.step == Step::Writing {
            state.write_config();
            continue;
        }

        if let Some(event) = events.next().await {
            match event {
                Event::Key(key) => {
                    let should_exit = state.on_key(key);
                    if should_exit {
                        return state.step == Step::Done;
                    }
                }
                Event::Mouse(_) => {}     // ignore in setup
                Event::Resize(_, _) => {} // redraw on next loop
                Event::Tick => {}
            }
        }
    }
}
