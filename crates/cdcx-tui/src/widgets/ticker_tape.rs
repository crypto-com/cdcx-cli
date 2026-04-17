use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::AppState;

/// Draw a scrolling ticker tape showing top gainers and losers.
pub fn draw_ticker_tape(frame: &mut Frame, area: Rect, state: &AppState, tick: u64) {
    if area.width < 10 {
        return;
    }

    let mut entries: Vec<(&str, f64, f64)> = state
        .tickers
        .iter()
        .filter(|(_, t)| t.ask > 0.0)
        .map(|(name, t)| (name.as_str(), t.ask, t.change_pct))
        .collect();
    entries.sort_by(|a, b| {
        b.2.abs()
            .partial_cmp(&a.2.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    entries.truncate(20);

    if entries.is_empty() {
        return;
    }

    // Build tape as a flat list of (char, color_index) pairs — no byte slicing needed
    // color_index: true = positive, false = negative
    let mut tape: Vec<(char, bool)> = Vec::new();
    for (name, price, change) in &entries {
        let positive = *change >= 0.0;
        let arrow = if positive { '\u{25b2}' } else { '\u{25bc}' };
        let segment = format!("  {} {:.2} {}{:.2}%  |", name, price, arrow, change);
        for ch in segment.chars() {
            tape.push((ch, positive));
        }
    }

    if tape.is_empty() {
        return;
    }

    let tape_len = tape.len();
    let width = area.width as usize;
    let divisor = state.ticker_speed_divisor.max(1) as usize;
    let scroll = (tick as usize / divisor) % tape_len;

    // Build visible line by picking chars from the tape (wrapping around)
    let mut spans: Vec<Span> = Vec::new();
    let mut current_text = String::new();
    let mut current_positive = tape[(scroll) % tape_len].1;

    for i in 0..width {
        let idx = (scroll + i) % tape_len;
        let (ch, positive) = tape[idx];

        // If color changes, flush current span and start a new one
        if positive != current_positive && !current_text.is_empty() {
            let color = if current_positive {
                state.theme.colors.positive
            } else {
                state.theme.colors.negative
            };
            spans.push(Span::styled(
                current_text.clone(),
                Style::default().fg(color),
            ));
            current_text.clear();
            current_positive = positive;
        }

        current_text.push(ch);
    }

    // Flush remaining
    if !current_text.is_empty() {
        let color = if current_positive {
            state.theme.colors.positive
        } else {
            state.theme.colors.negative
        };
        spans.push(Span::styled(current_text, Style::default().fg(color)));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(state.theme.colors.status_bar_bg)),
        area,
    );
}
