use clap::Parser;

#[derive(Parser, Debug)]
pub struct GlobalFlags {
    /// Output format: json (default), table, ndjson
    #[arg(short = 'o', long = "output", global = true)]
    pub output: Option<String>,

    /// Pass raw JSON as request body
    #[arg(long = "json", global = true)]
    pub json_input: Option<String>,

    /// Environment: production (default), uat
    #[arg(long, global = true)]
    pub env: Option<String>,

    /// Config profile name
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Show what would be sent without executing
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Skip confirmation prompts
    #[arg(long, global = true)]
    pub yes: bool,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,
}
