use clap::Parser;

#[derive(Parser, Debug)]
pub struct StreamCmd {
    #[command(subcommand)]
    pub command: StreamSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum StreamSubcommand {
    /// Stream ticker updates
    Ticker {
        /// Instrument name(s)
        #[arg(required = true)]
        instruments: Vec<String>,
    },
    /// Stream order book updates
    Book {
        /// Instrument name(s)
        #[arg(required = true)]
        instruments: Vec<String>,
        /// Book depth
        #[arg(long, default_value = "10")]
        depth: String,
    },
    /// Stream trade updates
    Trades {
        /// Instrument name(s)
        #[arg(required = true)]
        instruments: Vec<String>,
    },
    /// Stream candlestick updates
    Candlestick {
        /// Instrument name(s)
        #[arg(required = true)]
        instruments: Vec<String>,
        /// Interval (1m, 5m, etc.)
        #[arg(long, default_value = "1m")]
        interval: String,
    },
    /// Stream index price
    Index {
        #[arg(required = true)]
        instruments: Vec<String>,
    },
    /// Stream mark price
    Mark {
        #[arg(required = true)]
        instruments: Vec<String>,
    },
    /// Stream settlement price
    Settlement {
        #[arg(required = true)]
        instruments: Vec<String>,
    },
    /// Stream funding rate
    Funding {
        #[arg(required = true)]
        instruments: Vec<String>,
    },
    /// Stream user order updates (requires auth)
    Orders,
    /// Stream user trade updates (requires auth)
    UserTrades,
    /// Stream balance updates (requires auth)
    Balance,
    /// Stream position updates (requires auth)
    Positions,
    /// Stream account risk updates (requires auth)
    AccountRisk,
}
