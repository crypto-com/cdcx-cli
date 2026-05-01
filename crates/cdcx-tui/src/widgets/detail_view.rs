use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::state::AppState;

/// Instrument detail view: ticker summary + order book + recent trades
pub fn draw_detail(
    frame: &mut Frame,
    area: Rect,
    instrument: &str,
    state: &AppState,
    book_data: &Option<serde_json::Value>,
    recent_trades: &[serde_json::Value],
) {
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // Header: instrument + ticker summary
    let ticker = state.tickers.get(instrument);
    let header_line = if let Some(t) = ticker {
        let change_color = if t.change_pct >= 0.0 {
            state.theme.colors.positive
        } else {
            state.theme.colors.negative
        };
        Line::from(vec![
            Span::styled(
                format!(" {} ", instrument),
                Style::default()
                    .fg(state.theme.colors.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:.2}", t.ask),
                Style::default()
                    .fg(state.theme.colors.fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:+.2}%", t.change_pct * 100.0),
                Style::default().fg(change_color),
            ),
            Span::raw("  "),
            Span::styled(
                format!("H:{:.2}", t.high),
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::raw("  "),
            Span::styled(
                format!("L:{:.2}", t.low),
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::raw("  "),
            Span::styled(
                format!("Bid:{:.2}  Ask:{:.2}", t.bid, t.ask),
                Style::default().fg(state.theme.colors.volume),
            ),
        ])
    } else {
        Line::from(Span::styled(
            format!(" {} — loading...", instrument),
            Style::default().fg(state.theme.colors.muted),
        ))
    };
    frame.render_widget(Paragraph::new(header_line), header_area);

    // Body: side-by-side book + trades
    let [book_area, trades_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(body_area);

    // Order book
    draw_book(frame, book_area, state, instrument, book_data);

    // Recent trades
    draw_trades(frame, trades_area, state, recent_trades);

    // Footer
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Esc:back to table  k:candlestick chart  t:trade",
            Style::default().fg(state.theme.colors.muted),
        ))),
        footer_area,
    );
}

fn draw_book(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    instrument: &str,
    book_data: &Option<serde_json::Value>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(state.theme.colors.border))
        .title(" Order Book ");
    let full_inner = block.inner(area);
    frame.render_widget(block, area);

    // Split: table area + 1-row pressure bar at bottom
    let [inner, pressure_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(full_inner);

    let Some(raw) = book_data else {
        frame.render_widget(
            Paragraph::new("Loading book...").style(Style::default().fg(state.theme.colors.muted)),
            inner,
        );
        return;
    };

    // Book is at data[0] inside the result
    let book = raw
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first());

    let Some(book) = book else {
        frame.render_widget(
            Paragraph::new("No book data").style(Style::default().fg(state.theme.colors.muted)),
            inner,
        );
        return;
    };

    let bids = book.get("bids").and_then(|v| v.as_array());
    let asks = book.get("asks").and_then(|v| v.as_array());

    let max_rows = (inner.height as usize).saturating_sub(1);

    let header = Row::new(vec!["Price", "Qty", "Total", "Depth"]).style(
        Style::default()
            .fg(state.theme.colors.header)
            .add_modifier(Modifier::BOLD),
    );
    let widths = [
        Constraint::Length(12),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Fill(1), // Depth column fills remaining space
    ];

    // Parse quantities
    let parse_levels = |items: &[serde_json::Value], n: usize| -> Vec<(String, String, f64)> {
        items
            .iter()
            .take(n)
            .filter_map(|item| {
                let arr = item.as_array()?;
                let price = arr.first()?.as_str()?.to_string();
                let qty_str = arr.get(1)?.as_str()?.to_string();
                let qty: f64 = qty_str.parse().unwrap_or(0.0);
                Some((price, qty_str, qty))
            })
            .collect()
    };

    // Reserve 2 rows for: current price + pressure bar at bottom
    let book_half = (max_rows.saturating_sub(3)) / 2;

    let ask_levels = asks.map(|a| parse_levels(a, book_half)).unwrap_or_default();
    let bid_levels = bids.map(|b| parse_levels(b, book_half)).unwrap_or_default();

    // Cumulative
    let cumulate = |levels: &[(String, String, f64)]| -> Vec<f64> {
        let mut cum = 0.0;
        levels
            .iter()
            .map(|(_, _, q)| {
                cum += q;
                cum
            })
            .collect()
    };
    let ask_cum = cumulate(&ask_levels);
    let bid_cum = cumulate(&bid_levels);
    let max_cum = ask_cum
        .last()
        .copied()
        .unwrap_or(0.0)
        .max(bid_cum.last().copied().unwrap_or(0.0));

    let mut rows: Vec<Row> = Vec::new();

    // Asks (reversed: furthest from spread at top, closest at bottom)
    for (i, (price, qty_str, _)) in ask_levels.iter().enumerate().rev() {
        let cum_qty = ask_cum[i];
        let bar = depth_bar(cum_qty, max_cum, 20);
        let intensity = if max_cum > 0.0 {
            (cum_qty / max_cum).min(1.0)
        } else {
            0.0
        };
        let bar_color = intensity_color(state.theme.colors.negative, intensity);
        rows.push(Row::new(vec![
            Cell::from(price.clone()).style(Style::default().fg(state.theme.colors.negative)),
            Cell::from(qty_str.clone()).style(Style::default().fg(state.theme.colors.fg)),
            Cell::from(format!("{:.5}", cum_qty))
                .style(Style::default().fg(state.theme.colors.muted)),
            Cell::from(bar).style(Style::default().fg(bar_color)),
        ]));
    }

    // Current price row (spread midpoint)
    if let Some(ticker) = state.tickers.get(instrument) {
        let arrow = if ticker.change_pct >= 0.0 {
            "\u{2191}"
        } else {
            "\u{2193}"
        };
        let color = if ticker.change_pct >= 0.0 {
            state.theme.colors.positive
        } else {
            state.theme.colors.negative
        };
        rows.push(Row::new(vec![
            Cell::from(format!("{:.2} {}", ticker.ask, arrow))
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Cell::from(""),
            Cell::from(""),
            Cell::from(""),
        ]));
    }

    // Bids
    for (i, (price, qty_str, _)) in bid_levels.iter().enumerate() {
        let cum_qty = bid_cum[i];
        let bar = depth_bar(cum_qty, max_cum, 20);
        let intensity = if max_cum > 0.0 {
            (cum_qty / max_cum).min(1.0)
        } else {
            0.0
        };
        let bar_color = intensity_color(state.theme.colors.positive, intensity);
        rows.push(Row::new(vec![
            Cell::from(price.clone()).style(Style::default().fg(state.theme.colors.positive)),
            Cell::from(qty_str.clone()).style(Style::default().fg(state.theme.colors.fg)),
            Cell::from(format!("{:.5}", cum_qty))
                .style(Style::default().fg(state.theme.colors.muted)),
            Cell::from(bar).style(Style::default().fg(bar_color)),
        ]));
    }

    let table = Table::new(rows, widths).header(header).column_spacing(1);
    frame.render_widget(table, inner);

    // Pressure bar — full width, rendered as a Paragraph below the table
    let total_ask: f64 = ask_levels.iter().map(|(_, _, q)| q).sum();
    let total_bid: f64 = bid_levels.iter().map(|(_, _, q)| q).sum();
    let total = total_ask + total_bid;
    if total > 0.0 && pressure_area.width > 10 {
        let bid_pct = total_bid / total * 100.0;
        let ask_pct = total_ask / total * 100.0;
        let full_w = pressure_area.width as usize;
        // Reserve chars for labels: "B 45.7% " = ~9 chars, " 54.3% S" = ~9 chars
        let label_left = format!("B {:.1}% ", bid_pct);
        let label_right = format!(" {:.1}% S", ask_pct);
        let bar_w = full_w.saturating_sub(label_left.len() + label_right.len());
        let bid_bar_w = ((bid_pct / 100.0) * bar_w as f64).round() as usize;
        let ask_bar_w = bar_w.saturating_sub(bid_bar_w);

        let line = Line::from(vec![
            Span::styled(
                label_left,
                Style::default()
                    .fg(state.theme.colors.positive)
                    .bg(Color::Rgb(0, 30, 0)),
            ),
            Span::styled(
                "\u{2588}".repeat(bid_bar_w),
                Style::default()
                    .fg(state.theme.colors.positive)
                    .bg(Color::Rgb(0, 30, 0)),
            ),
            Span::styled(
                "\u{2588}".repeat(ask_bar_w),
                Style::default()
                    .fg(state.theme.colors.negative)
                    .bg(Color::Rgb(30, 0, 0)),
            ),
            Span::styled(
                label_right,
                Style::default()
                    .fg(state.theme.colors.negative)
                    .bg(Color::Rgb(30, 0, 0)),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), pressure_area);
    }
}

