//! Panel renderers for the Discover tab. Pure functions over snapshot data —
//! the tab owns the fetch/refresh lifecycle and hands each panel whatever is
//! currently loaded (or `None` while pending).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use cdcx_core::price_api::{
    DirectoryEntry, MarketPair, RedditPost, SocialMetrics, StatisticsResponse, TrendingToken,
    VideoNews,
};

use crate::format::{format_compact, format_price};
use crate::theme::ThemeColors;

/// Sub-tab used inside the news panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsPanel {
    /// Highest-upvote Reddit posts + top videos, interleaved by time.
    Highlights,
    Reddit,
    Video,
}

impl NewsPanel {
    pub fn label(&self) -> &'static str {
        match self {
            NewsPanel::Highlights => "Highlights",
            NewsPanel::Reddit => "Reddit",
            NewsPanel::Video => "Video",
        }
    }
}

fn panel_block<'a>(title: &'a str, colors: &ThemeColors) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.border))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ))
}

/// Header line: symbol, name, rank, current price, 24h change.
pub fn render_header(
    frame: &mut Frame,
    area: Rect,
    entry: Option<&DirectoryEntry>,
    price: Option<f64>,
    change_24h: Option<f64>,
    colors: &ThemeColors,
) {
    let mut spans: Vec<Span> = Vec::new();
    if let Some(e) = entry {
        spans.push(Span::styled(
            format!(" {} ", e.symbol),
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{} ", e.name),
            Style::default().fg(colors.fg),
        ));
        if let Some(rank) = e.rank {
            spans.push(Span::styled(
                format!("#{}  ", rank),
                Style::default().fg(colors.muted),
            ));
        }
    } else {
        spans.push(Span::styled(" — ", Style::default().fg(colors.muted)));
    }
    if let Some(p) = price {
        spans.push(Span::styled(
            format!("${}  ", format_price(p)),
            Style::default().fg(colors.fg).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(c) = change_24h {
        let color = if c >= 0.0 {
            colors.positive
        } else {
            colors.negative
        };
        spans.push(Span::styled(
            format!("{:+.2}% 24h", c * 100.0),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Price panel: large current price + tiny inline range summary.
pub fn render_price(
    frame: &mut Frame,
    area: Rect,
    entry: Option<&DirectoryEntry>,
    price: Option<f64>,
    change_24h: Option<f64>,
    convert: &str,
    colors: &ThemeColors,
) {
    let block = panel_block("Price", colors);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        entry
            .map(|e| format!("{} / {}", e.symbol, convert))
            .unwrap_or_else(|| "—".into()),
        Style::default().fg(colors.muted),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        price
            .map(|p| format!("${}", format_price(p)))
            .unwrap_or_else(|| "…".into()),
        Style::default().fg(colors.fg).add_modifier(Modifier::BOLD),
    )));
    if let Some(c) = change_24h {
        let color = if c >= 0.0 {
            colors.positive
        } else {
            colors.negative
        };
        lines.push(Line::from(Span::styled(
            format!("{:+.2}% 24h", c * 100.0),
            Style::default().fg(color),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Ranges panel: high/low for each period returned by `statistics`.
pub fn render_ranges(
    frame: &mut Frame,
    area: Rect,
    stats: Option<&StatisticsResponse>,
    convert: &str,
    colors: &ThemeColors,
) {
    let title = format!("Ranges ({})", convert);
    let block = panel_block(&title, colors);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(stats) = stats else {
        frame.render_widget(
            Paragraph::new("Loading…").style(Style::default().fg(colors.muted)),
            inner,
        );
        return;
    };

    // Preferred order — statistics may include or omit any of these.
    let order = ["1d", "7d", "30d", "90d", "180d", "365d", "ytd", "all"];
    let mut rows: Vec<Row> = Vec::new();
    for key in order {
        if let Some(s) = stats.statistics.iter().find(|p| p.period == key) {
            rows.push(Row::new(vec![
                Cell::from(Span::styled(
                    format!(" {}", s.period),
                    Style::default().fg(colors.header),
                )),
                Cell::from(Span::styled(
                    format_price(s.low),
                    Style::default().fg(colors.negative),
                )),
                Cell::from(Span::styled(
                    format_price(s.high),
                    Style::default().fg(colors.positive),
                )),
            ]));
        }
    }

    let widths = [
        Constraint::Length(6),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ];
    let header = Row::new(vec!["", "Low", "High"]).style(Style::default().fg(colors.muted));
    let table = Table::new(rows, widths).header(header).column_spacing(1);
    frame.render_widget(table, inner);
}

/// Social metrics panel: follower counts with simple magnitude bars.
pub fn render_social(
    frame: &mut Frame,
    area: Rect,
    metrics: Option<&SocialMetrics>,
    colors: &ThemeColors,
) {
    let block = panel_block("Social", colors);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(m) = metrics else {
        frame.render_widget(
            Paragraph::new("Loading…").style(Style::default().fg(colors.muted)),
            inner,
        );
        return;
    };

    let reddit = m.reddit.as_ref().and_then(|p| p.followers);
    let twitter = m.twitter.as_ref().and_then(|p| p.followers);
    let telegram = m.telegram.as_ref().and_then(|p| p.followers);

    // Scale bars against the largest value so relative size reads naturally.
    let max = [reddit, twitter, telegram]
        .iter()
        .filter_map(|v| *v)
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    fn row<'a>(label: &'a str, value: Option<i64>, max: f64, colors: &ThemeColors) -> Line<'a> {
        match value {
            None => Line::from(vec![
                Span::styled(format!(" {:<9}", label), Style::default().fg(colors.header)),
                Span::styled("—", Style::default().fg(colors.muted)),
            ]),
            Some(v) => {
                let ratio = (v as f64 / max).clamp(0.0, 1.0);
                let width = (ratio * 8.0).round() as usize;
                let bar: String = std::iter::repeat_n('\u{2588}', width).collect();
                let bar_rest: String = std::iter::repeat_n('\u{2591}', 8 - width).collect();
                Line::from(vec![
                    Span::styled(format!(" {:<9}", label), Style::default().fg(colors.header)),
                    Span::styled(
                        format!("{:>7} ", format_compact(v as f64)),
                        Style::default().fg(colors.fg),
                    ),
                    Span::styled(bar, Style::default().fg(colors.accent)),
                    Span::styled(bar_rest, Style::default().fg(colors.muted)),
                ])
            }
        }
    }

    let lines = vec![
        row("Reddit", reddit, max, colors),
        row("X/Twitter", twitter, max, colors),
        row("Telegram", telegram, max, colors),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Exchange listings panel: top N by 24h USD volume, highlight Crypto.com.
pub fn render_listings(
    frame: &mut Frame,
    area: Rect,
    pairs: Option<&[MarketPair]>,
    limit: usize,
    colors: &ThemeColors,
) {
    let block = panel_block("Listed Exchanges (top by USD volume)", colors);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(pairs) = pairs else {
        frame.render_widget(
            Paragraph::new("Loading…").style(Style::default().fg(colors.muted)),
            inner,
        );
        return;
    };

    let mut sorted: Vec<&MarketPair> = pairs.iter().collect();
    sorted.sort_by(|a, b| {
        b.quote_usd_volume_24h
            .unwrap_or(0.0)
            .partial_cmp(&a.quote_usd_volume_24h.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let header = Row::new(vec!["Exchange", "Pair", "Price", "24h Vol", ""])
        .style(Style::default().fg(colors.muted));
    let widths = [
        Constraint::Length(18),
        Constraint::Length(14),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(3),
    ];

    let rows: Vec<Row> = sorted
        .iter()
        .take(limit)
        .map(|p| {
            let name = p.exchange_name.clone().unwrap_or_else(|| "—".into());
            let is_cdc = name.to_lowercase().contains("crypto.com");
            let row_style = if is_cdc {
                Style::default()
                    .fg(colors.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.fg)
            };
            Row::new(vec![
                Cell::from(truncate(&name, 18)),
                Cell::from(p.market_pair_name.clone().unwrap_or_default()),
                Cell::from(
                    p.quote_usd_price
                        .map(format_price)
                        .unwrap_or_else(|| "—".into()),
                ),
                Cell::from(
                    p.quote_usd_volume_24h
                        .map(format_compact)
                        .unwrap_or_else(|| "—".into()),
                ),
                Cell::from(if is_cdc { "\u{2605}" } else { "" }),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(rows, widths).header(header).column_spacing(1);
    frame.render_widget(table, inner);
}

/// Trending panel: small vertical list of globally-trending tokens.
pub fn render_trending(
    frame: &mut Frame,
    area: Rect,
    trending: Option<&[TrendingToken]>,
    colors: &ThemeColors,
) {
    let block = panel_block("Trending", colors);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(trending) = trending else {
        frame.render_widget(
            Paragraph::new("Loading…").style(Style::default().fg(colors.muted)),
            inner,
        );
        return;
    };

    let lines: Vec<Line> = trending
        .iter()
        .take(inner.height as usize)
        .map(|t| {
            let color = if t.usd_price_change_24h >= 0.0 {
                colors.positive
            } else {
                colors.negative
            };
            Line::from(vec![
                Span::styled(
                    format!(" {:<8}", t.symbol),
                    Style::default().fg(colors.header),
                ),
                Span::styled(
                    format!("{:+6.2}%  ", t.usd_price_change_24h * 100.0),
                    Style::default().fg(color),
                ),
                Span::styled(truncate(&t.name, 16), Style::default().fg(colors.muted)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// News panel with Reddit / Video / Highlights sub-tabs.
///
/// `selected` is a row index into the active sub-tab's list (clamped by the
/// caller). `expanded` toggles inline rendering of the selected item's body
/// / description / URL — the expansion sits below the highlighted row rather
/// than replacing the list.
#[allow(clippy::too_many_arguments)]
pub fn render_news(
    frame: &mut Frame,
    area: Rect,
    active: NewsPanel,
    reddit: Option<&[RedditPost]>,
    videos: Option<&[VideoNews]>,
    selected: usize,
    expanded: bool,
    colors: &ThemeColors,
) {
    let footer = if expanded {
        "J/K:nav  Enter:collapse  b:open"
    } else {
        "J/K:nav  Enter:expand  b:open  N:cycle"
    };
    let title = format!("News  [{}]  \u{2190} {}", footer, active.label());
    let block = panel_block(&title, colors);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [sub_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);

    // Sub-tab strip
    let sub_spans: Vec<Span> = [NewsPanel::Highlights, NewsPanel::Reddit, NewsPanel::Video]
        .iter()
        .enumerate()
        .flat_map(|(i, kind)| {
            let style = if *kind == active {
                Style::default()
                    .fg(colors.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.muted)
            };
            let mut out = Vec::new();
            if i > 0 {
                out.push(Span::styled(" │ ", Style::default().fg(colors.muted)));
            }
            out.push(Span::styled(kind.label().to_string(), style));
            out
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(sub_spans)), sub_area);

    match active {
        NewsPanel::Reddit => {
            render_reddit_list(frame, list_area, reddit, selected, expanded, colors)
        }
        NewsPanel::Video => render_video_list(frame, list_area, videos, selected, expanded, colors),
        NewsPanel::Highlights => {
            render_highlights(frame, list_area, reddit, videos, selected, colors)
        }
    }
}

/// Compute which item index starts at the top of the visible viewport so the
/// `selected` item stays on-screen. Keeps the cursor at least one row from
/// the edges when possible to preserve context.
fn viewport_start(selected: usize, total: usize, visible: usize) -> usize {
    if total <= visible || selected < visible.saturating_sub(1) {
        return 0;
    }
    selected + 1 - visible
}

/// Render the inline-expanded preview for a Reddit post directly after the
/// selected row. Includes author, score, body (if any), and source URL.
fn reddit_expansion_lines(
    post: &RedditPost,
    max_width: usize,
    colors: &ThemeColors,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    // Meta line: author · score · time
    let meta = format!(
        " u/{}   {} upvotes   {}",
        post.username.clone().unwrap_or_else(|| "—".into()),
        post.upvotes,
        post.create_time.clone().unwrap_or_default()
    );
    out.push(Line::from(Span::styled(
        meta,
        Style::default().fg(colors.muted),
    )));

    // Body — wrap manually to max_width so the Paragraph doesn't re-wrap
    // inside an expansion that already has bounded width.
    let body = post
        .text
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or("(no body text)");
    for chunk in wrap_lines(body, max_width.saturating_sub(2)) {
        out.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(chunk, Style::default().fg(colors.fg)),
        ]));
    }

    // Source URL. Prefer external link (news-sharing posts) otherwise the
    // comment thread.
    let link = post.link.clone().or_else(|| {
        post.url
            .as_ref()
            .map(|u| format!("https://reddit.com{}", u))
    });
    if let Some(link) = link {
        out.push(Line::from(vec![
            Span::styled(" \u{2192} ", Style::default().fg(colors.accent)),
            Span::styled(
                truncate(&link, max_width.saturating_sub(4)),
                Style::default().fg(colors.muted),
            ),
        ]));
    }
    out
}

fn video_expansion_lines(
    v: &VideoNews,
    max_width: usize,
    colors: &ThemeColors,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let meta = format!(" {}", v.create_time.clone().unwrap_or_default());
    out.push(Line::from(Span::styled(
        meta,
        Style::default().fg(colors.muted),
    )));

    let body = v
        .description
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or("(no description)");
    for chunk in wrap_lines(body, max_width.saturating_sub(2)) {
        out.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(chunk, Style::default().fg(colors.fg)),
        ]));
    }

    let url = format!("https://youtube.com/watch?v={}", v.id);
    out.push(Line::from(vec![
        Span::styled(" \u{2192} ", Style::default().fg(colors.accent)),
        Span::styled(
            truncate(&url, max_width.saturating_sub(4)),
            Style::default().fg(colors.muted),
        ),
    ]));
    out
}

/// Hard-wrap `text` to `max_width` characters per line. Soft-wraps at
/// whitespace; falls back to character split for long unbroken runs.
fn wrap_lines(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return Vec::new();
    }
    // Paragraph breaks on blank lines — preserve those.
    let mut out = Vec::new();
    for para in text.split("\n\n") {
        let flattened = para.replace('\n', " ");
        let mut line = String::new();
        for word in flattened.split_whitespace() {
            if line.is_empty() {
                // Long single word: hard-split.
                if word.chars().count() > max_width {
                    let mut remaining = word;
                    while remaining.chars().count() > max_width {
                        let piece: String = remaining.chars().take(max_width).collect();
                        out.push(piece);
                        remaining = &remaining[remaining
                            .char_indices()
                            .nth(max_width)
                            .map(|(i, _)| i)
                            .unwrap_or(remaining.len())..];
                    }
                    line.push_str(remaining);
                } else {
                    line.push_str(word);
                }
            } else if line.chars().count() + 1 + word.chars().count() <= max_width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
        // Blank separator between paragraphs (but not after the last one).
    }
    // Cap total lines so a wall-of-text post doesn't consume the whole panel.
    out.truncate(12);
    out
}

fn render_reddit_list(
    frame: &mut Frame,
    area: Rect,
    reddit: Option<&[RedditPost]>,
    selected: usize,
    expanded: bool,
    colors: &ThemeColors,
) {
    let Some(posts) = reddit else {
        frame.render_widget(
            Paragraph::new("Loading…").style(Style::default().fg(colors.muted)),
            area,
        );
        return;
    };
    if posts.is_empty() {
        frame.render_widget(
            Paragraph::new("No posts").style(Style::default().fg(colors.muted)),
            area,
        );
        return;
    }

    let visible = area.height as usize;
    let start = viewport_start(selected, posts.len(), visible);
    let mut lines: Vec<Line> = Vec::with_capacity(visible);

    for (idx, p) in posts.iter().enumerate().skip(start).take(visible) {
        let is_sel = idx == selected;
        let row_style = if is_sel {
            Style::default()
                .fg(colors.selected_fg)
                .bg(colors.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors.fg)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{}{:>5} ",
                    if is_sel { "\u{25B6} " } else { "  " },
                    compact_time(p.create_time.as_deref())
                ),
                row_style,
            ),
            Span::styled(
                format!("\u{25B2}{:<4} ", p.upvotes),
                Style::default().fg(colors.positive),
            ),
            Span::styled(
                truncate(&p.title, area.width.saturating_sub(22) as usize),
                row_style,
            ),
        ]));
        // Inline expansion: attach detail lines under the selected row, then
        // let the rest of the list continue below.
        if is_sel && expanded {
            lines.extend(reddit_expansion_lines(p, area.width as usize, colors));
        }
    }

    // Wrap disabled — selection highlight relies on each row occupying
    // exactly one terminal line. Wrap would offset indices.
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_video_list(
    frame: &mut Frame,
    area: Rect,
    videos: Option<&[VideoNews]>,
    selected: usize,
    expanded: bool,
    colors: &ThemeColors,
) {
    let Some(videos) = videos else {
        frame.render_widget(
            Paragraph::new("Loading…").style(Style::default().fg(colors.muted)),
            area,
        );
        return;
    };
    if videos.is_empty() {
        frame.render_widget(
            Paragraph::new("No videos").style(Style::default().fg(colors.muted)),
            area,
        );
        return;
    }

    let visible = area.height as usize;
    let start = viewport_start(selected, videos.len(), visible);
    let mut lines: Vec<Line> = Vec::with_capacity(visible);

    for (idx, v) in videos.iter().enumerate().skip(start).take(visible) {
        let is_sel = idx == selected;
        let row_style = if is_sel {
            Style::default()
                .fg(colors.selected_fg)
                .bg(colors.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors.fg)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{}{:>5} ",
                    if is_sel { "\u{25B6} " } else { "  " },
                    compact_time(v.create_time.as_deref())
                ),
                row_style,
            ),
            Span::styled("\u{25B6} ", Style::default().fg(colors.accent)),
            Span::styled(
                truncate(&v.title, area.width.saturating_sub(14) as usize),
                row_style,
            ),
        ]));
        if is_sel && expanded {
            lines.extend(video_expansion_lines(v, area.width as usize, colors));
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_highlights(
    frame: &mut Frame,
    area: Rect,
    reddit: Option<&[RedditPost]>,
    videos: Option<&[VideoNews]>,
    selected: usize,
    colors: &ThemeColors,
) {
    // Highlights: top 5 reddit posts (by upvotes) + top 3 videos, interleaved
    // by create_time. No inline expansion here — the merged list makes URL
    // resolution ambiguous; users press N to drill into Reddit or Video
    // sub-tabs for expansion and opening.
    let mut items: Vec<(String, Line)> = Vec::new();
    if let Some(posts) = reddit {
        let mut top = posts.iter().collect::<Vec<_>>();
        top.sort_by_key(|p| -p.upvotes);
        for p in top.into_iter().take(5) {
            let time = p.create_time.clone().unwrap_or_default();
            let line = Line::from(vec![
                Span::styled(" r/ ", Style::default().fg(colors.accent)),
                Span::styled(
                    format!("\u{25B2}{:<4} ", p.upvotes),
                    Style::default().fg(colors.positive),
                ),
                Span::styled(
                    truncate(&p.title, area.width.saturating_sub(14) as usize),
                    Style::default().fg(colors.fg),
                ),
            ]);
            items.push((time, line));
        }
    }
    if let Some(vs) = videos {
        for v in vs.iter().take(3) {
            let time = v.create_time.clone().unwrap_or_default();
            let line = Line::from(vec![
                Span::styled(" \u{25B6}  ", Style::default().fg(colors.accent)),
                Span::styled(
                    truncate(&v.title, area.width.saturating_sub(8) as usize),
                    Style::default().fg(colors.fg),
                ),
            ]);
            items.push((time, line));
        }
    }

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("Loading…").style(Style::default().fg(colors.muted)),
            area,
        );
        return;
    }

    // Newest first.
    items.sort_by(|a, b| b.0.cmp(&a.0));
    let visible = area.height as usize;
    let start = viewport_start(selected, items.len(), visible);
    let lines: Vec<Line> = items
        .into_iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(idx, (_, line))| {
            if idx == selected {
                // Prepend a selection marker without mutating the stored line.
                let mut spans = line.spans;
                // Replace the leading spacer with a marker.
                if let Some(first) = spans.first_mut() {
                    *first = Span::styled(
                        "\u{25B6}".to_string(),
                        Style::default()
                            .fg(colors.accent)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                Line::from(spans)
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// Convert an ISO-ish timestamp to `HH:MM` for compact display. Falls back to
/// the raw string if parsing fails.
fn compact_time(ts: Option<&str>) -> String {
    let Some(ts) = ts else {
        return "—".into();
    };
    // Expected shape: "2026-04-23T11:55:23" — pull out HH:MM directly.
    ts.split('T')
        .nth(1)
        .and_then(|t| t.get(..5))
        .unwrap_or(ts)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_appends_ellipsis() {
        assert_eq!(truncate("hello world", 8), "hello w\u{2026}");
    }

    #[test]
    fn compact_time_extracts_hhmm() {
        assert_eq!(compact_time(Some("2026-04-23T11:55:23")), "11:55");
        assert_eq!(compact_time(None), "—");
        assert_eq!(compact_time(Some("garbage")), "garbage");
    }
}
