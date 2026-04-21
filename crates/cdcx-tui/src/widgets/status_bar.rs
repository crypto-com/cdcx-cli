use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{AppState, ConnectionStatus, ToastStyle};

pub fn draw_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let conn_label = match state.market_connection {
        ConnectionStatus::Connected => " LIVE ",
        ConnectionStatus::Connecting => " CONNECTING ",
        ConnectionStatus::Reconnecting => " RECONNECTING ",
        ConnectionStatus::Error => " DISCONNECTED ",
    };
    let conn_color = match state.market_connection {
        ConnectionStatus::Connected => state.theme.colors.positive,
        ConnectionStatus::Connecting => state.theme.colors.volume,
        ConnectionStatus::Reconnecting => state.theme.colors.accent,
        ConnectionStatus::Error => state.theme.colors.negative,
    };

    let mut spans = vec![
        Span::styled(
            format!(" {} ", state.env_label()),
            Style::default()
                .fg(state.theme.colors.status_bar_fg)
                .bg(state.theme.colors.status_bar_bg),
        ),
        Span::raw(" "),
        Span::styled(conn_label, Style::default().fg(conn_color)),
        Span::raw("  "),
        if state.paper_mode {
            Span::styled(
                " PAPER ",
                Style::default()
                    .fg(Color::Rgb(0, 0, 0))
                    .bg(Color::Rgb(255, 180, 0)),
            )
        } else {
            Span::raw("")
        },
        Span::raw(if state.paper_mode { " " } else { "" }),
    ];

    // Show toast if active, otherwise show default hints
    if let Some(toast) = state.active_toast() {
        let toast_color = match toast.style {
            ToastStyle::Info => state.theme.colors.accent,
            ToastStyle::Success => state.theme.colors.positive,
            ToastStyle::Error => state.theme.colors.negative,
        };
        spans.push(Span::styled(
            format!(" {} ", toast.message),
            Style::default()
                .fg(toast_color)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        // Paper mode balance or live session P&L
        if state.paper_mode {
            if let Some(ref engine) = state.paper_engine {
                // Total value = cash + position market value
                let mut pos_value = 0.0;
                for p in &engine.state.positions {
                    if p.quantity.abs() < 1e-12 {
                        continue;
                    }
                    let mark = state
                        .tickers
                        .get(&p.instrument_name)
                        .map(|t| t.ask)
                        .unwrap_or(p.avg_entry_price);
                    pos_value += p.quantity.abs() * mark;
                }
                let total = engine.state.balance + pos_value;
                let pnl = total - engine.state.initial_balance;
                let pnl_color = if pnl >= 0.0 {
                    state.theme.colors.positive
                } else {
                    state.theme.colors.negative
                };
                spans.push(Span::styled(
                    format!("${:.2} ({:+.2}) ", total, pnl),
                    Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
            }
        } else if state.current_portfolio_value > 0.0 {
            let total = state.current_portfolio_value;
            if let Some(start_val) = state.session_start_value {
                let pnl = total - start_val;
                let pnl_color = if pnl >= 0.0 {
                    state.theme.colors.positive
                } else {
                    state.theme.colors.negative
                };
                spans.push(Span::styled(
                    format!("${:.2} ({:+.2}) ", total, pnl),
                    Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!("${:.2} ", total),
                    Style::default()
                        .fg(state.theme.colors.accent)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("Theme: {}", state.theme.name),
            Style::default().fg(state.theme.colors.muted),
        ));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "q:quit  Tab:switch  v:volume unit  ,:settings  ?:help",
            Style::default().fg(state.theme.colors.muted),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(state.theme.colors.status_bar_bg)),
        area,
    );
}
