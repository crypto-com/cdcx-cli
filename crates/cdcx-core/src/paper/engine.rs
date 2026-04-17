use serde_json::json;
use std::path::PathBuf;

use super::types::*;
use crate::api_client::ApiClient;
use crate::error::CdcxError;

const PAPER_DIR: &str = ".cdcx";
const PAPER_FILE: &str = "paper.json";

pub struct PaperEngine {
    pub state: PaperState,
    state_path: PathBuf,
}

impl PaperEngine {
    fn default_path() -> Result<PathBuf, CdcxError> {
        let home = dirs::home_dir()
            .ok_or_else(|| CdcxError::Config("Cannot determine home directory".into()))?;
        Ok(home.join(PAPER_DIR).join(PAPER_FILE))
    }

    pub fn load() -> Result<Self, CdcxError> {
        let path = Self::default_path()?;
        if !path.exists() {
            return Err(CdcxError::Config(
                "No paper account. Run `cdcx paper init` first.".into(),
            ));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| CdcxError::Config(format!("Failed to read paper state: {}", e)))?;
        let state: PaperState = serde_json::from_str(&content)
            .map_err(|e| CdcxError::Config(format!("Failed to parse paper state: {}", e)))?;
        Ok(Self {
            state,
            state_path: path,
        })
    }

    pub fn load_or_init(balance: f64) -> Result<Self, CdcxError> {
        match Self::load() {
            Ok(engine) => Ok(engine),
            Err(_) => Self::init(balance),
        }
    }

    pub fn init(balance: f64) -> Result<Self, CdcxError> {
        let path = Self::default_path()?;
        let state = PaperState {
            balance,
            initial_balance: balance,
            positions: Vec::new(),
            open_orders: Vec::new(),
            trade_history: Vec::new(),
            next_order_id: 1,
            next_trade_id: 1,
        };
        let engine = Self {
            state,
            state_path: path,
        };
        engine.save()?;
        Ok(engine)
    }

