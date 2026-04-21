pub mod history;
pub mod market;
pub mod orders;
pub mod portfolio;
pub mod positions;
pub mod watchlist;

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::state::{AppState, TickerData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Market,
    Portfolio,
    Orders,
    History,
    Watchlist,
    Positions,
}

impl TabKind {
    pub const ALL: &[TabKind] = &[
        TabKind::Market,
        TabKind::Portfolio,
        TabKind::Orders,
        TabKind::History,
        TabKind::Watchlist,
        TabKind::Positions,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            TabKind::Market => "Market",
            TabKind::Portfolio => "Portfolio",
            TabKind::Orders => "Orders",
            TabKind::History => "History",
            TabKind::Watchlist => "Watchlist",
            TabKind::Positions => "Positions",
        }
    }
}

use crate::widgets::candlestick::Candle;

#[derive(Debug)]
pub enum DataEvent {
    TickerUpdate(TickerData),
    CandleUpdate {
        instrument: String,
        candle: Candle,
    },
    BookSnapshot(serde_json::Value),
    TradeSnapshot(serde_json::Value),
    RestResponse {
        method: String,
        data: serde_json::Value,
    },
    /// Positions snapshot from `user.positions` WS channel.
    PositionsSnapshot(Vec<serde_json::Value>),
    /// Balance snapshot from `user.balance` WS channel.
    BalanceSnapshot(Vec<serde_json::Value>),
    /// Order update from `user.order` WS channel.
    OrdersUpdate(Vec<serde_json::Value>),
}

pub trait Tab {
    fn on_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool;
    fn on_data(&mut self, event: &DataEvent, state: &mut AppState);
    fn draw(&self, frame: &mut Frame, area: Rect, state: &AppState);
    fn subscriptions(&self, state: &AppState) -> Vec<String>;
    /// Returns true when the tab has an active text input (e.g. search bar).
    /// When true, global single-key bindings are bypassed so keystrokes reach the input.
    fn is_editing(&self) -> bool {
        false
    }
    /// Returns the currently selected instrument, if any.
    fn selected_instrument(&self) -> Option<&str> {
        None
    }
    /// Export current view data as CSV string.
    fn export_csv(&self, _state: &AppState) -> Option<String> {
        None
    }
    /// Get candle data for an instrument (if the tab stores it).
    fn get_candles(&self, _instrument: &str) -> &[crate::widgets::candlestick::Candle] {
        &[]
    }
    /// Handle mouse click in the content area. row/col are relative to content area origin.
    /// Returns true if consumed.
    fn on_click(&mut self, _row: u16, _col: u16, _state: &mut AppState) -> bool {
        false
    }
    /// Handle double-click. Returns true if consumed.
    fn on_double_click(&mut self, _row: u16, _col: u16, _state: &mut AppState) -> bool {
        false
    }
    /// Called when this tab becomes active. Tabs can override to trigger a refresh.
    fn on_activate(&mut self) {}
    /// Navigate to a specific instrument's detail view (if supported by this tab).
    fn navigate_to_instrument(&mut self, _instrument: &str, _state: &AppState) {}
}
