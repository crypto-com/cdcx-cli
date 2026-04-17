use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::state::AppState;

pub fn draw_spotlight(frame: &mut Frame, area: Rect, instrument: &str, state: &AppState) {
    let width = 42u16.min(area.width.saturating_sub(4));
    let height = 16u16.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal = Rect::new(x, y, width, height);

    frame.render_widget(Clear, modal);

    let colors = &state.theme.colors;
    let ticker = state.tickers.get(instrument);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        format!(" {} ", instrument),
        Style::default()
            .fg(colors.accent)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    if let Some(t) = ticker {
        let change_color = if t.change_pct >= 0.0 {
            colors.positive
        } else {
            colors.negative
        };

        lines.push(Line::from(Span::styled(
            format!(" {:.2}", t.ask),
            Style::default()
                .fg(colors.header)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!(" {:+.2}%", t.change_pct * 100.0),
            Style::default().fg(change_color),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" High: ", Style::default().fg(colors.muted)),
            Span::styled(format!("{:.2}", t.high), Style::default().fg(colors.fg)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(" Low:  ", Style::default().fg(colors.muted)),
            Span::styled(format!("{:.2}", t.low), Style::default().fg(colors.fg)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Bid:  ", Style::default().fg(colors.muted)),
            Span::styled(
                format!("{:.2}", t.bid),
                Style::default().fg(colors.positive),
            ),
            Span::styled("  Ask: ", Style::default().fg(colors.muted)),
            Span::styled(
                format!("{:.2}", t.ask),
                Style::default().fg(colors.negative),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled(" Vol:  ", Style::default().fg(colors.muted)),
            Span::styled(
                format!("{:.0}", t.volume_usd),
                Style::default().fg(colors.volume),
            ),
        ]));

        if instrument.contains("-PERP") && t.funding_rate != 0.0 {
            let fr_color = if t.funding_rate >= 0.0 {
                colors.positive
            } else {
                colors.negative
            };
            lines.push(Line::from(vec![
                Span::styled(" Fund: ", Style::default().fg(colors.muted)),
                Span::styled(
                    format!("{:+.4}%", t.funding_rate * 100.0),
                    Style::default().fg(fr_color),
                ),
            ]));
        }

        // Sparkline
        if let Some(spark_data) = state.sparklines.get(instrument) {
            if !spark_data.is_empty() {
                lines.push(Line::from(""));
                let blocks = [
                    '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}',
                    '\u{2587}', '\u{2588}',
                ];
                let min = spark_data.iter().copied().fold(f64::MAX, f64::min);
                let max = spark_data.iter().copied().fold(f64::MIN, f64::max);
                let range = max - min;
                let spark: String = spark_data
                    .iter()
                    .map(|&p| {
                        let idx = if range > 0.0 {
                            ((p - min) / range * 7.0).round() as usize
                        } else {
                            4
                        };
                        blocks[idx.min(7)]
                    })
                    .collect();
                lines.push(Line::from(Span::styled(
                    format!(" {}", spark),
                    Style::default().fg(colors.accent),
                )));
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            " No data available",
            Style::default().fg(colors.muted),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " t:trade  Esc/any:close",
        Style::default().fg(colors.muted),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.accent))
        .title(" Spotlight ");

    frame.render_widget(Paragraph::new(lines).block(block), modal);
}
