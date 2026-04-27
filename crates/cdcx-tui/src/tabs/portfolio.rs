use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::state::{AppState, RestRequest};
use crate::tabs::{DataEvent, Tab};

// Unified view model — both live and paper populate this
#[derive(Debug, Clone)]
struct Holding {
    name: String,
    amount: f64,
    available: f64,
    value: f64, // market value in quote currency
}

fn is_stablecoin(symbol: &str) -> bool {
    matches!(symbol, "USDT" | "USD" | "USDC" | "DAI" | "TUSD" | "BUSD")
}

/// Parse a single balance record into the static view fields. `value` is
/// left at zero and filled by `recompute_holding_values` so the ticker-
/// driven value recompute has a single writer (same pattern as Positions).
fn parse_balance_record(item: &serde_json::Value) -> Option<Holding> {
    let currency = item
        .get("instrument_name")
        .or_else(|| item.get("currency"))
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();

    let amount = item
        .get("total_cash_balance")
        .or_else(|| item.get("quantity"))
        .or_else(|| item.get("balance"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    if amount <= 0.0 {
        return None;
    }

    let available = item
        .get("total_available_balance")
        .or_else(|| item.get("available"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    Some(Holding {
        name: currency,
        amount,
        available,
        value: 0.0,
    })
}

/// Re-price every holding from `state.tickers`. Stablecoins are valued 1:1;
/// non-stable holdings use `{SYM}_USDT` last ask. Called from REST, WS, and
/// tick paths so holdings stay live between `user.balance` deltas (which
/// only fire on trades/deposits, not on market price moves).
fn recompute_holding_values(holdings: &mut [Holding], state: &AppState) -> (f64, f64) {
    let mut cash_balance = 0.0;
    let mut position_value = 0.0;
    for h in holdings {
        if is_stablecoin(&h.name) {
            h.value = h.amount;
            cash_balance += h.value;
            continue;
        }
        let pair = format!("{}_USDT", h.name);
        let last = state.tickers.get(&pair).map(|t| t.ask).unwrap_or(0.0);
        h.value = h.amount * last;
        position_value += h.value;
    }
    (cash_balance, position_value)
}

#[derive(Debug, Clone, Default)]
struct PortfolioView {
    holdings: Vec<Holding>,
    cash_balance: f64,
    position_value: f64,
    total_value: f64,
    initial_value: f64,
    unrealized_pnl: f64,
    realized_pnl: f64,
}

pub struct PortfolioTab {
    view: PortfolioView,
    loaded: bool,
    selected: usize,
}

impl Default for PortfolioTab {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioTab {
    pub fn new() -> Self {
        Self {
            view: PortfolioView::default(),
            loaded: false,
            selected: 0,
        }
    }

    fn request_data(&self, state: &AppState) {
        if state.authenticated && !state.paper_mode {
            let _ = state.rest_tx.send(RestRequest {
                method: "private/user-balance".into(),
                params: serde_json::json!({}),
                is_private: true,
            });
        }
    }

    /// Shared between REST response and `user.balance` WS snapshot. Rebuilds
    /// holdings from records, then runs the ticker-driven value recompute.
    fn rebuild_from_records(&mut self, records: &[serde_json::Value], state: &mut AppState) {
        let mut holdings: Vec<Holding> = records.iter().filter_map(parse_balance_record).collect();
        let (cash_balance, position_value) = recompute_holding_values(&mut holdings, state);
        holdings.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap());

        let total_value = cash_balance + position_value;
        if state.session_start_value.is_none() && total_value > 0.0 {
            state.session_start_value = Some(total_value);
        }
        state.current_portfolio_value = total_value;

        self.view = PortfolioView {
            holdings,
            cash_balance,
            position_value,
            total_value,
            initial_value: state.session_start_value.unwrap_or(total_value),
            unrealized_pnl: 0.0,
            realized_pnl: 0.0,
        };
        self.loaded = true;
    }

    /// Re-run the ticker-based value pass on the existing holdings without
    /// touching the payload-sourced amount/available fields. Called each
    /// tick so values follow the market between `user.balance` deltas.
    fn refresh_live_values(&mut self, state: &mut AppState) {
        if self.view.holdings.is_empty() {
            return;
        }
        let (cash_balance, position_value) =
            recompute_holding_values(&mut self.view.holdings, state);
        self.view.cash_balance = cash_balance;
        self.view.position_value = position_value;
        self.view.total_value = cash_balance + position_value;
        state.current_portfolio_value = self.view.total_value;
        self.view
            .holdings
            .sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap());
    }

    fn build_paper_view(state: &AppState) -> PortfolioView {
        let Some(ref engine) = state.paper_engine else {
            return PortfolioView::default();
        };
        let s = &engine.state;

        let mut holdings = Vec::new();
        let mut position_value = 0.0;
        let mut unrealized_pnl = 0.0;
        let mut realized_pnl = 0.0;

        for p in &s.positions {
            if p.quantity.abs() < 1e-12 {
                realized_pnl += p.realized_pnl;
                continue;
            }
            let mark = state
                .tickers
                .get(&p.instrument_name)
                .map(|t| t.ask)
                .unwrap_or(p.avg_entry_price);
            let val = p.quantity.abs() * mark;
            position_value += val;

            let unreal = if p.quantity > 0.0 {
                (mark - p.avg_entry_price) * p.quantity
            } else {
                (p.avg_entry_price - mark) * p.quantity.abs()
            };
            unrealized_pnl += unreal;
            realized_pnl += p.realized_pnl;

            // Extract base currency from instrument (BTC_USDT → BTC)
            let base = p
                .instrument_name
                .split('_')
                .next()
                .unwrap_or(&p.instrument_name);
            holdings.push(Holding {
                name: base.to_string(),
                amount: p.quantity.abs(),
                available: p.quantity.abs(),
                value: val,
            });
        }

        // Add cash as a holding
        if s.balance > 0.01 {
            holdings.insert(
                0,
                Holding {
                    name: "USD (cash)".into(),
                    amount: s.balance,
                    available: s.balance,
                    value: s.balance,
                },
            );
        }

        let total_value = s.balance + position_value;

        PortfolioView {
            holdings,
            cash_balance: s.balance,
            position_value,
            total_value,
            initial_value: s.initial_balance,
            unrealized_pnl,
            realized_pnl,
        }
    }
}

impl Tab for PortfolioTab {
    fn on_key(&mut self, key: KeyEvent, _state: &mut AppState) -> bool {
        match key.code {
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                true
            }
            KeyCode::Down => {
                if self.selected < self.view.holdings.len().saturating_sub(1) {
                    self.selected += 1;
                }
                true
            }
            KeyCode::Char('r') => {
                self.loaded = false;
                true
            }
            _ => false,
        }
    }

    fn on_data(&mut self, event: &DataEvent, state: &mut AppState) {
        // Paper mode: rebuild view from engine on every tick
        if state.paper_mode {
            self.view = Self::build_paper_view(state);
            self.loaded = true;
            return;
        }

        match event {
            DataEvent::RestResponse { method, data } if method == "private/user-balance" => {
                if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                    self.rebuild_from_records(arr, state);
                }
            }
            DataEvent::BalanceSnapshot(records) => {
                self.rebuild_from_records(records, state);
            }
            _ => {
                if !self.loaded {
                    self.request_data(state);
                    return;
                }
                // Keep non-stable holding values tracking the market between
                // `user.balance` deltas (which only fire on trades/deposits).
                self.refresh_live_values(state);
            }
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        if !state.paper_mode && !state.authenticated {
            frame.render_widget(
                Paragraph::new(
                    "Portfolio \u{2014} not authenticated. Set CDC_API_KEY and CDC_API_SECRET.",
                )
                .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        if !self.loaded {
            frame.render_widget(
                Paragraph::new("Portfolio \u{2014} loading...")
                    .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        let v = &self.view;

        if v.holdings.is_empty() {
            frame.render_widget(
                Paragraph::new("Portfolio \u{2014} no holdings.")
                    .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        let [summary_area, table_area, footer_area] = Layout::vertical([
            Constraint::Length(6),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);

        // Summary header — shown for both live and paper
        let total_pnl = v.total_value - v.initial_value;
        let pnl_color = if total_pnl >= 0.0 {
            state.theme.colors.positive
        } else {
            state.theme.colors.negative
        };

        let mut summary_lines = vec![
            Line::from(vec![
                Span::styled(
                    "  Total Value: ",
                    Style::default().fg(state.theme.colors.muted),
                ),
                Span::styled(
                    format!("${:.2}", v.total_value),
                    Style::default()
                        .fg(state.theme.colors.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Cash: ", Style::default().fg(state.theme.colors.muted)),
                Span::styled(
                    format!("${:.2}", v.cash_balance),
                    Style::default().fg(state.theme.colors.fg),
                ),
                Span::styled(
                    "  Positions: ",
                    Style::default().fg(state.theme.colors.muted),
                ),
                Span::styled(
                    format!("${:.2}", v.position_value),
                    Style::default().fg(state.theme.colors.volume),
                ),
            ]),
        ];

        if state.paper_mode && v.initial_value > 0.0 {
            let unreal_color = if v.unrealized_pnl >= 0.0 {
                state.theme.colors.positive
            } else {
                state.theme.colors.negative
            };
            summary_lines.push(Line::from(vec![
                Span::styled("  P&L: ", Style::default().fg(state.theme.colors.muted)),
                Span::styled(
                    format!("${:+.2}", total_pnl),
                    Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  Unrealized: ",
                    Style::default().fg(state.theme.colors.muted),
                ),
                Span::styled(
                    format!("${:+.2}", v.unrealized_pnl),
                    Style::default().fg(unreal_color),
                ),
                Span::styled(
                    "  Realized: ",
                    Style::default().fg(state.theme.colors.muted),
                ),
                Span::styled(
                    format!("${:+.2}", v.realized_pnl),
                    Style::default().fg(state.theme.colors.fg),
                ),
            ]));
        } else if !state.paper_mode && v.initial_value > 0.0 {
            summary_lines.push(Line::from(vec![
                Span::styled(
                    "  Session P&L: ",
                    Style::default().fg(state.theme.colors.muted),
                ),
                Span::styled(
                    format!("${:+.2}", total_pnl),
                    Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "  ({:+.1}%)",
                        if v.initial_value > 0.0 {
                            total_pnl / v.initial_value * 100.0
                        } else {
                            0.0
                        }
                    ),
                    Style::default().fg(pnl_color),
                ),
            ]));
        }

        summary_lines.push(Line::from(""));

        frame.render_widget(Paragraph::new(summary_lines), summary_area);

        // Holdings table
        let header = Row::new(vec![
            Cell::from("Asset"),
            Cell::from("Amount"),
            Cell::from("Available"),
            Cell::from("Value (USD)"),
            Cell::from("Allocation"),
        ])
        .style(
            Style::default()
                .fg(state.theme.colors.header)
                .add_modifier(Modifier::BOLD),
        );

        let widths = [
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Length(20),
        ];

        let rows: Vec<Row> = v
            .holdings
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let is_selected = i == self.selected;
                let pct = if v.total_value > 0.0 {
                    h.value / v.total_value * 100.0
                } else {
                    0.0
                };
                let bar_len = (pct / 5.0).round() as usize;
                let bar = "\u{2588}".repeat(bar_len);

                let row_style = if is_selected {
                    Style::default()
                        .fg(state.theme.colors.selected_fg)
                        .bg(state.theme.colors.selected_bg)
                } else {
                    Style::default().fg(state.theme.colors.fg)
                };

                if is_selected {
                    Row::new(vec![
                        Cell::from(h.name.as_str()),
                        Cell::from(format!("{:.4}", h.amount)),
                        Cell::from(format!("{:.4}", h.available)),
                        Cell::from(format!("${:.2}", h.value)),
                        Cell::from(format!("{} {:.1}%", bar, pct)),
                    ])
                    .style(row_style)
                } else {
                    Row::new(vec![
                        Cell::from(h.name.as_str()),
                        Cell::from(format!("{:.4}", h.amount)),
                        Cell::from(format!("{:.4}", h.available)),
                        Cell::from(format!("${:.2}", h.value)),
                        Cell::from(format!("{} {:.1}%", bar, pct))
                            .style(Style::default().fg(state.theme.colors.volume)),
                    ])
                    .style(row_style)
                }
            })
            .collect();

        let table = Table::new(rows, widths).header(header).column_spacing(1);
        frame.render_widget(table, table_area);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "r:refresh  \u{2191}\u{2193}:navigate",
                Style::default().fg(state.theme.colors.muted),
            ))),
            footer_area,
        );
    }

    fn subscriptions(&self, _state: &AppState) -> Vec<String> {
        vec![]
    }

    fn on_activate(&mut self) {
        // Intentionally no-op: state stays fresh via user.balance WS +
        // per-tick ticker-driven value recompute, so wiping the cache on
        // tab-switch only produces a blank frame before REST refills.
    }
}
