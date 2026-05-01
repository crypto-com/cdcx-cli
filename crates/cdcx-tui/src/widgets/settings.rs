use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::theme::{Theme, ThemeColors};

const TICK_RATES: &[(u64, &str)] = &[
    (100, "100ms (fast, higher CPU)"),
    (250, "250ms (recommended)"),
    (500, "500ms (relaxed)"),
    (1000, "1000ms (slow, minimal CPU)"),
];

const TICKER_SPEEDS: &[(u64, &str, &str)] = &[
    (4, "Slow", "slow"),
    (2, "Medium", "medium"),
    (1, "Fast", "fast"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsRow {
    Theme,
    TickerSpeed,
    TickRate,
}

impl SettingsRow {
    const ALL: &[SettingsRow] = &[
        SettingsRow::Theme,
        SettingsRow::TickerSpeed,
        SettingsRow::TickRate,
    ];

    fn label(&self) -> &'static str {
        match self {
            SettingsRow::Theme => "Theme",
            SettingsRow::TickerSpeed => "Ticker Tape",
            SettingsRow::TickRate => "Tick Rate",
        }
    }
}

pub enum SettingsAction {
    /// Keep the panel open, no external effect.
    None,
    /// Theme was changed — caller should apply it live.
    ThemeChanged(Theme),
    /// Ticker tape speed changed — caller should apply it live.
    TickerSpeedChanged(u64),
    /// User saved — caller should persist and apply.
    Save {
        theme: Theme,
        tick_rate_ms: u64,
        ticker_speed_divisor: u64,
    },
    /// User closed without saving — caller should revert theme and speed.
    Close,
}

pub struct SettingsPanel {
    selected: usize,
    theme_idx: usize,
    themes: Vec<(&'static str, Theme)>,
    ticker_speed_idx: usize,
    tick_rate_idx: usize,
    original_theme_name: String,
    original_ticker_speed_divisor: u64,
    original_tick_rate_ms: u64,
    saved: bool,
}

impl SettingsPanel {
    pub fn new(
        current_theme_name: &str,
        current_tick_rate_ms: u64,
        current_ticker_speed_divisor: u64,
    ) -> Self {
        let themes: Vec<(&'static str, Theme)> = Theme::builtin_names()
            .iter()
            .map(|name| (*name, Theme::builtin(name).unwrap()))
            .collect();

        let theme_idx = themes
            .iter()
            .position(|(name, _)| *name == current_theme_name)
            .unwrap_or(0);

        let tick_rate_idx = TICK_RATES
            .iter()
            .position(|(ms, _)| *ms == current_tick_rate_ms)
            .unwrap_or(1);

        let ticker_speed_idx = TICKER_SPEEDS
            .iter()
            .position(|(div, _, _)| *div == current_ticker_speed_divisor)
            .unwrap_or(1); // default to medium

        Self {
            selected: 0,
            theme_idx,
            themes,
            ticker_speed_idx,
            tick_rate_idx,
            original_theme_name: current_theme_name.to_string(),
            original_ticker_speed_divisor: current_ticker_speed_divisor,
            original_tick_rate_ms: current_tick_rate_ms,
            saved: false,
        }
    }

    pub fn selected_theme(&self) -> &Theme {
        &self.themes[self.theme_idx].1
    }

    pub fn selected_theme_name(&self) -> &str {
        self.themes[self.theme_idx].0
    }

    pub fn selected_tick_rate_ms(&self) -> u64 {
        TICK_RATES[self.tick_rate_idx].0
    }

    pub fn selected_ticker_speed_divisor(&self) -> u64 {
        TICKER_SPEEDS[self.ticker_speed_idx].0
    }

    fn selected_ticker_speed_label(&self) -> &'static str {
        TICKER_SPEEDS[self.ticker_speed_idx].1
    }

    pub fn on_key(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char(',') => {
                if self.saved {
                    return SettingsAction::Close;
                }
                // Revert theme and ticker speed if changed but not saved
                let theme_changed = self.selected_theme_name() != self.original_theme_name;
                let speed_changed =
                    self.selected_ticker_speed_divisor() != self.original_ticker_speed_divisor;
                if theme_changed || speed_changed {
                    if let Some(original) = Theme::builtin(&self.original_theme_name) {
                        return SettingsAction::Save {
                            theme: original,
                            tick_rate_ms: self.original_tick_rate_ms,
                            ticker_speed_divisor: self.original_ticker_speed_divisor,
                        };
                    }
                }
                SettingsAction::Close
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                SettingsAction::None
            }
            KeyCode::Down => {
                if self.selected < SettingsRow::ALL.len() - 1 {
                    self.selected += 1;
                }
                SettingsAction::None
            }
            KeyCode::Left => self.cycle_value(-1),
            KeyCode::Right | KeyCode::Tab => self.cycle_value(1),
            KeyCode::Enter => {
                self.saved = true;
                SettingsAction::Save {
                    theme: self.selected_theme().clone(),
                    tick_rate_ms: self.selected_tick_rate_ms(),
                    ticker_speed_divisor: self.selected_ticker_speed_divisor(),
                }
            }
            _ => SettingsAction::None,
        }
    }

    fn cycle_value(&mut self, direction: i32) -> SettingsAction {
        match SettingsRow::ALL[self.selected] {
            SettingsRow::Theme => {
                let len = self.themes.len();
                if direction > 0 {
                    self.theme_idx = (self.theme_idx + 1) % len;
                } else {
                    self.theme_idx = (self.theme_idx + len - 1) % len;
                }
                SettingsAction::ThemeChanged(self.selected_theme().clone())
            }
            SettingsRow::TickerSpeed => {
                let len = TICKER_SPEEDS.len();
                if direction > 0 {
                    self.ticker_speed_idx = (self.ticker_speed_idx + 1) % len;
                } else {
                    self.ticker_speed_idx = (self.ticker_speed_idx + len - 1) % len;
                }
                SettingsAction::TickerSpeedChanged(self.selected_ticker_speed_divisor())
            }
            SettingsRow::TickRate => {
                let len = TICK_RATES.len();
                if direction > 0 {
                    self.tick_rate_idx = (self.tick_rate_idx + 1) % len;
                } else {
                    self.tick_rate_idx = (self.tick_rate_idx + len - 1) % len;
                }
                SettingsAction::None
            }
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let width = 52u16;
        let height = 20u16;
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let modal = Rect::new(x, y, width.min(area.width), height.min(area.height));

        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.accent))
            .title(" Settings ");
        let inner = block.inner(modal);
        frame.render_widget(block, modal);

        let [settings_area, _, preview_area, _, footer_area] = Layout::vertical([
            Constraint::Length(5),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);

        // Settings rows
        let mut lines = Vec::new();
        for (i, row) in SettingsRow::ALL.iter().enumerate() {
            let is_selected = i == self.selected;
            let label = row.label();
            let value = match row {
                SettingsRow::Theme => self.selected_theme_name().to_string(),
                SettingsRow::TickerSpeed => self.selected_ticker_speed_label().to_string(),
                SettingsRow::TickRate => {
                    let (ms, desc) = TICK_RATES[self.tick_rate_idx];
                    if ms == self.original_tick_rate_ms {
                        desc.to_string()
                    } else {
                        format!("{} *", desc)
                    }
                }
            };

            let arrow_style = if is_selected {
                Style::default().fg(colors.accent)
            } else {
                Style::default().fg(colors.muted)
            };
            let label_style = if is_selected {
                Style::default()
                    .fg(colors.header)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.fg)
            };
            let value_style = if is_selected {
                Style::default().fg(colors.accent)
            } else {
                Style::default().fg(colors.fg)
            };

            lines.push(Line::from(vec![
                Span::styled(if is_selected { " \u{25b6} " } else { "   " }, arrow_style),
                Span::styled(format!("{:<12}", label), label_style),
                Span::styled(" \u{25c0} ", arrow_style),
                Span::styled(value, value_style),
                Span::styled(" \u{25b6}", arrow_style),
            ]));
        }

        // Tick rate note
        if self.selected_tick_rate_ms() != self.original_tick_rate_ms {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "   * takes effect on next launch",
                Style::default().fg(colors.muted),
            )));
        }

        frame.render_widget(Paragraph::new(lines), settings_area);

        // Live theme preview
        let preview_theme = self.selected_theme();
        let c = &preview_theme.colors;
        let preview_lines = vec![
            Line::from(Span::styled(" Preview:", Style::default().fg(c.muted))),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    " BTC_USDT  ",
                    Style::default().fg(c.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled("67,234.50  ", Style::default().fg(c.fg)),
                Span::styled("+2.34%  ", Style::default().fg(c.positive)),
                Span::styled("Vol: 1.2B", Style::default().fg(c.volume)),
            ]),
            Line::from(vec![
                Span::styled(" ETH_USDT  ", Style::default().fg(c.fg)),
                Span::styled(" 3,456.78  ", Style::default().fg(c.fg)),
                Span::styled("-0.87%  ", Style::default().fg(c.negative)),
                Span::styled("Vol: 892M", Style::default().fg(c.volume)),
            ]),
            Line::from(vec![
                Span::styled(" SOL_USDT  ", Style::default().fg(c.fg)),
                Span::styled("   178.92  ", Style::default().fg(c.fg)),
                Span::styled("+5.12%  ", Style::default().fg(c.positive)),
                Span::styled("Vol: 445M", Style::default().fg(c.volume)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(" Status: ", Style::default().fg(c.muted)),
                Span::styled("LIVE", Style::default().fg(c.positive)),
                Span::styled("  |  ", Style::default().fg(c.border)),
                Span::styled(
                    "PROD",
                    Style::default().fg(c.status_bar_fg).bg(c.status_bar_bg),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(preview_lines), preview_area);

        // Footer
        let footer = Line::from(vec![
            Span::styled(" Enter", Style::default().fg(colors.accent)),
            Span::styled(":save  ", Style::default().fg(colors.muted)),
            Span::styled("Esc", Style::default().fg(colors.accent)),
            Span::styled(":close  ", Style::default().fg(colors.muted)),
            Span::styled("\u{2190}\u{2192}", Style::default().fg(colors.accent)),
            Span::styled(":change", Style::default().fg(colors.muted)),
        ]);
        frame.render_widget(Paragraph::new(footer), footer_area);
    }
}

/// Read-modify-write a tui.toml file at `path` through a `toml::Table`. This
/// preserves unknown keys (custom themes, forward-compatibility), which is
/// why we parse into a dynamic Table instead of round-tripping TuiConfig.
/// Split out so tests can pass a tempdir path.
fn mutate_tui_toml_at<F>(path: &std::path::Path, mutate: F) -> Result<(), String>
where
    F: FnOnce(&mut toml::Table),
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config dir: {}", e))?;
    }

    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut config: toml::Table = existing.parse().unwrap_or_default();

    mutate(&mut config);

    let toml_str = toml::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    let schema_url = cdcx_core::github::raw("main", "schemas/configs/tui.json");
    let output = format!("#:schema {}\n\n{}", schema_url, toml_str);
    std::fs::write(path, output).map_err(|e| format!("Failed to write config: {}", e))?;

    Ok(())
}

