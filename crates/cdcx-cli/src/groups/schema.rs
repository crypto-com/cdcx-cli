use clap::Parser;

#[derive(Parser, Debug)]
pub struct SchemaCmd {
    #[command(subcommand)]
    pub command: SchemaSubcommand,
}

#[derive(clap::Subcommand, Debug)]
pub enum SchemaSubcommand {
    /// List all endpoints or filter by group
    List {
        /// Filter by group name
        #[arg(long)]
        group: Option<String>,
    },
    /// Show full schema for an endpoint
    Show {
        /// API method (e.g., public/get-tickers)
        method: String,
    },
    /// Output full tool catalog JSON
    Catalog,
    /// Force-refresh the cached OpenAPI spec
    Update,
    /// Show cache status, age, and endpoint count
    Status,
}
