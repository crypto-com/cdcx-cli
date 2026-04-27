use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Default)]
pub struct TuiConfig {
    pub theme: Option<String>,
    pub tick_rate_ms: Option<u64>,
    pub ticker_speed: Option<String>,
    pub default_instrument: Option<String>,
    #[serde(default)]
    pub watchlist: Vec<String>,
    #[serde(default)]
    pub themes: HashMap<String, CustomThemeConfig>,
    /// Opt into the price-api research pane (Bloomberg-style split view with
    /// coin metadata / ranges / social / news). Off by default while the
    /// underlying consumer-web API is unofficial. Overridden by CLI flag
    /// `--beta-research-pane` or env var `CDCX_BETA_RESEARCH_PANE=1`.
    pub beta_research_pane: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CustomThemeConfig {
    pub bg: Option<String>,
    pub fg: Option<String>,
    pub accent: Option<String>,
    pub positive: Option<String>,
    pub negative: Option<String>,
    pub border: Option<String>,
    pub header: Option<String>,
    pub selected_bg: Option<String>,
    pub selected_fg: Option<String>,
    pub muted: Option<String>,
    pub volume: Option<String>,
    pub status_bar_bg: Option<String>,
    pub status_bar_fg: Option<String>,
}

impl CustomThemeConfig {
    /// Convert this custom config into a Theme, using a base theme for defaults.
    pub fn to_theme(&self, name: &str, base: &crate::theme::ThemeColors) -> crate::theme::Theme {
        use ratatui::style::Color;
        let p = |hex: &Option<String>, default: Color| -> Color {
            hex.as_deref().and_then(parse_hex_color).unwrap_or(default)
        };
        crate::theme::Theme {
            name: name.to_string(),
            colors: crate::theme::ThemeColors {
                bg: p(&self.bg, base.bg),
                fg: p(&self.fg, base.fg),
                accent: p(&self.accent, base.accent),
                positive: p(&self.positive, base.positive),
                negative: p(&self.negative, base.negative),
                border: p(&self.border, base.border),
                header: p(&self.header, base.header),
                selected_bg: p(&self.selected_bg, base.selected_bg),
                selected_fg: p(&self.selected_fg, base.selected_fg),
                muted: p(&self.muted, base.muted),
                volume: p(&self.volume, base.volume),
                status_bar_bg: p(&self.status_bar_bg, base.status_bar_bg),
                status_bar_fg: p(&self.status_bar_fg, base.status_bar_fg),
            },
        }
    }
}

fn parse_hex_color(hex: &str) -> Option<ratatui::style::Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(ratatui::style::Color::Rgb(r, g, b))
}

impl TuiConfig {
    pub fn load() -> Self {
        let Some(home) = dirs::home_dir() else {
            return Self::default();
        };
        let path = home.join(".config").join("cdcx").join("tui.toml");
        let Ok(content) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&content).unwrap_or_default()
    }

    pub fn exists() -> bool {
        dirs::home_dir()
            .map(|h| h.join(".config").join("cdcx").join("tui.toml").exists())
            .unwrap_or(false)
    }

    pub fn tick_rate(&self) -> u64 {
        self.tick_rate_ms.unwrap_or(250)
    }

    /// Ticker tape scroll speed as a tick divisor. Higher = slower.
    pub fn ticker_speed_divisor(&self) -> u64 {
        match self.ticker_speed.as_deref() {
            Some("slow") => 4,
            Some("fast") => 1,
            _ => 2, // "medium" or default
        }
    }

    /// Resolve the beta research-pane flag with the documented precedence:
    /// CLI flag > env var > config file > default (false).
    ///
    /// `flag` is the `--beta-research-pane` CLI arg (true if present). Env
    /// var accepts `1`, `true`, `yes` (case-insensitive) to enable.
    pub fn resolve_beta_research_pane(&self, flag: bool) -> bool {
        if flag {
            return true;
        }
        if let Ok(v) = std::env::var("CDCX_BETA_RESEARCH_PANE") {
            let v = v.to_ascii_lowercase();
            if matches!(v.as_str(), "1" | "true" | "yes") {
                return true;
            }
            if matches!(v.as_str(), "0" | "false" | "no") {
                return false;
            }
            // Unrecognised value: fall through to config.
        }
        self.beta_research_pane.unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
            theme = "cyber-midnight"
            tick_rate_ms = 500
            default_instrument = "ETH_USDT"
            watchlist = ["BTC_USDT", "ETH_USDT"]
        "#;
        let config: TuiConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.theme.as_deref(), Some("cyber-midnight"));
        assert_eq!(config.tick_rate(), 500);
        assert_eq!(config.watchlist.len(), 2);
    }

    #[test]
    fn test_default_config() {
        let config = TuiConfig::default();
        assert_eq!(config.tick_rate(), 250);
        assert!(config.watchlist.is_empty());
    }

    #[test]
    fn test_empty_toml() {
        let config: TuiConfig = toml::from_str("").unwrap();
        assert!(config.theme.is_none());
        assert_eq!(config.tick_rate(), 250);
    }

    #[test]
    fn test_beta_flag_defaults_off() {
        std::env::remove_var("CDCX_BETA_RESEARCH_PANE");
        let cfg = TuiConfig::default();
        assert!(!cfg.resolve_beta_research_pane(false));
    }

    #[test]
    fn test_beta_cli_flag_wins_over_config_false() {
        std::env::remove_var("CDCX_BETA_RESEARCH_PANE");
        let cfg = TuiConfig {
            beta_research_pane: Some(false),
            ..Default::default()
        };
        assert!(
            cfg.resolve_beta_research_pane(true),
            "CLI --beta-research-pane must override config's explicit false"
        );
    }

    #[test]
    fn test_beta_config_on_enables_when_no_flag() {
        std::env::remove_var("CDCX_BETA_RESEARCH_PANE");
        let cfg = TuiConfig {
            beta_research_pane: Some(true),
            ..Default::default()
        };
        assert!(cfg.resolve_beta_research_pane(false));
    }
}
