use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::state::{AppState, RestRequest};
use crate::tabs::{DataEvent, Tab};

#[derive(Debug, Clone)]
struct Position {
    instrument: String,
    side: String,
    quantity: f64,
    entry_price: f64,
    mark_price: f64,
    pnl: f64,
    liquidation_price: f64,
}

pub struct PositionsTab {
    positions: Vec<Position>,
    loaded: bool,
    selected: usize,
    detail_position: Option<usize>,
}

impl Default for PositionsTab {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionsTab {
    pub fn new() -> Self {
        Self {
            positions: vec![],
            loaded: false,
            selected: 0,
            detail_position: None,
        }
    }

    fn load_paper_positions(&mut self, state: &AppState) {
        if let Some(ref engine) = state.paper_engine {
            self.positions = engine
                .state
                .positions
                .iter()
                .filter(|p| p.quantity.abs() > 1e-12)
                .map(|p| {
                    let mark = state
                        .tickers
                        .get(&p.instrument_name)
                        .map(|t| t.ask)
                        .unwrap_or(p.avg_entry_price);
                    let direction = if p.quantity > 0.0 { 1.0 } else { -1.0 };
                    let pnl = (mark - p.avg_entry_price) * p.quantity.abs() * direction;
                    Position {
                        instrument: p.instrument_name.clone(),
                        side: if p.quantity > 0.0 {
                            "LONG".into()
                        } else {
                            "SHORT".into()
                        },
                        quantity: p.quantity.abs(),
                        entry_price: p.avg_entry_price,
                        mark_price: mark,
                        pnl,
                        liquidation_price: 0.0,
                    }
                })
                .collect();
            self.positions.sort_by(|a, b| a.instrument.cmp(&b.instrument));
            self.loaded = true;
        }
    }

    fn request_data(&self, state: &AppState) {
        if state.authenticated && !state.paper_mode {
            let _ = state.rest_tx.send(RestRequest {
                method: "private/get-positions".into(),
                params: serde_json::json!({}),
                is_private: true,
            });
        }
    }

