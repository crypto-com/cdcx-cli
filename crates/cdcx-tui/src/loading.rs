use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme::ThemeColors;

const LOGO: &[&str] = &[
    "  \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2557}  \u{2588}\u{2588}\u{2557}",
    "  \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2554}\u{255d}",
    "  \u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}  \u{2588}\u{2588}\u{2551}\u{2588}\u{2588}\u{2551}      \u{255a}\u{2588}\u{2588}\u{2588}\u{2554}\u{255d} ",
    "  \u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}  \u{2588}\u{2588}\u{2551}\u{2588}\u{2588}\u{2551}      \u{2588}\u{2588}\u{2554}\u{2588}\u{2588}\u{2557} ",
    "  \u{255a}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2554}\u{255d}\u{255a}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2554}\u{255d}\u{2588}\u{2588}\u{2557}",
    "   \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}  \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{255d} \u{255a}\u{2550}\u{255d}",
];

const WAVE_CHARS: &[char] = &[
    ' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
    '\u{2588}',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingStep {
    Instruments,
    Tickers,
    Connecting,
    Done,
}

pub struct LoadingState {
    pub step: LoadingStep,
    pub instrument_count: usize,
    pub ticker_count: usize,
    pub frame: u64,
}

impl Default for LoadingState {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadingState {
    pub fn new() -> Self {
        Self {
            step: LoadingStep::Instruments,
            instrument_count: 0,
            ticker_count: 0,
            frame: 0,
        }
    }

    pub fn tick(&mut self) {
        self.frame += 1;
    }
}

pub fn draw_loading(frame: &mut Frame, state: &LoadingState, colors: &ThemeColors) {
    let area = frame.area();

    let [_, logo_area, _, wave_area, _, status_area, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(6),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(5),
        Constraint::Fill(1),
    ])
    .areas(area);

    // Logo — centered, accent colored
    let logo_lines: Vec<Line> = LOGO
        .iter()
        .map(|line| Line::from(Span::styled(*line, Style::default().fg(colors.accent))))
        .collect();
    let logo_width = 38; // approximate logo width
    let logo_offset = (area.width.saturating_sub(logo_width)) / 2;
    let logo_rect = Rect::new(
        area.x + logo_offset,
        logo_area.y,
        logo_width.min(area.width),
        logo_area.height,
    );
    frame.render_widget(Paragraph::new(logo_lines), logo_rect);

    // Subtitle
    let subtitle = Line::from(Span::styled(
        "Crypto.com Exchange Terminal",
        Style::default().fg(colors.muted),
    ));
    let sub_width = 28u16;
    let sub_offset = (area.width.saturating_sub(sub_width)) / 2;
    frame.render_widget(
        Paragraph::new(subtitle),
        Rect::new(area.x + sub_offset, logo_area.y + 6, sub_width, 1),
    );

    // Animated wave — sine wave using block characters
    let wave_width = area.width.saturating_sub(8) as usize;
    if wave_width > 4 {
        let wave_line = generate_wave(wave_width, state.frame, colors);
        let wave_offset = 4u16;
        frame.render_widget(
            Paragraph::new(wave_line),
            Rect::new(area.x + wave_offset, wave_area.y + 1, wave_width as u16, 1),
        );
    }

    // Status steps — centered
    let status_width = 44u16;
    let status_offset = (area.width.saturating_sub(status_width)) / 2;
    let status_rect = Rect::new(
        area.x + status_offset,
        status_area.y,
        status_width.min(area.width),
        status_area.height,
    );

    let spinner_frames = [
        "\u{28fb}", "\u{28fd}", "\u{28fe}", "\u{28f7}", "\u{28ef}", "\u{28df}", "\u{28bf}",
        "\u{287f}",
    ];
    let spinner = spinner_frames[(state.frame as usize / 2) % spinner_frames.len()];

    let check = "\u{2713}";

    let lines = vec![
        status_line(
            LoadingStep::Instruments,
            state,
            check,
            spinner,
            "Loading instruments",
            &format!("({} found)", state.instrument_count),
            colors,
        ),
        status_line(
            LoadingStep::Tickers,
            state,
            check,
            spinner,
            "Fetching initial tickers",
            &format!("({} loaded)", state.ticker_count),
            colors,
        ),
        status_line(
            LoadingStep::Connecting,
            state,
            check,
            spinner,
            "Preparing dashboard",
            "",
            colors,
        ),
    ];

    frame.render_widget(Paragraph::new(lines), status_rect);
}

fn status_line(
    step: LoadingStep,
    state: &LoadingState,
    check: &str,
    spinner: &str,
    label: &str,
    detail: &str,
    colors: &ThemeColors,
) -> Line<'static> {
    let is_complete = (step as u8) < (state.step as u8);
    let is_active = step == state.step;

    let (icon, icon_color) = if is_complete {
        (check.to_string(), colors.positive)
    } else if is_active {
        (spinner.to_string(), colors.accent)
    } else {
        ("\u{00b7}".to_string(), colors.muted)
    };

    let label_color = if is_complete || is_active {
        colors.fg
    } else {
        colors.muted
    };

    let mut spans = vec![
        Span::styled(format!("  {} ", icon), Style::default().fg(icon_color)),
        Span::styled(label.to_string(), Style::default().fg(label_color)),
    ];

    if is_complete && !detail.is_empty() {
        spans.push(Span::styled(
            format!(" {}", detail),
            Style::default().fg(colors.muted),
        ));
    }

    Line::from(spans)
}

fn generate_wave(width: usize, frame: u64, colors: &ThemeColors) -> Line<'static> {
    let mut spans = Vec::with_capacity(width);
    let phase = frame as f64 * 0.15;

    for i in 0..width {
        let x = i as f64 * 0.12 + phase;
        // Composite of two sine waves for a more organic look
        let y = (x.sin() * 0.4 + (x * 2.3).sin() * 0.3 + (x * 0.7).sin() * 0.3 + 1.0) / 2.0;
        let idx = (y * (WAVE_CHARS.len() - 1) as f64).round() as usize;
        let ch = WAVE_CHARS[idx.min(WAVE_CHARS.len() - 1)];

        let color = if y > 0.7 {
            colors.positive
        } else if y > 0.4 {
            colors.accent
        } else {
            colors.muted
        };

        spans.push(Span::styled(String::from(ch), Style::default().fg(color)));
    }

    Line::from(spans)
}
