use serde::{Deserialize, Serialize};

pub const TAKER_FEE_RATE: f64 = 0.000_75; // 0.075%
pub const MAKER_FEE_RATE: f64 = 0.000_40; // 0.04%

/// Paper trading account state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperState {
    pub balance: f64,
    pub initial_balance: f64,
    pub positions: Vec<Position>,
    pub open_orders: Vec<PaperOrder>,
    pub trade_history: Vec<PaperTrade>,
    pub next_order_id: u64,
    pub next_trade_id: u64,
}

/// A single position in an instrument
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub instrument_name: String,
    /// Positive = long, negative = short
    pub quantity: f64,
    pub avg_entry_price: f64,
    pub realized_pnl: f64,
}

/// A paper trading order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperOrder {
    pub order_id: u64,
    pub instrument_name: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    /// Required for limit orders
    pub price: Option<f64>,
    pub status: OrderStatus,
    pub created_at: String,
    pub filled_at: Option<String>,
    pub fill_price: Option<f64>,
    pub fee: Option<f64>,
}

/// A completed trade
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperTrade {
    pub trade_id: u64,
    pub order_id: u64,
    pub instrument_name: String,
    pub side: OrderSide,
    pub quantity: f64,
    pub price: f64,
    pub fee: f64,
    pub timestamp: String,
}

/// Buy or sell side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Market or limit order type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderType {
    Market,
    Limit,
}

/// Order status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderStatus {
    Open,
    Filled,
    Cancelled,
}

/// Request to place a new paper order
#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub instrument_name: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    pub price: Option<f64>,
}

/// Snapshot of portfolio for display
#[derive(Debug, Serialize)]
pub struct PortfolioStatus {
    pub balance: f64,
    pub initial_balance: f64,
    pub open_order_count: usize,
    pub positions: Vec<PositionView>,
    pub total_unrealized_pnl: f64,
    pub total_realized_pnl: f64,
}

/// Position view for display
#[derive(Debug, Serialize)]
pub struct PositionView {
    pub instrument_name: String,
    pub side: String,
    pub quantity: f64,
    pub avg_entry_price: f64,
    pub current_price: f64,
    pub unrealized_pnl: f64,
    pub realized_pnl: f64,
}

/// Ticker snapshot (bid/ask/last prices)
#[derive(Debug, Clone)]
pub struct TickerSnapshot {
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
}

/// Fee rates (public for testing)
pub const TAKER_FEE: f64 = TAKER_FEE_RATE;
pub const MAKER_FEE: f64 = MAKER_FEE_RATE;
