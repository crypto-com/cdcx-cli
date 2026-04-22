use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::theme::ThemeColors;

#[derive(Debug, Clone)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub timestamp: u64,
}

impl Candle {
    pub fn from_json(val: &serde_json::Value) -> Option<Self> {
        Some(Self {
            open: parse_f64(val, "o")?,
            high: parse_f64(val, "h")?,
            low: parse_f64(val, "l")?,
            close: parse_f64(val, "c")?,
            volume: val
                .get("v")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
            timestamp: val.get("t").and_then(|v| v.as_u64()).unwrap_or(0),
        })
    }

    /// Update this candle with new streaming data (same timestamp = same candle)
    pub fn update_from(&mut self, other: &Candle) {
        self.high = self.high.max(other.high);
        self.low = self.low.min(other.low);
        self.close = other.close;
        self.volume = other.volume;
    }
}

fn parse_f64(val: &serde_json::Value, key: &str) -> Option<f64> {
    val.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
}

/// Fill gaps in a candle series by inserting synthetic zero-volume flat candles for any
/// timeframe periods that the exchange omitted (illiquid instruments like RWA perps return
/// candles only for periods with trading activity). Synthetic candles carry forward the
/// previous close as O/H/L/C with `volume = 0`, so the renderer draws them as a flat line.
pub fn fill_candle_gaps(candles: &[Candle], interval_ms: u64) -> Vec<Candle> {
    if interval_ms == 0 || candles.len() < 2 {
        return candles.to_vec();
    }
    let mut out: Vec<Candle> = Vec::with_capacity(candles.len());
    out.push(candles[0].clone());
    for next in &candles[1..] {
        let prev_close = out.last().map(|c| c.close).unwrap_or(0.0);
        let prev_ts = out.last().map(|c| c.timestamp).unwrap_or(0);
        let mut t = prev_ts.saturating_add(interval_ms);
        while t < next.timestamp {
            out.push(Candle {
                open: prev_close,
                high: prev_close,
                low: prev_close,
                close: prev_close,
                volume: 0.0,
                timestamp: t,
            });
            t = t.saturating_add(interval_ms);
        }
        out.push(next.clone());
    }
    out
}

/// Draw a single-instrument candlestick chart with header and footer.
pub fn draw_candlestick(
    frame: &mut Frame,
    area: Rect,
    instrument: &str,
    candles: &[Candle],
    timeframe: &str,
    colors: &ThemeColors,
    footer_text: &str,
) {
    let [header_area, chart_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // Header
    let last_price = candles.last().map(|c| c.close).unwrap_or(0.0);
    let change = if candles.len() >= 2 {
        let prev = candles[candles.len() - 2].close;
        if prev > 0.0 {
            (last_price - prev) / prev * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };
    let change_color = if change >= 0.0 {
        colors.positive
    } else {
        colors.negative
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {} ", instrument),
                Style::default()
                    .fg(colors.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {} ", timeframe),
                Style::default().fg(colors.muted),
            ),
            if !candles.is_empty() {
                Span::styled(
                    format!(" {:.2} ({:+.2}%)", last_price, change),
                    Style::default().fg(change_color),
                )
            } else {
                Span::styled(" loading...", Style::default().fg(colors.muted))
            },
        ])),
        header_area,
    );

    // Chart body — no border for single chart (maximize space)
    render_chart_panel(frame, chart_area, candles, colors);

    // Footer
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer_text,
            Style::default().fg(colors.muted),
        ))),
        footer_area,
    );
}

/// Draw a compare view grid — just the charts, no footer (caller handles footer).
pub fn draw_compare_charts(
    frame: &mut Frame,
    area: Rect,
    charts: &[(&str, &[Candle])],
    timeframe: &str,
    colors: &ThemeColors,
) {
    let count = charts.len();
    if count == 0 {
        return;
    }

    match count {
        1 => {
            draw_chart_with_label(frame, area, charts[0].0, charts[0].1, timeframe, colors);
        }
        2 => {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(area);
            draw_chart_with_label(frame, left, charts[0].0, charts[0].1, timeframe, colors);
            draw_chart_with_label(frame, right, charts[1].0, charts[1].1, timeframe, colors);
        }
        _ => {
            let [top_row, bottom_row] =
                Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(area);
            let [tl, tr] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(top_row);
            let [bl, br] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(bottom_row);

            let cells = [tl, tr, bl, br];
            for (i, cell) in cells.iter().enumerate() {
                if i < charts.len() {
                    draw_chart_with_label(
                        frame,
                        *cell,
                        charts[i].0,
                        charts[i].1,
                        timeframe,
                        colors,
                    );
                }
            }
        }
    }
}