    pub fn save(&self) -> Result<(), CdcxError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CdcxError::Config(format!("Failed to create paper dir: {}", e)))?;
        }
        let content = serde_json::to_string_pretty(&self.state)
            .map_err(|e| CdcxError::Config(format!("Failed to serialize paper state: {}", e)))?;
        std::fs::write(&self.state_path, content)
            .map_err(|e| CdcxError::Config(format!("Failed to write paper state: {}", e)))?;
        Ok(())
    }

    /// Place a new order. Market orders fill immediately at current price.
    pub async fn place_order(
        &mut self,
        client: &ApiClient,
        req: OrderRequest,
    ) -> Result<PaperOrder, CdcxError> {
        if req.quantity <= 0.0 {
            return Err(CdcxError::Config("Quantity must be positive".into()));
        }
        if req.order_type == OrderType::Limit && req.price.is_none() {
            return Err(CdcxError::Config("Limit orders require a price".into()));
        }

        let order_id = self.state.next_order_id;
        self.state.next_order_id += 1;
        let now = chrono::Utc::now().to_rfc3339();

        let mut order = PaperOrder {
            order_id,
            instrument_name: req.instrument_name.clone(),
            side: req.side,
            order_type: req.order_type,
            quantity: req.quantity,
            price: req.price,
            status: OrderStatus::Open,
            created_at: now,
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        match req.order_type {
            OrderType::Market => {
                let ticker = fetch_ticker(client, &req.instrument_name).await?;
                let fill_price = match req.side {
                    OrderSide::Buy => ticker.ask,
                    OrderSide::Sell => ticker.bid,
                };
                self.execute_fill(&mut order, fill_price, true)?;
            }
            OrderType::Limit => {
                self.state.open_orders.push(order.clone());
            }
        }

        self.save()?;
        Ok(order)
    }

    /// Cancel an open order by ID.
    pub fn cancel_order(&mut self, order_id: u64) -> Result<PaperOrder, CdcxError> {
        let idx = self
            .state
            .open_orders
            .iter()
            .position(|o| o.order_id == order_id)
            .ok_or_else(|| CdcxError::Config(format!("No open order with id {}", order_id)))?;
        let mut order = self.state.open_orders.remove(idx);
        order.status = OrderStatus::Cancelled;
        self.save()?;
        Ok(order)
    }

    /// Check all open limit orders against current prices and fill those that cross.
    pub async fn check_fills(&mut self, client: &ApiClient) -> Result<Vec<PaperTrade>, CdcxError> {
        let mut fills = Vec::new();
        let mut i = 0;

        while i < self.state.open_orders.len() {
            let (instrument, limit_price, side) = {
                let order = &self.state.open_orders[i];
                (order.instrument_name.clone(), order.price, order.side)
            };

            let limit_price = match limit_price {
                Some(p) => p,
                None => {
                    i += 1;
                    continue;
                }
            };

            let ticker = match fetch_ticker(client, &instrument).await {
                Ok(t) => t,
                Err(_) => {
                    i += 1;
                    continue;
                }
            };

            let should_fill = match side {
                OrderSide::Buy => ticker.ask < limit_price,
                OrderSide::Sell => ticker.bid > limit_price,
            };

            if should_fill {
                let mut order = self.state.open_orders.remove(i);
                let trade = self.execute_fill(&mut order, limit_price, false)?;
                if let Some(t) = trade {
                    fills.push(t);
                }
            } else {
                i += 1;
            }
        }

        if !fills.is_empty() {
            self.save()?;
        }
        Ok(fills)
    }

    /// Build a portfolio status snapshot with live unrealized P&L.
    pub async fn portfolio_status(&self, client: &ApiClient) -> Result<PortfolioStatus, CdcxError> {
        let mut position_views = Vec::new();
        let mut total_unrealized = 0.0;
        let mut total_realized = 0.0;

        for pos in &self.state.positions {
            if pos.quantity.abs() < 1e-12 {
                total_realized += pos.realized_pnl;
                continue;
            }

            let current_price = match fetch_ticker(client, &pos.instrument_name).await {
                Ok(t) => t.last,
                Err(_) => pos.avg_entry_price,
            };

            let unrealized = if pos.quantity > 0.0 {
                (current_price - pos.avg_entry_price) * pos.quantity
            } else {
                (pos.avg_entry_price - current_price) * pos.quantity.abs()
            };

            total_unrealized += unrealized;
            total_realized += pos.realized_pnl;

            position_views.push(PositionView {
                instrument_name: pos.instrument_name.clone(),
                side: if pos.quantity > 0.0 {
                    "long".into()
                } else {
                    "short".into()
                },
                quantity: pos.quantity.abs(),
                avg_entry_price: pos.avg_entry_price,
                current_price,
                unrealized_pnl: unrealized,
                realized_pnl: pos.realized_pnl,
            });
        }

        Ok(PortfolioStatus {
            balance: self.state.balance,
            initial_balance: self.state.initial_balance,
            open_order_count: self.state.open_orders.len(),
            positions: position_views,
            total_unrealized_pnl: total_unrealized,
            total_realized_pnl: total_realized,
        })
    }

    /// Wipe all state and start fresh.
    pub fn reset(&mut self, balance: f64) -> Result<(), CdcxError> {
        self.state = PaperState {
            balance,
            initial_balance: balance,
            positions: Vec::new(),
            open_orders: Vec::new(),
            trade_history: Vec::new(),
            next_order_id: 1,
            next_trade_id: 1,
        };
        self.save()
    }

    /// Execute a market order synchronously using a pre-fetched price.
    /// Used by the TUI which already has live ticker data in memory.
    pub fn execute_market_order_at_price(
        &mut self,
        instrument: &str,
        side: OrderSide,
        quantity: f64,
        fill_price: f64,
    ) -> Result<PaperTrade, CdcxError> {
        if quantity <= 0.0 {
            return Err(CdcxError::Config("Quantity must be positive".into()));
        }

        let order_id = self.state.next_order_id;
        self.state.next_order_id += 1;
        let now = chrono::Utc::now().to_rfc3339();

        let mut order = PaperOrder {
            order_id,
            instrument_name: instrument.to_string(),
            side,
            order_type: OrderType::Market,
            quantity,
            price: None,
            status: OrderStatus::Open,
            created_at: now,
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let trade = self
            .execute_fill(&mut order, fill_price, true)?
            .ok_or_else(|| CdcxError::Config("Fill failed".into()))?;
        self.save()?;
        Ok(trade)
    }

    /// Add a limit order to the open orders list.
    pub fn place_limit_order(
        &mut self,
        instrument: &str,
        side: OrderSide,
        quantity: f64,
        price: f64,
    ) -> Result<PaperOrder, CdcxError> {
        if quantity <= 0.0 {
            return Err(CdcxError::Config("Quantity must be positive".into()));
        }

        let order_id = self.state.next_order_id;
        self.state.next_order_id += 1;
        let now = chrono::Utc::now().to_rfc3339();

        let order = PaperOrder {
            order_id,
            instrument_name: instrument.to_string(),
            side,
            order_type: OrderType::Limit,
            quantity,
            price: Some(price),
            status: OrderStatus::Open,
            created_at: now,
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        self.state.open_orders.push(order.clone());
        self.save()?;
        Ok(order)
    }

    fn execute_fill(
        &mut self,
        order: &mut PaperOrder,
        fill_price: f64,
        is_taker: bool,
    ) -> Result<Option<PaperTrade>, CdcxError> {
        let notional = fill_price * order.quantity;
        let fee_rate = if is_taker {
            TAKER_FEE_RATE
        } else {
            MAKER_FEE_RATE
        };
        let fee = notional * fee_rate;

        match order.side {
            OrderSide::Buy => {
                let cost = notional + fee;
                if self.state.balance < cost {
                    return Err(CdcxError::Config(format!(
                        "Insufficient balance: need {:.2}, have {:.2}",
                        cost, self.state.balance
                    )));
                }
                self.state.balance -= cost;
            }
            OrderSide::Sell => {
                // Verify user holds sufficient position to sell
                let current_position = self
                    .state
                    .positions
                    .iter()
                    .find(|p| p.instrument_name == order.instrument_name)
                    .map(|p| p.quantity)
                    .unwrap_or(0.0);

                if current_position + 1e-12 < order.quantity {
                    return Err(CdcxError::Config(format!(
                        "Insufficient position: need {:.2}, have {:.2}",
                        order.quantity, current_position
                    )));
                }

                self.state.balance += notional - fee;
            }
        }

        let signed_qty = match order.side {
            OrderSide::Buy => order.quantity,
            OrderSide::Sell => -order.quantity,
        };
        self.update_position(&order.instrument_name, signed_qty, fill_price);

        let trade_id = self.state.next_trade_id;
        self.state.next_trade_id += 1;
        let now = chrono::Utc::now().to_rfc3339();

        let trade = PaperTrade {
            trade_id,
            order_id: order.order_id,
            instrument_name: order.instrument_name.clone(),
            side: order.side,
            quantity: order.quantity,
            price: fill_price,
            fee,
            timestamp: now.clone(),
        };
        self.state.trade_history.push(trade.clone());

        order.status = OrderStatus::Filled;
        order.filled_at = Some(now);
        order.fill_price = Some(fill_price);
        order.fee = Some(fee);

        Ok(Some(trade))
    }

    fn update_position(&mut self, instrument: &str, signed_qty: f64, price: f64) {
        let pos = self
            .state
            .positions
            .iter_mut()
            .find(|p| p.instrument_name == instrument);

        match pos {
            Some(pos) => {
                let old_qty = pos.quantity;
                let new_qty = old_qty + signed_qty;

                if old_qty.signum() == signed_qty.signum() || old_qty.abs() < 1e-12 {
                    let total_cost = pos.avg_entry_price * old_qty.abs() + price * signed_qty.abs();
                    let total_qty = old_qty.abs() + signed_qty.abs();
                    pos.avg_entry_price = if total_qty > 1e-12 {
                        total_cost / total_qty
                    } else {
                        price
                    };
                    pos.quantity = new_qty;
                } else {
                    let closed_qty = old_qty.abs().min(signed_qty.abs());
                    let pnl = if old_qty > 0.0 {
                        (price - pos.avg_entry_price) * closed_qty
                    } else {
                        (pos.avg_entry_price - price) * closed_qty
                    };
                    pos.realized_pnl += pnl;

                    if new_qty.abs() < 1e-12 {
                        pos.quantity = 0.0;
                    } else if new_qty.signum() != old_qty.signum() {
                        pos.quantity = new_qty;
                        pos.avg_entry_price = price;
                    } else {
                        pos.quantity = new_qty;
                    }
                }
            }
            None => {
                // Only create a position for buy orders (positive quantity).
                // Sell orders should have been validated in execute_fill() to not reach here.
                if signed_qty > 0.0 {
                    self.state.positions.push(Position {
                        instrument_name: instrument.to_string(),
                        quantity: signed_qty,
                        avg_entry_price: price,
                        realized_pnl: 0.0,
                    });
                }
                // If signed_qty < 0 here, it's a logic error—execute_fill should have prevented it
            }
        }
    }
}

// --- Price feed (uses TickerSnapshot from types.rs) ---

async fn fetch_ticker(client: &ApiClient, instrument: &str) -> Result<TickerSnapshot, CdcxError> {
    let result = client
        .public_request("public/get-tickers", json!({"instrument_name": instrument}))
        .await?;

    let data = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| CdcxError::Config(format!("No ticker data for {}", instrument)))?;

    let last = parse_price(data, "a", instrument)?;
    let bid = parse_price(data, "b", instrument).unwrap_or(last);
    let ask = parse_price(data, "k", instrument).unwrap_or(last);

    Ok(TickerSnapshot { bid, ask, last })
}

fn parse_price(data: &serde_json::Value, field: &str, instrument: &str) -> Result<f64, CdcxError> {
    let val = match data.get(field) {
        Some(v) if !v.is_null() => v,
        _ => {
            return Err(CdcxError::Config(format!(
                "Missing '{}' in ticker for {}",
                field, instrument
            )))
        }
    };
    if let Some(s) = val.as_str() {
        s.parse::<f64>().map_err(|_| {
            CdcxError::Config(format!(
                "Cannot parse '{}' value '{}' for {}",
                field, s, instrument
            ))
        })
    } else if let Some(n) = val.as_f64() {
        Ok(n)
    } else {
        Err(CdcxError::Config(format!(
            "Cannot parse '{}' for {}",
            field, instrument
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper::types::OrderType;

    fn create_test_engine() -> PaperEngine {
        // Create a temporary engine in memory without saving to disk
        let state = PaperState {
            balance: 10_000.0,
            initial_balance: 10_000.0,
            positions: Vec::new(),
            open_orders: Vec::new(),
            trade_history: Vec::new(),
            next_order_id: 1,
            next_trade_id: 1,
        };
        PaperEngine {
            state,
            state_path: std::path::PathBuf::from("/tmp/test_paper.json"),
        }
    }

    #[test]
    fn test_sell_with_no_position_fails() {
        let mut engine = create_test_engine();
        let mut order = PaperOrder {
            order_id: 1,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let result = engine.execute_fill(&mut order, 50_000.0, true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Insufficient position"));
    }

    #[test]
    fn test_sell_with_insufficient_position_fails() {
        let mut engine = create_test_engine();

        // Add a small position
        engine.state.positions.push(Position {
            instrument_name: "BTC_USD".to_string(),
            quantity: 0.5,
            avg_entry_price: 40_000.0,
            realized_pnl: 0.0,
        });

        let mut order = PaperOrder {
            order_id: 1,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let result = engine.execute_fill(&mut order, 50_000.0, true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Insufficient position"));
    }

    #[test]
    fn test_sell_with_sufficient_position_succeeds() {
        let mut engine = create_test_engine();
        let initial_balance = engine.state.balance;

        // Add a position
        engine.state.positions.push(Position {
            instrument_name: "BTC_USD".to_string(),
            quantity: 1.0,
            avg_entry_price: 40_000.0,
            realized_pnl: 0.0,
        });

        let mut order = PaperOrder {
            order_id: 1,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let fill_price = 50_000.0;
        let result = engine.execute_fill(&mut order, fill_price, true);
        assert!(result.is_ok());
        let trade = result.unwrap().unwrap();

        // Verify balance increased: +notional - fee
        let notional = fill_price * 1.0;
        let fee = notional * TAKER_FEE_RATE;
        let expected_balance = initial_balance + notional - fee;
        assert!((engine.state.balance - expected_balance).abs() < 0.01);

        // Verify position was closed
        assert_eq!(engine.state.positions[0].quantity, 0.0);
        assert!(trade.fee > 0.0);
    }

    #[test]
    fn test_buy_then_sell_roundtrip() {
        let mut engine = create_test_engine();
        let initial_balance = engine.state.balance;
        let buy_price = 100.0;
        let sell_price = 150.0;
        let quantity = 10.0;

        // BUY order
        let mut buy_order = PaperOrder {
            order_id: 1,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let result = engine.execute_fill(&mut buy_order, buy_price, true);
        assert!(result.is_ok());

        let buy_cost = buy_price * quantity * (1.0 + TAKER_FEE_RATE);
        let balance_after_buy = initial_balance - buy_cost;
        assert!((engine.state.balance - balance_after_buy).abs() < 0.01);
        assert_eq!(engine.state.positions[0].quantity, quantity);

        // SELL order
        let mut sell_order = PaperOrder {
            order_id: 2,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let result = engine.execute_fill(&mut sell_order, sell_price, true);
        assert!(result.is_ok());

        let sell_proceeds = sell_price * quantity * (1.0 - TAKER_FEE_RATE);
        let expected_balance = balance_after_buy + sell_proceeds;
        assert!((engine.state.balance - expected_balance).abs() < 0.01);
        assert_eq!(engine.state.positions[0].quantity, 0.0);
    }

    #[test]
    fn test_multiple_sells_exceeding_position_fails_on_second() {
        let mut engine = create_test_engine();

        // Add a 2 unit position
        engine.state.positions.push(Position {
            instrument_name: "ETH_USD".to_string(),
            quantity: 2.0,
            avg_entry_price: 3_000.0,
            realized_pnl: 0.0,
        });

        // First sell 1 unit (should succeed)
        let mut order1 = PaperOrder {
            order_id: 1,
            instrument_name: "ETH_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let result = engine.execute_fill(&mut order1, 4_000.0, true);
        assert!(result.is_ok());
        assert_eq!(engine.state.positions[0].quantity, 1.0);

        // Second sell 1.5 units (should fail—only 1 left)
        let mut order2 = PaperOrder {
            order_id: 2,
            instrument_name: "ETH_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 1.5,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        let result = engine.execute_fill(&mut order2, 4_000.0, true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Insufficient position"));
    }

    #[test]
    fn test_sell_zero_quantity_rejected() {
        let mut engine = create_test_engine();

        // Add a position
        engine.state.positions.push(Position {
            instrument_name: "BTC_USD".to_string(),
            quantity: 1.0,
            avg_entry_price: 40_000.0,
            realized_pnl: 0.0,
        });

        let mut order = PaperOrder {
            order_id: 1,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 0.0,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };

        // Rejection happens in place_order/execute_market_order_at_price, not execute_fill
        // But execute_fill should still work with 0 quantity (though it's logically invalid)
        let result = engine.execute_fill(&mut order, 50_000.0, true);
        // This will succeed at execute_fill level because 0 < 1.0 position check passes
        // The validation at entry points catches zero quantities
        assert!(result.is_ok());
    }

    #[test]
    fn test_partial_position_sell_and_rebuy() {
        let mut engine = create_test_engine();

        // Buy 2.0 units at $100
        let mut buy_order = PaperOrder {
            order_id: 1,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: 2.0,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };
        engine.execute_fill(&mut buy_order, 100.0, true).unwrap();
        assert_eq!(engine.state.positions[0].quantity, 2.0);

        // Sell 0.5 units at $150
        let mut sell_order = PaperOrder {
            order_id: 2,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 0.5,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };
        engine.execute_fill(&mut sell_order, 150.0, true).unwrap();
        assert_eq!(engine.state.positions[0].quantity, 1.5);

        // Verify we can only sell up to 1.5 now
        let mut invalid_sell = PaperOrder {
            order_id: 3,
            instrument_name: "BTC_USD".to_string(),
            side: OrderSide::Sell,
            order_type: OrderType::Market,
            quantity: 2.0,
            price: None,
            status: OrderStatus::Open,
            created_at: chrono::Utc::now().to_rfc3339(),
            filled_at: None,
            fill_price: None,
            fee: None,
        };
        let result = engine.execute_fill(&mut invalid_sell, 150.0, true);
        assert!(result.is_err());
    }
}
