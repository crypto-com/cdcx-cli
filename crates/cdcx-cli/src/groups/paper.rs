use cdcx_core::api_client::ApiClient;
use cdcx_core::env::Environment;
use cdcx_core::error::CdcxError;
use cdcx_core::paper::engine::PaperEngine;
use cdcx_core::paper::types::{OrderRequest, OrderSide, OrderType};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct PaperCmd {
    #[command(subcommand)]
    pub command: PaperSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum PaperSubcommand {
    /// Initialize a paper trading account
    Init {
        /// Starting balance in USD
        #[arg(long, default_value = "10000")]
        balance: f64,
    },
    /// Place a paper buy order
    Buy {
        /// Instrument (e.g. BTC_USDT)
        instrument: String,
        /// Quantity
        #[arg(long)]
        quantity: f64,
        /// Price (omit for market order)
        #[arg(long)]
        price: Option<f64>,
    },
    /// Place a paper sell order
    Sell {
        /// Instrument (e.g. BTC_USDT)
        instrument: String,
        /// Quantity
        #[arg(long)]
        quantity: f64,
        /// Price (omit for market order)
        #[arg(long)]
        price: Option<f64>,
    },
    /// Show paper portfolio positions and P&L
    Positions,
    /// Show paper trade history
    History,
    /// Show paper account balance
    Balance,
    /// Reset paper account to starting balance
    Reset {
        /// Starting balance
        #[arg(long, default_value = "10000")]
        balance: f64,
    },
}

pub async fn run_paper(cmd: &PaperSubcommand, env: Environment) -> Result<(), CdcxError> {
    let client = ApiClient::new(None, env);

    match cmd {
        PaperSubcommand::Init { balance } => {
            let engine = PaperEngine::init(*balance)?;
            println!(
                "Paper account initialized with ${:.2}",
                engine.state.balance
            );
            Ok(())
        }
        PaperSubcommand::Buy {
            instrument,
            quantity,
            price,
        } => {
            let mut engine = PaperEngine::load_or_init(10000.0)?;
            let order_type = if price.is_some() {
                OrderType::Limit
            } else {
                OrderType::Market
            };
            let req = OrderRequest {
                instrument_name: instrument.clone(),
                side: OrderSide::Buy,
                order_type,
                quantity: *quantity,
                price: *price,
            };
            let order = engine.place_order(&client, req).await?;
            let status = format!("{:?}", order.status).to_lowercase();
            println!(
                "Order #{}: BUY {} {} @ {} [{}]",
                order.order_id,
                quantity,
                instrument,
                order
                    .fill_price
                    .map(|p| format!("{:.2}", p))
                    .unwrap_or_else(|| price
                        .map(|p| format!("{:.2} (limit)", p))
                        .unwrap_or("market".into())),
                status,
            );
            println!("Balance: ${:.2}", engine.state.balance);
            Ok(())
        }
        PaperSubcommand::Sell {
            instrument,
            quantity,
            price,
        } => {
            let mut engine = PaperEngine::load_or_init(10000.0)?;
            let order_type = if price.is_some() {
                OrderType::Limit
            } else {
                OrderType::Market
            };
            let req = OrderRequest {
                instrument_name: instrument.clone(),
                side: OrderSide::Sell,
                order_type,
                quantity: *quantity,
                price: *price,
            };
            let order = engine.place_order(&client, req).await?;
            let status = format!("{:?}", order.status).to_lowercase();
            println!(
                "Order #{}: SELL {} {} @ {} [{}]",
                order.order_id,
                quantity,
                instrument,
                order
                    .fill_price
                    .map(|p| format!("{:.2}", p))
                    .unwrap_or_else(|| price
                        .map(|p| format!("{:.2} (limit)", p))
                        .unwrap_or("market".into())),
                status,
            );
            println!("Balance: ${:.2}", engine.state.balance);
            Ok(())
        }
        PaperSubcommand::Positions => {
            let engine = PaperEngine::load_or_init(10000.0)?;
            let status = engine.portfolio_status(&client).await?;
            println!(
                "Balance: ${:.2} (started: ${:.2})",
                status.balance, status.initial_balance
            );
            println!("Open orders: {}", status.open_order_count);
            if status.positions.is_empty() {
                println!("No positions.");
            } else {
                println!(
                    "{:<15} {:>6} {:>10} {:>12} {:>12} {:>12}",
                    "Instrument", "Side", "Qty", "Entry", "Current", "Unreal. P&L"
                );
                for p in &status.positions {
                    println!(
                        "{:<15} {:>6} {:>10.4} {:>12.2} {:>12.2} {:>12.2}",
                        p.instrument_name,
                        p.side,
                        p.quantity,
                        p.avg_entry_price,
                        p.current_price,
                        p.unrealized_pnl
                    );
                }
            }
            println!("Unrealized P&L: ${:.2}", status.total_unrealized_pnl);
            println!("Realized P&L:   ${:.2}", status.total_realized_pnl);
            Ok(())
        }
        PaperSubcommand::History => {
            let engine = PaperEngine::load_or_init(10000.0)?;
            if engine.state.trade_history.is_empty() {
                println!("No trades yet.");
            } else {
                println!(
                    "{:<5} {:>15} {:>6} {:>10} {:>12} {:>8}",
                    "ID", "Instrument", "Side", "Qty", "Price", "Fee"
                );
                for t in &engine.state.trade_history {
                    let side = format!("{:?}", t.side).to_uppercase();
                    println!(
                        "{:<5} {:>15} {:>6} {:>10.4} {:>12.2} {:>8.4}",
                        t.trade_id, t.instrument_name, side, t.quantity, t.price, t.fee
                    );
                }
            }
            Ok(())
        }
        PaperSubcommand::Balance => {
            let engine = PaperEngine::load_or_init(10000.0)?;
            println!("Balance: ${:.2}", engine.state.balance);
            println!("Initial: ${:.2}", engine.state.initial_balance);
            let pnl = engine.state.balance - engine.state.initial_balance;
            println!("P&L:     ${:+.2}", pnl);
            Ok(())
        }
        PaperSubcommand::Reset { balance } => {
            let mut engine = PaperEngine::load_or_init(10000.0)?;
            engine.reset(*balance)?;
            println!("Paper account reset to ${:.2}", balance);
            Ok(())
        }
    }
}