/// A single chart panel with instrument label, bordered.
fn draw_chart_with_label(
    frame: &mut Frame,
    area: Rect,
    instrument: &str,
    candles: &[Candle],
    timeframe: &str,
    colors: &ThemeColors,
) {
    let last_price = candles.last().map(|c| c.close).unwrap_or(0.0);
    let change = if candles.len() >= 2 {
        let prev = candles[candles.len() - 2].close;
        if prev > 0.0 {
            (last_price - prev) / prev * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };
    let change_color = if change >= 0.0 {
        colors.positive
    } else {
        colors.negative
    };

    let title = if candles.is_empty() {
        format!(" {} {} loading... ", instrument, timeframe)
    } else {
        format!(
            " {} {} {:.2} ({:+.2}%) ",
            instrument, timeframe, last_price, change
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.border))
        .title(Span::styled(title, Style::default().fg(change_color)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    render_chart_panel(frame, inner, candles, colors);
}

/// Core chart rendering — candle bodies and wicks with price axis.
pub fn render_chart_panel(frame: &mut Frame, area: Rect, candles: &[Candle], colors: &ThemeColors) {
    let chart_height = area.height as usize;
    let chart_width = area.width as usize;

    if chart_height < 3 || chart_width < 14 {
        return;
    }

    if candles.is_empty() {
        frame.render_widget(
            Paragraph::new("Waiting for data...").style(Style::default().fg(colors.muted)),
            area,
        );
        return;
    }

    // Price label is exactly 10 chars: "{:>9} " = 9 digits + 1 space
    // Each candle is exactly 3 display-width chars: 2 (body/wick) + 1 (space)
    // Be conservative — leave 1 char margin to prevent any overflow
    let price_col = 10usize;
    let usable = chart_width.saturating_sub(price_col + 1);
    let max_candles = usable / 3;

    if max_candles == 0 {
        return;
    }

    let visible: Vec<&Candle> = candles
        .iter()
        .rev()
        .take(max_candles)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut min_price = f64::MAX;
    let mut max_price = f64::MIN;
    for c in &visible {
        if c.low < min_price {
            min_price = c.low;
        }
        if c.high > max_price {
            max_price = c.high;
        }
    }

    let price_range = max_price - min_price;
    if price_range <= 0.0 {
        frame.render_widget(
            Paragraph::new("Insufficient price range.").style(Style::default().fg(colors.muted)),
            area,
        );
        return;
    }

    // Reserve bottom rows for volume bars
    let vol_rows = 3usize.min(chart_height / 4);
    let price_rows = chart_height.saturating_sub(vol_rows + 2); // +1 separator, +1 time axis

    // Find max volume for scaling
    let max_vol = visible.iter().map(|c| c.volume).fold(0.0f64, f64::max);

    let mut lines: Vec<Line> = Vec::with_capacity(chart_height);

    // Price chart rows
    let row_step = price_range / (price_rows.max(2) - 1) as f64;
    for row in 0..price_rows {
        let price_at_row = max_price - (row as f64 / (price_rows.max(2) - 1) as f64) * price_range;
        let mut spans: Vec<Span> = Vec::new();

        spans.push(Span::styled(
            format!("{:>9} ", format_chart_price(price_at_row)),
            Style::default().fg(colors.muted),
        ));

        for candle in &visible {
            // Synthetic no-trade candles (gap-fill) have zero volume and zero range —
            // snap them to the nearest price row and draw a dim dash so illiquid
            // periods render as a flat line instead of an empty column.
            let is_synthetic =
                candle.volume == 0.0 && candle.open == candle.close && candle.high == candle.low;
            if is_synthetic {
                if (price_at_row - candle.close).abs() <= row_step / 2.0 {
                    // Fill the full 3-char cell (no inter-cell space) so adjacent
                    // synthetic candles join into one continuous horizontal line.
                    spans.push(Span::styled(
                        "\u{2500}\u{2500}\u{2500}",
                        Style::default().fg(colors.muted),
                    ));
                } else {
                    spans.push(Span::raw("   "));
                }
                continue;
            }

            let is_bullish = candle.close >= candle.open;
            let body_top = candle.open.max(candle.close);
            let body_bot = candle.open.min(candle.close);
            let color = if is_bullish {
                colors.positive
            } else {
                colors.negative
            };

            let in_wick = price_at_row <= candle.high && price_at_row >= candle.low;
            let in_body = price_at_row <= body_top && price_at_row >= body_bot;

            let (ch, style) = if in_body {
                ("\u{2588}\u{2588}", Style::default().fg(color))
            } else if in_wick {
                (" \u{2502}", Style::default().fg(color))
            } else {
                ("  ", Style::default())
            };

            spans.push(Span::styled(ch, style));
            spans.push(Span::raw(" "));
        }

        lines.push(Line::from(spans));
    }

    // Separator with max vol label
    if vol_rows > 0 {
        let vol_label = format_vol(max_vol);
        let mut sep_spans: Vec<Span> = Vec::new();
        sep_spans.push(Span::styled(
            format!("{:>9} ", vol_label),
            Style::default().fg(colors.muted),
        ));
        let sep_width = visible.len() * 3;
        sep_spans.push(Span::styled(
            "\u{2500}".repeat(sep_width),
            Style::default().fg(colors.border),
        ));
        lines.push(Line::from(sep_spans));
    }

    // Volume bar rows
    let vol_blocks: &[char] = &[
        ' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
        '\u{2588}',
    ];
    for vrow in 0..vol_rows {
        let mut spans: Vec<Span> = Vec::new();
        // Volume scale: show "0" on the bottom row
        if vrow == vol_rows - 1 {
            spans.push(Span::styled(
                format!("{:>9} ", "0"),
                Style::default().fg(colors.muted),
            ));
        } else {
            spans.push(Span::styled("          ", Style::default()));
        }

        for candle in &visible {
            let is_bullish = candle.close >= candle.open;
            let color = if is_bullish {
                colors.positive
            } else {
                colors.negative
            };

            let vol_pct = if max_vol > 0.0 {
                candle.volume / max_vol
            } else {
                0.0
            };
            let filled_rows = vol_pct * vol_rows as f64;
            let row_from_bottom = (vol_rows - 1 - vrow) as f64;

            let block_idx = if filled_rows > row_from_bottom + 1.0 {
                vol_blocks.len() - 1
            } else if filled_rows > row_from_bottom {
                let frac = filled_rows - row_from_bottom;
                (frac * (vol_blocks.len() - 1) as f64).round() as usize
            } else {
                0
            };

            let ch = vol_blocks[block_idx.min(vol_blocks.len() - 1)];
            let s = format!("{}{} ", ch, ch);
            spans.push(Span::styled(s, Style::default().fg(color)));
        }

        lines.push(Line::from(spans));
    }

    // Time axis — show timestamps at intervals along the bottom
    if !visible.is_empty() {
        let mut time_spans: Vec<Span> = Vec::new();
        time_spans.push(Span::styled("          ", Style::default())); // price label width

        let step = (visible.len() / 5).max(1); // ~5 labels across the width
        for (i, candle) in visible.iter().enumerate() {
            if i % step == 0 && candle.timestamp > 0 {
                let secs = (candle.timestamp / 1000) as i64;
                let h = ((secs % 86400) / 3600) as u8;
                let m = ((secs % 3600) / 60) as u8;
                let label = format!("{:02}:{:02}", h, m);
                // Pad to align with candle width (3 chars per candle)
                let pad_needed = (step * 3).saturating_sub(label.len());
                time_spans.push(Span::styled(label, Style::default().fg(colors.muted)));
                if pad_needed > 0 {
                    time_spans.push(Span::raw(" ".repeat(pad_needed)));
                }
            }
        }
        lines.push(Line::from(time_spans));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn format_vol(vol: f64) -> String {
    if vol >= 1_000_000.0 {
        format!("{:.1}M", vol / 1_000_000.0)
    } else if vol >= 1_000.0 {
        format!("{:.1}K", vol / 1_000.0)
    } else if vol >= 1.0 {
        format!("{:.1}", vol)
    } else {
        format!("{:.4}", vol)
    }
}

fn format_chart_price(price: f64) -> String {
    if price >= 1000.0 {
        format!("{:.0}", price)
    } else if price >= 1.0 {
        format!("{:.2}", price)
    } else {
        format!("{:.4}", price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(ts: u64, close: f64) -> Candle {
        Candle {
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            timestamp: ts,
        }
    }

    #[test]
    fn fill_candle_gaps_inserts_flat_candles_for_missing_periods() {
        // 1h interval; missing two periods between t=0 and t=3h
        let interval_ms = 3_600_000u64;
        let input = vec![candle(0, 100.0), candle(3 * interval_ms, 110.0)];
        let out = fill_candle_gaps(&input, interval_ms);

        // Expect 4 candles total: real@0, synthetic@1h, synthetic@2h, real@3h
        assert_eq!(out.len(), 4, "two gaps must be filled");
        assert_eq!(out[1].timestamp, interval_ms);
        assert_eq!(out[2].timestamp, 2 * interval_ms);
        // Synthetic carry-forward: O=H=L=C=prev close, v=0
        for synthetic in &out[1..=2] {
            assert_eq!(synthetic.open, 100.0);
            assert_eq!(synthetic.close, 100.0);
            assert_eq!(synthetic.high, 100.0);
            assert_eq!(synthetic.low, 100.0);
            assert_eq!(synthetic.volume, 0.0);
        }
        // Real candle preserved at correct index
        assert_eq!(out[3].close, 110.0);
        assert_eq!(out[3].volume, 1.0);
    }

    #[test]
    fn fill_candle_gaps_noop_when_contiguous() {
        let interval_ms = 60_000u64;
        let input = vec![candle(0, 1.0), candle(interval_ms, 2.0)];
        let out = fill_candle_gaps(&input, interval_ms);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn fill_candle_gaps_handles_empty_and_single() {
        assert!(fill_candle_gaps(&[], 60_000).is_empty());
        let one = vec![candle(0, 1.0)];
        assert_eq!(fill_candle_gaps(&one, 60_000).len(), 1);
    }
}