fn default_tui_toml_path() -> Result<std::path::PathBuf, String> {
    let Some(home) = dirs::home_dir() else {
        return Err("Could not determine home directory".into());
    };
    Ok(home.join(".config").join("cdcx").join("tui.toml"))
}

fn mutate_tui_toml<F>(mutate: F) -> Result<(), String>
where
    F: FnOnce(&mut toml::Table),
{
    let path = default_tui_toml_path()?;
    mutate_tui_toml_at(&path, mutate)
}

/// Write settings to ~/.config/cdcx/tui.toml, preserving watchlist and custom themes.
pub fn save_settings(
    theme_name: &str,
    tick_rate_ms: u64,
    ticker_speed: &str,
) -> Result<(), String> {
    mutate_tui_toml(|config| {
        config.insert("theme".into(), toml::Value::String(theme_name.into()));
        config.insert(
            "tick_rate_ms".into(),
            toml::Value::Integer(tick_rate_ms as i64),
        );
        config.insert(
            "ticker_speed".into(),
            toml::Value::String(ticker_speed.into()),
        );
    })
}

/// Persist the current watchlist to ~/.config/cdcx/tui.toml so add/remove
/// actions survive across sessions. Preserves every other key.
pub fn save_watchlist(instruments: &[String]) -> Result<(), String> {
    save_watchlist_at(&default_tui_toml_path()?, instruments)
}

