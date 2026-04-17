use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub colors: ThemeColors,
}

#[derive(Debug, Clone)]
pub struct ThemeColors {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub positive: Color,
    pub negative: Color,
    pub border: Color,
    pub header: Color,
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub muted: Color,
    pub volume: Color,
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
}

impl Theme {
    pub fn builtin(name: &str) -> Option<Self> {
        match name {
            "terminal-pro" => Some(Self::terminal_pro()),
            "cyber-midnight" => Some(Self::cyber_midnight()),
            "monochrome" => Some(Self::monochrome()),
            "neon" => Some(Self::neon()),
            "micky-d" => Some(Self::micky_d()),
            "amber" => Some(Self::amber()),
            _ => None,
        }
    }

    pub fn builtin_names() -> &'static [&'static str] {
        &[
            "terminal-pro",
            "cyber-midnight",
            "monochrome",
            "neon",
            "micky-d",
            "amber",
        ]
    }

    fn terminal_pro() -> Self {
        Self {
            name: "terminal-pro".into(),
            colors: ThemeColors {
                bg: Color::Rgb(18, 18, 18),
                fg: Color::Rgb(220, 220, 220),
                accent: Color::Rgb(255, 180, 0),
                positive: Color::Green,
                negative: Color::Rgb(255, 68, 68),
                border: Color::Rgb(68, 68, 68),
                header: Color::White,
                selected_bg: Color::Rgb(255, 180, 0),
                selected_fg: Color::Black,
                muted: Color::Rgb(120, 120, 120),
                volume: Color::Rgb(100, 180, 255),
                status_bar_bg: Color::Rgb(30, 30, 30),
                status_bar_fg: Color::Rgb(180, 180, 180),
            },
        }
    }

    fn cyber_midnight() -> Self {
        Self {
            name: "cyber-midnight".into(),
            colors: ThemeColors {
                bg: Color::Rgb(10, 10, 30),
                fg: Color::Rgb(200, 200, 220),
                accent: Color::Rgb(0, 212, 255),
                positive: Color::Rgb(0, 255, 136),
                negative: Color::Rgb(255, 68, 100),
                border: Color::Rgb(50, 50, 80),
                header: Color::White,
                selected_bg: Color::Rgb(0, 212, 255),
                selected_fg: Color::Black,
                muted: Color::Rgb(100, 100, 140),
                volume: Color::Rgb(255, 170, 0),
                status_bar_bg: Color::Rgb(15, 15, 35),
                status_bar_fg: Color::Rgb(160, 160, 200),
            },
        }
    }

    fn monochrome() -> Self {
        Self {
            name: "monochrome".into(),
            colors: ThemeColors {
                bg: Color::Black,
                fg: Color::White,
                accent: Color::White,
                positive: Color::White,
                negative: Color::DarkGray,
                border: Color::DarkGray,
                header: Color::White,
                selected_bg: Color::White,
                selected_fg: Color::Black,
                muted: Color::DarkGray,
                volume: Color::White,
                status_bar_bg: Color::DarkGray,
                status_bar_fg: Color::White,
            },
        }
    }

    fn neon() -> Self {
        Self {
            name: "neon".into(),
            colors: ThemeColors {
                bg: Color::Rgb(10, 0, 20),
                fg: Color::Rgb(230, 230, 255),
                accent: Color::Rgb(255, 0, 255),  // hot magenta
                positive: Color::Rgb(0, 255, 65), // electric green
                negative: Color::Rgb(255, 0, 80), // neon pink-red
                border: Color::Rgb(80, 0, 120),   // deep purple
                header: Color::Rgb(255, 255, 255),
                selected_bg: Color::Rgb(255, 0, 255), // magenta highlight
                selected_fg: Color::Rgb(0, 0, 0),
                muted: Color::Rgb(120, 80, 160), // muted purple
                volume: Color::Rgb(0, 200, 255), // electric blue
                status_bar_bg: Color::Rgb(20, 0, 40),
                status_bar_fg: Color::Rgb(200, 150, 255),
            },
        }
    }

    fn micky_d() -> Self {
        // Inspired by the golden arches — warm golds, ketchup reds, dark bg
        Self {
            name: "micky-d".into(),
            colors: ThemeColors {
                bg: Color::Rgb(25, 12, 0),             // deep fryer brown-black
                fg: Color::Rgb(255, 220, 160),         // warm cream
                accent: Color::Rgb(241, 196, 15),      // golden arches yellow #f1c40f
                positive: Color::Rgb(241, 196, 15),    // bullish = golden yellow
                negative: Color::Rgb(231, 76, 60),     // bearish = ketchup red #e74c3c
                border: Color::Rgb(100, 60, 20),       // warm brown
                header: Color::Rgb(255, 235, 180),     // light gold
                selected_bg: Color::Rgb(230, 126, 34), // mcdonalds orange #e67e22
                selected_fg: Color::Rgb(0, 0, 0),
                muted: Color::Rgb(140, 100, 50),      // muted caramel
                volume: Color::Rgb(230, 126, 34),     // orange #e67e22
                status_bar_bg: Color::Rgb(40, 20, 0), // dark warm
                status_bar_fg: Color::Rgb(212, 172, 13), // mustard gold #d4ac0d
            },
        }
    }
    fn amber() -> Self {
        // Classic financial terminal — black bg, signature amber/orange,
        // green/red for price movement, navy-blue accents
        Self {
            name: "amber".into(),
            colors: ThemeColors {
                bg: Color::Rgb(0, 0, 0),                // pure black terminal
                fg: Color::Rgb(255, 176, 0),            // amber/orange
                accent: Color::Rgb(255, 141, 0),        // deeper orange for highlights
                positive: Color::Rgb(0, 200, 0),        // green (up)
                negative: Color::Rgb(255, 48, 48),      // red (down)
                border: Color::Rgb(48, 48, 48),         // subtle dark gray grid lines
                header: Color::Rgb(255, 176, 0),        // amber headers
                selected_bg: Color::Rgb(0, 48, 120),    // navy-blue selection
                selected_fg: Color::Rgb(255, 255, 255), // white text on blue
                muted: Color::Rgb(128, 128, 128),       // gray for secondary info
                volume: Color::Rgb(80, 160, 255),       // steel blue for volume bars
                status_bar_bg: Color::Rgb(0, 40, 100),  // dark navy status bar
                status_bar_fg: Color::Rgb(255, 176, 0), // amber status text
            },
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::terminal_pro()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_terminal_pro() {
        let theme = Theme::builtin("terminal-pro").unwrap();
        assert_eq!(theme.name, "terminal-pro");
        assert_eq!(theme.colors.positive, Color::Green);
    }

    #[test]
    fn test_builtin_cyber_midnight() {
        let theme = Theme::builtin("cyber-midnight").unwrap();
        assert_eq!(theme.name, "cyber-midnight");
    }

    #[test]
    fn test_builtin_neon() {
        let theme = Theme::builtin("neon").unwrap();
        assert_eq!(theme.name, "neon");
        assert_eq!(theme.colors.accent, Color::Rgb(255, 0, 255));
    }

    #[test]
    fn test_builtin_micky_d() {
        let theme = Theme::builtin("micky-d").unwrap();
        assert_eq!(theme.name, "micky-d");
        assert_eq!(theme.colors.positive, Color::Rgb(241, 196, 15)); // golden arches
    }

    #[test]
    fn test_builtin_amber() {
        let theme = Theme::builtin("amber").unwrap();
        assert_eq!(theme.name, "amber");
        assert_eq!(theme.colors.bg, Color::Rgb(0, 0, 0));
        assert_eq!(theme.colors.fg, Color::Rgb(255, 176, 0));
    }

    #[test]
    fn test_builtin_names_all_resolve() {
        for name in Theme::builtin_names() {
            assert!(
                Theme::builtin(name).is_some(),
                "Theme '{}' failed to resolve",
                name
            );
        }
    }

    #[test]
    fn test_builtin_fallback() {
        assert!(Theme::builtin("nonexistent").is_none());
    }

    #[test]
    fn test_default_is_terminal_pro() {
        let theme = Theme::default();
        assert_eq!(theme.name, "terminal-pro");
    }
}