    /// Rebuild the positions list from a raw records array — shared between
    /// the REST response arm and the `user.positions` WS snapshot arm so the
    /// field-parsing heuristics stay in one place.
    fn rebuild_positions(&mut self, records: &[serde_json::Value], state: &AppState) {
        self.positions = records
            .iter()
            .filter_map(|item| parse_position_record(item, state))
            .collect();
        self.positions.sort_by(|a, b| a.instrument.cmp(&b.instrument));
        // Immediately apply the same ticker-driven recompute the tick path uses,
        // otherwise the WS payload's zero-initialised mark/pnl flashes on screen
        // for one frame before the next tick fills it in.
        apply_live_mark_and_pnl(&mut self.positions, state);
        if self.selected >= self.positions.len() {
            self.selected = self.positions.len().saturating_sub(1);
        }
        self.loaded = true;
    }
}

/// Overwrite `mark_price` and `pnl` on every position from the streaming
/// ticker cache. Single source of truth for live values — called from both
/// `rebuild_positions` (WS snapshot / REST response) and the tick fallthrough
/// to avoid flicker between conflicting writers.
fn apply_live_mark_and_pnl(positions: &mut [Position], state: &AppState) {
    for pos in positions {
        let Some(ticker) = state.tickers.get(&pos.instrument) else {
            continue;
        };
        pos.mark_price = ticker.ask;
        if pos.entry_price > 0.0 {
            let direction = if pos.side == "BUY" || pos.side == "LONG" {
                1.0
            } else {
                -1.0
            };
            pos.pnl = (pos.mark_price - pos.entry_price) * pos.quantity * direction;
        }
    }
}

/// Parse a single position record from the exchange into the row struct.
/// Only static fields (instrument / side / qty / entry / liq) are read here;
/// `mark_price` and `pnl` are owned exclusively by `apply_live_mark_and_pnl`
/// so we have a single writer and no per-frame flicker. Returns `None` for
/// zero-quantity rows — the exchange echoes closed positions.
fn parse_position_record(item: &serde_json::Value, _state: &AppState) -> Option<Position> {
    let instrument = item
        .get("instrument_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if instrument.is_empty() {
        return None;
    }

    // The exchange encodes direction in the sign of `quantity`
    // (positive = long, negative = short) and does NOT send an
    // explicit `side` field on positions. Fall back to the
    // quantity-sign convention when `side` is absent.
    let raw_qty: f64 = item
        .get("quantity")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    if raw_qty.abs() < 1e-12 {
        return None;
    }
    let side = item
        .get("side")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if raw_qty >= 0.0 {
                "LONG".into()
            } else {
                "SHORT".into()
            }
        });

    // The exchange returns `open_pos_cost` (total USD notional) rather than
    // a per-unit `average_price`. Divide to recover the entry price. Fall
    // back to `average_price` in case a future API version adds it directly.
    let entry_price = item
        .get("average_price")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|p| *p > 0.0)
        .or_else(|| {
            item.get("open_pos_cost")
                .or_else(|| item.get("cost"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .and_then(|cost| {
                    if raw_qty.abs() > 0.0 {
                        Some(cost.abs() / raw_qty.abs())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or(0.0);

    Some(Position {
        instrument,
        side,
        quantity: raw_qty.abs(),
        entry_price,
        mark_price: 0.0,
        pnl: 0.0,
        liquidation_price: item
            .get("liquidation_price")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0),
    })
}

impl Tab for PositionsTab {
    fn on_key(&mut self, key: KeyEvent, _state: &mut AppState) -> bool {
        match key.code {
            KeyCode::Esc => {
                if self.detail_position.is_some() {
                    self.detail_position = None;
                    return true;
                }
                false
            }
            KeyCode::Enter => {
                if self.detail_position.is_none() {
                    self.detail_position = Some(self.selected);
                    return true;
                }
                false
            }
            KeyCode::Up if self.detail_position.is_none() => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                true
            }
            KeyCode::Down if self.detail_position.is_none() => {
                if self.selected < self.positions.len().saturating_sub(1) {
                    self.selected += 1;
                }
                true
            }
            KeyCode::Char('r') if self.detail_position.is_none() => {
                self.loaded = false;
                true
            }
            _ => false,
        }
    }

    fn on_data(&mut self, event: &DataEvent, state: &mut AppState) {
        match event {
            DataEvent::RestResponse { method, data } if method == "private/get-positions" => {
                if let Some(arr) = data.get("data").and_then(|d| d.as_array()) {
                    self.rebuild_positions(arr, state);
                }
            }
            DataEvent::PositionsSnapshot(records) => {
                // `user.positions` sends the full open-position set on each
                // delta, so replace wholesale — matches AppState::update_positions.
                self.rebuild_positions(records, state);
            }
            _ => {
                if state.paper_mode {
                    self.load_paper_positions(state);
                    return;
                }
                if !self.loaded {
                    self.request_data(state);
                }
                if self.loaded {
                    apply_live_mark_and_pnl(&mut self.positions, state);
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState) {
        if !state.paper_mode && !state.authenticated {
            frame.render_widget(
                Paragraph::new(
                    "Positions \u{2014} not authenticated. Set CDC_API_KEY and CDC_API_SECRET.",
                )
                .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        if !self.loaded {
            frame.render_widget(
                Paragraph::new("Positions \u{2014} loading...")
                    .style(Style::default().fg(state.theme.colors.muted)),
                area,
            );
            return;
        }

        // Render detail view if a position is selected for detail
        if let Some(idx) = self.detail_position {
            if let Some(pos) = self.positions.get(idx) {
                self.draw_detail(frame, area, state, pos);
                return;
            }
        }

        let [table_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        if self.positions.is_empty() {
            frame.render_widget(
                Paragraph::new("No open positions.")
                    .style(Style::default().fg(state.theme.colors.muted)),
                table_area,
            );
        } else {
            let header = Row::new(vec![
                Cell::from("Instrument"),
                Cell::from("Side"),
                Cell::from("Size"),
                Cell::from("Entry"),
                Cell::from("Mark"),
                Cell::from("P&L"),
                Cell::from("Liq. Price"),
            ])
            .style(
                Style::default()
                    .fg(state.theme.colors.header)
                    .add_modifier(Modifier::BOLD),
            );

            let widths = [
                Constraint::Length(16),
                Constraint::Length(6),
                Constraint::Length(12),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Length(14),
            ];

            let rows: Vec<Row> = self
                .positions
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let is_selected = i == self.selected;
                    let pnl_color = if p.pnl >= 0.0 {
                        state.theme.colors.positive
                    } else {
                        state.theme.colors.negative
                    };
                    let side_color = if p.side == "BUY" || p.side == "LONG" {
                        state.theme.colors.positive
                    } else {
                        state.theme.colors.negative
                    };

                    let row_style = if is_selected {
                        Style::default()
                            .fg(state.theme.colors.selected_fg)
                            .bg(state.theme.colors.selected_bg)
                    } else {
                        Style::default().fg(state.theme.colors.fg)
                    };

                    // When selected, use uniform colors for readability
                    if is_selected {
                        Row::new(vec![
                            Cell::from(p.instrument.as_str()),
                            Cell::from(p.side.as_str()),
                            Cell::from(format!("{:.4}", p.quantity)),
                            Cell::from(format!("{:.2}", p.entry_price)),
                            Cell::from(format!("{:.2}", p.mark_price)),
                            Cell::from(format!("{:+.2}", p.pnl)),
                            Cell::from(if p.liquidation_price > 0.0 {
                                format!("{:.2}", p.liquidation_price)
                            } else {
                                "\u{2014}".into()
                            }),
                        ])
                        .style(row_style)
                    } else {
                        Row::new(vec![
                            Cell::from(p.instrument.as_str()),
                            Cell::from(p.side.as_str()).style(Style::default().fg(side_color)),
                            Cell::from(format!("{:.4}", p.quantity)),
                            Cell::from(format!("{:.2}", p.entry_price)),
                            Cell::from(format!("{:.2}", p.mark_price))
                                .style(Style::default().fg(state.theme.colors.volume)),
                            Cell::from(format!("{:+.2}", p.pnl))
                                .style(Style::default().fg(pnl_color)),
                            Cell::from(if p.liquidation_price > 0.0 {
                                format!("{:.2}", p.liquidation_price)
                            } else {
                                "\u{2014}".into()
                            })
                            .style(Style::default().fg(state.theme.colors.muted)),
                        ])
                        .style(row_style)
                    }
                })
                .collect();

            let table = Table::new(rows, widths).header(header).column_spacing(1);
            frame.render_widget(table, table_area);
        }

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "r:refresh  \u{2191}\u{2193}:navigate  Enter:detail  t:trade  x:close  o/O:OCO/OTOCO  c:cancel-orders",
                Style::default().fg(state.theme.colors.muted),
            ))),
            footer_area,
        );
    }

    fn subscriptions(&self, _state: &AppState) -> Vec<String> {
        // Subscribe to tickers for mark price updates on position instruments
        self.positions
            .iter()
            .map(|p| format!("ticker.{}", p.instrument))
            .collect()
    }

    fn on_activate(&mut self) {
        // Intentionally no-op: size/entry/liq stay current via user.positions
        // snapshots, and mark/P&L recompute every tick from state.tickers.
        // The initial REST prime runs on first activation via the fallthrough.
    }

    fn selected_instrument(&self) -> Option<&str> {
        self.positions
            .get(self.selected)
            .map(|p| p.instrument.as_str())
    }

    fn on_click(&mut self, row: u16, _col: u16, _state: &mut AppState) -> bool {
        if self.detail_position.is_some() {
            return false;
        }
        // Layout: row 0 = table header, row 1+ = data rows
        if row >= 1 {
            let data_row = (row - 1) as usize;
            if data_row < self.positions.len() {
                self.selected = data_row;
                return true;
            }
        }
        false
    }

    fn on_double_click(&mut self, row: u16, _col: u16, _state: &mut AppState) -> bool {
        if self.detail_position.is_some() {
            return false;
        }
        if row >= 1 {
            let data_row = (row - 1) as usize;
            if data_row < self.positions.len() {
                self.selected = data_row;
                self.detail_position = Some(self.selected);
                return true;
            }
        }
        false
    }
}

impl PositionsTab {
    fn draw_detail(&self, frame: &mut Frame, area: Rect, state: &AppState, pos: &Position) {
        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        // Calculate position value
        let position_value = pos.quantity * pos.mark_price;

        // Determine P&L color
        let pnl_color = if pos.pnl >= 0.0 {
            state.theme.colors.positive
        } else {
            state.theme.colors.negative
        };

        // Create detail lines in two-column layout
        let mut lines: Vec<Line> = vec![];

        // Title
        lines.push(Line::from(Span::styled(
            format!("Position Detail: {}", pos.instrument),
            Style::default()
                .fg(state.theme.colors.header)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        // Column 1 & 2
        lines.push(Line::from(vec![
            Span::styled(
                "Instrument: ",
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::raw(pos.instrument.as_str()),
            Span::raw("  "),
            Span::styled("Side: ", Style::default().fg(state.theme.colors.muted)),
            Span::styled(
                pos.side.as_str(),
                Style::default().fg(if pos.side == "BUY" || pos.side == "LONG" {
                    state.theme.colors.positive
                } else {
                    state.theme.colors.negative
                }),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("Size: ", Style::default().fg(state.theme.colors.muted)),
            Span::raw(format!("{:.4}", pos.quantity)),
            Span::raw("  "),
            Span::styled(
                "Entry Price: ",
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::raw(format!("{:.2}", pos.entry_price)),
        ]));

        lines.push(Line::from(vec![
            Span::styled(
                "Mark Price: ",
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::raw(format!("{:.2}", pos.mark_price)),
            Span::raw("  "),
            Span::styled(
                "Position Value: ",
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::raw(format!("{:.2}", position_value)),
        ]));

        lines.push(Line::from(""));

        lines.push(Line::from(vec![
            Span::styled(
                "Unrealized P&L: ",
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::styled(format!("{:+.2}", pos.pnl), Style::default().fg(pnl_color)),
            Span::raw("  "),
            Span::styled(
                "Liquidation Price: ",
                Style::default().fg(state.theme.colors.muted),
            ),
            Span::raw(if pos.liquidation_price > 0.0 {
                format!("{:.2}", pos.liquidation_price)
            } else {
                "\u{2014}".into()
            }),
        ]));

        let paragraph = Paragraph::new(lines).style(Style::default().fg(state.theme.colors.fg));
        frame.render_widget(paragraph, content_area);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Esc:back to positions",
                Style::default().fg(state.theme.colors.muted),
            ))),
            footer_area,
        );
    }
}