/// Internal form of `save_watchlist` that writes to an explicit path. Tests
/// use this to verify round-trip behavior against a tempdir.
pub(crate) fn save_watchlist_at(
    path: &std::path::Path,
    instruments: &[String],
) -> Result<(), String> {
    let values: Vec<toml::Value> = instruments
        .iter()
        .map(|s| toml::Value::String(s.clone()))
        .collect();
    mutate_tui_toml_at(path, |config| {
        config.insert("watchlist".into(), toml::Value::Array(values));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_panel_new_defaults() {
        let panel = SettingsPanel::new("terminal-pro", 250, 2);
        assert_eq!(panel.selected_theme_name(), "terminal-pro");
        assert_eq!(panel.selected_tick_rate_ms(), 250);
    }

    #[test]
    fn test_settings_panel_new_custom_tick_rate() {
        let panel = SettingsPanel::new("cyber-midnight", 500, 2);
        assert_eq!(panel.selected_theme_name(), "cyber-midnight");
        assert_eq!(panel.selected_tick_rate_ms(), 500);
    }

    #[test]
    fn test_cycle_theme_right() {
        let mut panel = SettingsPanel::new("terminal-pro", 250, 2);
        panel.selected = 0; // Theme row
        let action = panel.cycle_value(1);
        assert!(matches!(action, SettingsAction::ThemeChanged(_)));
        assert_eq!(panel.selected_theme_name(), "cyber-midnight");
    }

    #[test]
    fn test_cycle_theme_wraps() {
        let names = Theme::builtin_names();
        let last = names[names.len() - 1];
        let mut panel = SettingsPanel::new(last, 250, 2);
        panel.selected = 0;
        panel.cycle_value(1);
        assert_eq!(panel.selected_theme_name(), "terminal-pro");
    }

    #[test]
    fn test_cycle_ticker_speed() {
        let mut panel = SettingsPanel::new("terminal-pro", 250, 2);
        panel.selected = 1; // TickerSpeed row
        let action = panel.cycle_value(1);
        assert!(matches!(action, SettingsAction::TickerSpeedChanged(1))); // medium -> fast
        assert_eq!(panel.selected_ticker_speed_divisor(), 1);
    }

    #[test]
    fn test_cycle_tick_rate() {
        let mut panel = SettingsPanel::new("terminal-pro", 250, 2);
        panel.selected = 2; // TickRate row
        panel.cycle_value(1);
        assert_eq!(panel.selected_tick_rate_ms(), 500);
    }

    #[test]
    fn test_cycle_tick_rate_wraps() {
        let mut panel = SettingsPanel::new("terminal-pro", 1000, 2);
        panel.selected = 2;
        panel.cycle_value(1);
        assert_eq!(panel.selected_tick_rate_ms(), 100);
    }

    #[test]
    fn test_enter_returns_save() {
        let mut panel = SettingsPanel::new("terminal-pro", 250, 2);
        let action = panel.on_key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(action, SettingsAction::Save { .. }));
    }

    #[test]
    fn test_esc_returns_close() {
        let mut panel = SettingsPanel::new("terminal-pro", 250, 2);
        let action = panel.on_key(KeyEvent::new(
            KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(matches!(action, SettingsAction::Close));
    }

    #[test]
    fn test_esc_reverts_theme_change() {
        let mut panel = SettingsPanel::new("terminal-pro", 250, 2);
        panel.selected = 0;
        panel.cycle_value(1); // change to cyber-midnight
        assert_ne!(panel.selected_theme_name(), "terminal-pro");
        let action = panel.on_key(KeyEvent::new(
            KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        // Should revert — returns Save with original theme
        assert!(matches!(action, SettingsAction::Save { .. }));
    }

    #[test]
    fn test_navigate_rows() {
        let mut panel = SettingsPanel::new("terminal-pro", 250, 2);
        assert_eq!(panel.selected, 0);
        panel.on_key(KeyEvent::new(
            KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(panel.selected, 1);
        panel.on_key(KeyEvent::new(
            KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(panel.selected, 2);
        panel.on_key(KeyEvent::new(
            KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(panel.selected, 2); // clamped
        panel.on_key(KeyEvent::new(
            KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(panel.selected, 1);
    }

    // ---- Watchlist persistence (Issue #23) ----

    /// Build a unique path under the system temp dir for a test. Each test
    /// gets its own file; no cleanup needed — OS reclaims /tmp eventually.
    fn temp_tui_toml(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("cdcx-test-{}-{}.toml", tag, nanos))
    }

    /// save_watchlist_at on a fresh file must write the `watchlist` key so
    /// TuiConfig::load() can read it back.
    #[test]
    fn save_watchlist_persists_to_disk() {
        let path = temp_tui_toml("save-watchlist");
        let _ = std::fs::remove_file(&path);

        save_watchlist_at(&path, &["BTC_USDT".into(), "ETH_USDT".into()])
            .expect("save must succeed on writable tempdir");

        let content = std::fs::read_to_string(&path).expect("file must exist");
        assert!(
            content.contains("watchlist"),
            "output must contain `watchlist` key, got: {}",
            content
        );
        assert!(content.contains("BTC_USDT"));
        assert!(content.contains("ETH_USDT"));

        // Round-trip: parse it back through a TuiConfig-shaped toml.
        let parsed: toml::Table = content
            .lines()
            .filter(|l| !l.starts_with("#:schema"))
            .collect::<Vec<_>>()
            .join("\n")
            .parse()
            .expect("output must be valid TOML");
        let arr = parsed
            .get("watchlist")
            .and_then(|v| v.as_array())
            .expect("watchlist must round-trip as an array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str(), Some("BTC_USDT"));

        let _ = std::fs::remove_file(&path);
    }

    /// Subsequent saves must replace the watchlist entry, not append to it.
    /// Also verifies that other pre-existing keys (theme, custom themes) are
    /// preserved across the save — that's the whole reason we use the
    /// read-modify-write Table approach instead of serializing TuiConfig.
    #[test]
    fn save_watchlist_preserves_other_keys() {
        let path = temp_tui_toml("preserve-other-keys");
        let _ = std::fs::remove_file(&path);

        // Seed the file with a theme + a custom theme block the user may have.
        let seed = "theme = \"cyber-midnight\"\n\
tick_rate_ms = 500\n\
\n\
[themes.my-custom]\n\
bg = \"#000000\"\n\
fg = \"#ffffff\"\n";
        std::fs::create_dir_all(path.parent().unwrap()).ok();
        std::fs::write(&path, seed).expect("seed write");

        save_watchlist_at(&path, &["ADA_USDT".into()]).expect("save");

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(content.contains("cyber-midnight"), "theme must survive");
        assert!(
            content.contains("my-custom"),
            "custom theme table must survive"
        );
        assert!(content.contains("ADA_USDT"), "new watchlist entry present");

        // And re-saving over an existing watchlist must replace, not append.
        save_watchlist_at(&path, &["DOGE_USDT".into()]).expect("second save");
        let content2 = std::fs::read_to_string(&path).expect("re-read");
        assert!(!content2.contains("ADA_USDT"), "old entry must be gone");
        assert!(content2.contains("DOGE_USDT"));

        let _ = std::fs::remove_file(&path);
    }

    /// Empty watchlist still writes a valid file with `watchlist = []`. This
    /// matters: if the user removes all entries, we must not leave the prior
    /// list in place.
    #[test]
    fn save_watchlist_handles_empty_list() {
        let path = temp_tui_toml("empty-watchlist");
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir_all(path.parent().unwrap()).ok();
        std::fs::write(&path, "watchlist = [\"BTC_USDT\"]\n").ok();

        save_watchlist_at(&path, &[]).expect("save empty");

        let content = std::fs::read_to_string(&path).expect("read");
        assert!(!content.contains("BTC_USDT"), "prior entry must be cleared");
        let parsed: toml::Table = content
            .lines()
            .filter(|l| !l.starts_with("#:schema"))
            .collect::<Vec<_>>()
            .join("\n")
            .parse()
            .expect("valid TOML");
        let arr = parsed.get("watchlist").and_then(|v| v.as_array()).unwrap();
        assert!(arr.is_empty());

        let _ = std::fs::remove_file(&path);
    }
}