fn draw_trades(frame: &mut Frame, area: Rect, state: &AppState, trades: &[serde_json::Value]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(state.theme.colors.border))
        .title(" Recent Trades ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if trades.is_empty() {
        frame.render_widget(
            Paragraph::new("Loading trades...")
                .style(Style::default().fg(state.theme.colors.muted)),
            inner,
        );
        return;
    }

    let max_rows = (inner.height as usize).saturating_sub(1);

    let header = Row::new(vec!["Price", "Qty", "Side"]).style(
        Style::default()
            .fg(state.theme.colors.header)
            .add_modifier(Modifier::BOLD),
    );
    let widths = [
        Constraint::Length(14),
        Constraint::Length(12),
        Constraint::Length(6),
    ];

    let rows: Vec<Row> = trades
        .iter()
        .take(max_rows)
        .map(|item| {
            let price = item.get("p").and_then(|v| v.as_str()).unwrap_or("");
            let qty = item.get("q").and_then(|v| v.as_str()).unwrap_or("");
            let side_raw = item.get("s").and_then(|v| v.as_str()).unwrap_or("");
            let side = side_raw.to_uppercase();
            let color = if side == "BUY" {
                state.theme.colors.positive
            } else {
                state.theme.colors.negative
            };
            Row::new(vec![
                Cell::from(price.to_string()),
                Cell::from(qty.to_string()),
                Cell::from(side.clone()),
            ])
            .style(Style::default().fg(color))
        })
        .collect();

    let table = Table::new(rows, widths).header(header).column_spacing(1);
    frame.render_widget(table, inner);
}

/// Scale a theme color's brightness by intensity (0.3 min to 1.0).
/// Keeps the color's hue from the theme, adjusts brightness for the heatmap effect.
fn intensity_color(base: Color, intensity: f64) -> Color {
    // Minimum brightness 30% so even small orders are visible
    let scale = 0.3 + intensity * 0.7;
    match base {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * scale) as u8,
            (g as f64 * scale) as u8,
            (b as f64 * scale) as u8,
        ),
        Color::Green => Color::Rgb(0, (100.0 + intensity * 155.0) as u8, 0),
        Color::Red => Color::Rgb((100.0 + intensity * 155.0) as u8, 0, 0),
        // For non-RGB colors, approximate
        other => other,
    }
}

fn depth_bar(qty: f64, max_qty: f64, width: usize) -> String {
    if max_qty <= 0.0 || qty <= 0.0 {
        return " ".repeat(width);
    }
    let filled = ((qty / max_qty) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        " ".repeat(width - filled)
    )
}
