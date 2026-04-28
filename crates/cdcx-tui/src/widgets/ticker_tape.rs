use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph};
use ratatui::Frame;

use crate::state::{AppState, UpdateState};

/// Draw a scrolling ticker tape showing top gainers and losers.
/// When an update notice or progress is present, pins it to the left.
pub fn draw_ticker_tape(frame: &mut Frame, area: Rect, state: &AppState, tick: u64) {
    if area.width < 10 {
        return;
    }

    // If actively updating, show progress bar on the left instead of notice
    let notice_width = state
        .update_notice
        .as_ref()
        .map(|n| (n.len() as u16 + 5).min(area.width / 2))
        .unwrap_or(area.width / 3);

    let (tape_area, left_area) = if let Some(ref progress) = state.update_progress {
        let width = notice_width.max(20).min(area.width / 2);
        let [left, right] = ratatui::layout::Layout::horizontal([
            ratatui::layout::Constraint::Length(width),
            ratatui::layout::Constraint::Fill(1),
        ])
        .areas(area);

        match progress {
            UpdateState::Downloading { downloaded, total } => {
                let (ratio, label) = if let Some(t) = total {
                    let r = (*downloaded as f64 / *t as f64).min(1.0);
                    let mb = *downloaded as f64 / 1_048_576.0;
                    let total_mb = *t as f64 / 1_048_576.0;
                    (r, format!("{:.1}/{:.1} MB", mb, total_mb))
                } else {
                    let mb = *downloaded as f64 / 1_048_576.0;
                    (0.0, format!("{:.1} MB...", mb))
                };
                frame.render_widget(
                    Gauge::default()
                        .ratio(ratio)
                        .label(label)
                        .gauge_style(
                            Style::default()
                                .fg(state.theme.colors.accent)
                                .bg(state.theme.colors.status_bar_bg),
                        )
                        .style(
                            Style::default()
                                .fg(state.theme.colors.status_bar_fg)
                                .bg(state.theme.colors.status_bar_bg),
                        ),
                    left,
                );
            }
            UpdateState::Extracting | UpdateState::Installing => {
                let label = match progress {
                    UpdateState::Extracting => "Extracting...",
                    _ => "Installing...",
                };
                frame.render_widget(
                    Gauge::default()
                        .ratio(0.5)
                        .label(label)
                        .gauge_style(
                            Style::default()
                                .fg(state.theme.colors.accent)
                                .bg(state.theme.colors.status_bar_bg),
                        )
                        .style(
                            Style::default()
                                .fg(state.theme.colors.status_bar_fg)
                                .bg(state.theme.colors.status_bar_bg),
                        ),
                    left,
                );
            }
            UpdateState::Done { version } => {
                let text = format!(" \u{2714} Updated to v{} \u{2014} restarting... ", version);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        text,
                        Style::default()
                            .fg(state.theme.colors.positive)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(state.theme.colors.status_bar_bg)),
                    left,
                );
            }
            UpdateState::Failed(msg) => {
                let short = if msg.len() > 30 { &msg[..30] } else { msg };
                let text = format!(" \u{2717} {} ", short);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        text,
                        Style::default()
                            .fg(state.theme.colors.negative)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(state.theme.colors.status_bar_bg)),
                    left,
                );
            }
        }

        (right, Some(left))
    } else if let Some(ref notice) = state.update_notice {
        let notice_width = (notice.len() as u16 + 5).min(area.width / 2);
        let [left, right] = ratatui::layout::Layout::horizontal([
            ratatui::layout::Constraint::Length(notice_width),
            ratatui::layout::Constraint::Fill(1),
        ])
        .areas(area);

        let text = format!(" \u{1f680} {} ", notice);
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                text,
                Style::default().fg(state.theme.colors.muted),
            )]))
            .style(Style::default().bg(state.theme.colors.status_bar_bg)),
            left,
        );
        (right, Some(left))
    } else {
        (area, None)
    };

    let _ = left_area;
    let area = tape_area;

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
