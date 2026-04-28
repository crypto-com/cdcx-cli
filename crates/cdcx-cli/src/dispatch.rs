use crate::cli_builder;
use crate::global::GlobalFlags;
use crate::groups::schema::SchemaSubcommand;
use crate::groups::stream::StreamSubcommand;
use crate::mcp::server::CdcxMcpServer;
use cdcx_core::api_client::ApiClient;
use cdcx_core::auth::Credentials;
use cdcx_core::env::Environment;
use cdcx_core::error::CdcxError;
use cdcx_core::output::OutputFormat;
use cdcx_core::safety::{dry_run_output, should_prompt, SafetyTier};
use cdcx_core::sanitize::validate_json_payload;
use cdcx_core::schema::SchemaRegistry;
use cdcx_core::ws_client::WsClient;
use std::io::IsTerminal;

/// Wrapper for private API requests that handles --json merge, --dry-run, and confirmation prompts.
async fn dispatch_private(
    client: &ApiClient,
    method: &str,
    mut params: serde_json::Value,
    global: &GlobalFlags,
    format: OutputFormat,
) -> Result<serde_json::Value, CdcxError> {
    // Merge --json input if provided (overrides CLI-derived params)
    if let Some(ref json_input) = global.json_input {
        let json_params: serde_json::Value = serde_json::from_str(json_input)
            .map_err(|e| CdcxError::Config(format!("Invalid --json input: {}", e)))?;
        if let Some(obj) = json_params.as_object() {
            let target = params
                .as_object_mut()
                .ok_or_else(|| CdcxError::Config("Expected object params".into()))?;
            for (k, v) in obj {
                target.insert(k.clone(), v.clone());
            }
        }
    }

    // Validate params against adversarial input patterns
    validate_json_payload(&params).map_err(CdcxError::Api)?;

    // Dry run: show what would be sent without executing
    if global.dry_run {
        return Ok(dry_run_output(method, &params));
    }

    // Safety confirmation prompt (only in table/human mode)
    let tier = SafetyTier::from_method(method);
    let is_tty = std::io::stderr().is_terminal();
    if should_prompt(tier, is_tty, global.yes, global.dry_run, format) {
        eprint!("⚠ This will execute {} — continue? [y/N] ", method);
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(CdcxError::Io)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            return Err(CdcxError::Config("Operation cancelled by user".into()));
        }
    }

    client.request(method, params).await
}

/// Wrapper for public API requests that handles --json merge and --dry-run.
/// Public requests are always Read tier, so no confirmation prompt needed.
async fn dispatch_public(
    client: &ApiClient,
    method: &str,
    mut params: serde_json::Value,
    global: &GlobalFlags,
) -> Result<serde_json::Value, CdcxError> {
    // Merge --json input if provided
    if let Some(ref json_input) = global.json_input {
        let json_params: serde_json::Value = serde_json::from_str(json_input)
            .map_err(|e| CdcxError::Config(format!("Invalid --json input: {}", e)))?;
        if let Some(obj) = json_params.as_object() {
            let target = params
                .as_object_mut()
                .ok_or_else(|| CdcxError::Config("Expected object params".into()))?;
            for (k, v) in obj {
                target.insert(k.clone(), v.clone());
            }
        }
    }

    // Validate params against adversarial input patterns
    validate_json_payload(&params).map_err(CdcxError::Api)?;

    // Dry run
    if global.dry_run {
        return Ok(dry_run_output(method, &params));
    }

    client.public_request(method, params).await
}

/// Load config from ~/.config/cdcx/config.toml if it exists.
fn load_config() -> Result<Option<cdcx_core::config::Config>, CdcxError> {
    cdcx_core::config::Config::load_default()
}

/// Resolve environment: CLI flag > env var > config file > default (Production).
pub(crate) fn resolve_environment(global: &GlobalFlags) -> Result<Environment, CdcxError> {
    let config = load_config()?;
    Environment::resolve(
        global.env.as_deref(),
        config.as_ref(),
        global.profile.as_deref(),
    )
}

/// Resolve credentials, but allow None when --dry-run is set (dry-run doesn't need real creds).
fn resolve_credentials(global: &GlobalFlags) -> Result<Option<Credentials>, CdcxError> {
    if global.dry_run {
        return Ok(None);
    }
    let config = load_config()?;
    if config.is_some() {
        if let Some(path) = cdcx_core::config::Config::default_path() {
            cdcx_core::config::check_config_permissions(&path)?;
        }
    }
    Credentials::resolve(config.as_ref(), global.profile.as_deref()).map(Some)
}

/// Schema-driven dynamic dispatch for all API groups.
/// Replaces the 10 hand-coded run_* functions.
/// Same pattern as MCP's call_tool, but for CLI.
pub async fn dispatch_dynamic(
    group: &str,
    group_matches: &clap::ArgMatches,
    global: &GlobalFlags,
    registry: &SchemaRegistry,
    format: OutputFormat,
) -> Result<(serde_json::Value, String), CdcxError> {
    let (command, cmd_matches) = group_matches
        .subcommand()
        .ok_or_else(|| CdcxError::Config(format!("No subcommand provided for '{}'", group)))?;

    // Find endpoint in schema
    let endpoints = registry.get_by_group(group);
    let endpoint = endpoints
        .iter()
        .find(|ep| ep.command == command)
        .ok_or_else(|| CdcxError::Config(format!("Unknown command: {} {}", group, command)))?;

    // Extract params from ArgMatches using schema types
    let mut params = cli_builder::extract_params(cmd_matches, &endpoint.params);

    // Stamp client_oid with the cx1- CLI origin prefix so orders placed via `cdcx`
    // are identifiable downstream. Both create-order and advanced/create-order use
    // a scalar client_oid; create-order-list puts client_oid on each leg.
    // See cdcx-core::origin for the prefix scheme.
    use cdcx_core::origin::{tag_order_list_legs, tag_params_in_place, OriginChannel};
    let is_single_order = endpoint.method == "private/create-order"
        || endpoint.method == "private/advanced/create-order";
    let is_order_list = endpoint.method == "private/create-order-list"
        || endpoint.method == "private/create-oco-order"
        || endpoint.method == "private/create-oto-order"
        || endpoint.method == "private/create-otoco-order";
    if is_single_order {
        if let Some(tagged) = tag_params_in_place(&mut params, OriginChannel::Cli) {
            if tagged.truncated {
                eprintln!("warning: client_oid truncated to fit 36-char limit after cdcx prefix");
            }
        }
    } else if is_order_list {
        let truncations = tag_order_list_legs(&mut params, OriginChannel::Cli);
        if truncations > 0 {
            eprintln!(
                "warning: {} client_oid leg(s) truncated to fit 36-char limit after cdcx prefix",
                truncations
            );
        }
    }

    // Create client and dispatch based on auth requirement
    let env = resolve_environment(global)?;

    let method = endpoint.method.clone();
    if endpoint.auth_required {
        let credentials = resolve_credentials(global)?;
        let client = ApiClient::new(credentials, env);
        dispatch_private(&client, &endpoint.method, params, global, format)
            .await
            .map(|data| (data, method))
    } else {
        let client = ApiClient::new(None, env);
        dispatch_public(&client, &endpoint.method, params, global)
            .await
            .map(|data| (data, method))
    }
}

pub async fn run_schema(
    registry: Option<&SchemaRegistry>,
    cmd: &SchemaSubcommand,
) -> Result<serde_json::Value, CdcxError> {
    let require_registry = || -> Result<&SchemaRegistry, CdcxError> {
        registry.ok_or_else(|| {
            CdcxError::Config(
                "No API schema cached. Run 'cdcx setup' or 'cdcx schema update' first.".into(),
            )
        })
    };

    match cmd {
        SchemaSubcommand::List { group: None } => {
            let registry = require_registry()?;
            let endpoints = registry.list_all();
            let summaries: Vec<_> = endpoints
                .iter()
                .map(|ep| {
                    serde_json::json!({
                        "method": ep.method,
                        "command": ep.command,
                        "group": &ep.group,
                        "description": ep.description,
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(summaries))
        }
        SchemaSubcommand::List { group: Some(g) } => {
            let registry = require_registry()?;
            let endpoints = registry.get_by_group(g);
            let summaries: Vec<_> = endpoints
                .iter()
                .map(|ep| {
                    serde_json::json!({
                        "method": ep.method,
                        "command": ep.command,
                        "group": g,
                        "description": ep.description,
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(summaries))
        }
        SchemaSubcommand::Show { method } => {
            let registry = require_registry()?;
            registry
                .get_by_method(method)
                .ok_or_else(|| CdcxError::Config(format!("Method not found: {}", method)))
                .and_then(|schema| {
                    serde_json::to_value(schema).map_err(|e| {
                        CdcxError::Config(format!("Failed to serialize schema: {}", e))
                    })
                })
        }
        SchemaSubcommand::Catalog => {
            let registry = require_registry()?;
            let endpoints = registry.list_all();
            let catalog: Vec<_> = endpoints
                .iter()
                .filter_map(|ep| serde_json::to_value(ep).ok())
                .collect();
            Ok(serde_json::Value::Array(catalog))
        }
        SchemaSubcommand::Update => {
            let fetcher = cdcx_core::openapi::fetcher::SpecFetcher::default();
            eprintln!("Fetching OpenAPI spec...");
            let spec = fetcher
                .fetch_remote()
                .await
                .map_err(|e| CdcxError::Config(format!("Failed to fetch spec: {}", e)))?;

            let parsed = cdcx_core::openapi::parser::parse_openapi_spec(&spec)
                .map_err(|e| CdcxError::Config(format!("Failed to parse spec: {}", e)))?;

            let prev_count = fetcher.previous_endpoint_count();
            let new_count = parsed.endpoints.len();

            fetcher.write_cache(&spec).map_err(CdcxError::Io)?;
            fetcher.write_meta(new_count).map_err(CdcxError::Io)?;

            if let Some(prev) = prev_count {
                if prev != new_count {
                    eprintln!("Schema updated: {} -> {} endpoints", prev, new_count);
                }
            }
            eprintln!("Cache updated: {} endpoints", new_count);

            Ok(serde_json::json!({
                "status": "updated",
                "endpoint_count": new_count,
                "cache_path": fetcher.cache_path.to_string_lossy().to_string(),
            }))
        }
        SchemaSubcommand::Status => {
            let fetcher = cdcx_core::openapi::fetcher::SpecFetcher::default();
            let fresh = fetcher.cache_is_fresh();
            let endpoint_count = fetcher.previous_endpoint_count();
            let cache_exists = fetcher.cache_path.exists();
            let cache_age = if cache_exists {
                std::fs::metadata(&fetcher.cache_path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                    .map(|d| format!("{}h {}m", d.as_secs() / 3600, (d.as_secs() % 3600) / 60))
            } else {
                None
            };

            Ok(serde_json::json!({
                "cache_path": fetcher.cache_path.to_string_lossy().to_string(),
                "cache_exists": cache_exists,
                "cache_fresh": fresh,
                "cache_age": cache_age,
                "endpoint_count": endpoint_count,
                "source": if cache_exists && fresh { "openapi" } else { "toml-fallback" },
            }))
        }
    }
}

/// Handle a subscribe ack (a `{"method":"subscribe",...}` message with no result payload).
///
/// Returns an error if the exchange reported a non-zero code (e.g. unknown instrument,
/// malformed channel) so the user gets a fast failure instead of an infinite wait.
/// On success, prints a one-line status to stderr, keeping stdout clean for pipes.
fn handle_subscribe_ack(value: &serde_json::Value) -> Result<(), CdcxError> {
    let code = value.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    let channel = value
        .get("channel")
        .and_then(|c| c.as_str())
        .unwrap_or("<unknown>");

    if code != 0 {
        let msg = value
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("subscribe failed");
        return Err(CdcxError::Api(cdcx_core::error::ErrorEnvelope::api(
            code,
            &format!("subscribe to {} failed: {}", channel, msg),
        )));
    }

    eprintln!(
        "subscribed to {} — waiting for data (Ctrl+C to stop)",
        channel
    );
    if channel.starts_with("settlement.") {
        eprintln!(
            "note: settlement data is only pushed when the contract settles, \
             not on a heartbeat. Expect silence between settlement events."
        );
    }
    Ok(())
}

pub async fn run_stream(
    global: &crate::global::GlobalFlags,
    cmd: &StreamSubcommand,
    env: Environment,
    format: OutputFormat,
) -> Result<serde_json::Value, CdcxError> {
    use cdcx_core::sanitize::validate_input;

    let is_user_channel = matches!(
        cmd,
        StreamSubcommand::Orders
            | StreamSubcommand::UserTrades
            | StreamSubcommand::Balance
            | StreamSubcommand::Positions
            | StreamSubcommand::AccountRisk
    );

    // Helper: validate instrument names before formatting into channel strings
    let validate_instruments = |instruments: &[String]| -> Result<(), CdcxError> {
        for i in instruments {
            validate_input("instrument", i).map_err(CdcxError::Api)?;
        }
        Ok(())
    };

    let (channels, ws_url) = match cmd {
        StreamSubcommand::Ticker { instruments } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments
                .iter()
                .map(|i| format!("ticker.{}", i))
                .collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Book { instruments, depth } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments
                .iter()
                .map(|i| format!("book.{}.{}", i, depth))
                .collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Trades { instruments } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments.iter().map(|i| format!("trade.{}", i)).collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Candlestick {
            instruments,
            interval,
        } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments
                .iter()
                .map(|i| format!("candlestick.{}.{}", interval, i))
                .collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Index { instruments } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments.iter().map(|i| format!("index.{}", i)).collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Mark { instruments } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments.iter().map(|i| format!("mark.{}", i)).collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Settlement { instruments } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments
                .iter()
                .map(|i| format!("settlement.{}", i))
                .collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Funding { instruments } => {
            validate_instruments(instruments)?;
            let ch: Vec<String> = instruments
                .iter()
                .map(|i| format!("funding.{}", i))
                .collect();
            (ch, env.ws_market_url().to_string())
        }
        StreamSubcommand::Orders => (vec!["user.order".into()], env.ws_user_url().to_string()),
        StreamSubcommand::UserTrades => (vec!["user.trade".into()], env.ws_user_url().to_string()),
        StreamSubcommand::Balance => (vec!["user.balance".into()], env.ws_user_url().to_string()),
        StreamSubcommand::Positions => {
            (vec!["user.positions".into()], env.ws_user_url().to_string())
        }
        StreamSubcommand::AccountRisk => (
            vec!["user.account_risk".into()],
            env.ws_user_url().to_string(),
        ),
    };

    if global.dry_run {
        let payload = serde_json::json!({
            "dry_run": true,
            "channels": channels,
            "ws_url": ws_url,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return Ok(payload);
    }

    let mut client = if is_user_channel {
        let config = load_config()?;
        if config.is_some() {
            if let Some(path) = cdcx_core::config::Config::default_path() {
                cdcx_core::config::check_config_permissions(&path)?;
            }
        }
        let credentials = Credentials::resolve(config.as_ref(), global.profile.as_deref())?;
        WsClient::authenticated_connect(&ws_url, &credentials).await?
    } else {
        WsClient::connect(&ws_url).await?
    };

    client.subscribe(channels).await?;

    // Stream messages, handle SIGINT
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut header_printed = false;

    loop {
        tokio::select! {
            msg = client.next_message() => {
                match msg {
                    Some(Ok(value)) => {
                        // Subscribe acks carry no result payload — route them through the
                        // ack handler so non-zero codes surface instead of hanging silently.
                        if value.get("method").and_then(|m| m.as_str()) == Some("subscribe")
                            && value.get("result").is_none()
                        {
                            handle_subscribe_ack(&value)?;
                            continue;
                        }
                        match format {
                            OutputFormat::Table => {
                                if let Some(result) = value.get("result") {
                                    let channel = result.get("channel")
                                        .and_then(|c| c.as_str()).unwrap_or("");
                                    if !header_printed {
                                        if let Some(header) = cdcx_core::tables::stream_header(channel) {
                                            println!("{}", header);
                                        }
                                        header_printed = true;
                                    }
                                    if let Some(rows) = cdcx_core::tables::format_stream_rows(channel, result) {
                                        println!("{}", rows);
                                    }
                                }
                            }
                            _ => {
                                println!("{}", serde_json::to_string(&value).unwrap_or_default());
                            }
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("{}", serde_json::to_string(&serde_json::json!({"error": e.to_string()})).unwrap());
                        break;
                    }
                    None => break, // Connection closed
                }
            }
            _ = &mut ctrl_c => {
                let _ = client.close().await;
                break;
            }
        }
    }

    // Stream commands don't return a normal result - they output NDJSON directly
    // Return empty object to satisfy type signature
    Ok(serde_json::json!({"stream": "completed"}))
}

pub async fn run_update(check_only: bool) -> Result<(), CdcxError> {
    use cdcx_core::update::{download_and_install, is_newer, UpdateChecker};

    let current = env!("CARGO_PKG_VERSION");
    eprintln!("Current version: {}", current);
    eprintln!("Checking for updates...");

    let checker = UpdateChecker::default();
    let info = checker
        .fetch_latest()
        .await
        .map_err(|_| CdcxError::Config("Check updates failed, try again later".into()))?;

    if !is_newer(&info.version, current) {
        eprintln!("Already up to date ({})", current);
        return Ok(());
    }

    eprintln!("New version available: {} → {}", current, info.version);

    if check_only {
        eprintln!("Release: {}", info.html_url);
        return Ok(());
    }

    if info.download_url.is_none() {
        eprintln!(
            "No pre-built binary for {} — download manually:",
            cdcx_core::update::current_target()
        );
        eprintln!("  {}", info.html_url);
        return Ok(());
    }

    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    if is_tty {
        eprint!("Install version {}? [y/N] ", info.version);
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(CdcxError::Io)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Update cancelled.");
            return Ok(());
        }
    }

    eprintln!(
        "Downloading {}...",
        info.asset_name.as_deref().unwrap_or("update")
    );

    download_and_install(&info)
        .await
        .map_err(|e| CdcxError::Config(format!("Update failed: {e}")))?;

    eprintln!(
        "Updated to {} — restart cdcx to use the new version.",
        info.version
    );
    Ok(())
}

pub async fn run_mcp(
    services: String,
    allow_dangerous: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use cdcx_core::auth::Credentials;
    use rmcp::ServiceExt;

    let service_groups: Vec<String> = services.split(',').map(|s| s.trim().to_string()).collect();

    // Resolve credentials if needed (for private endpoints)
    // Most service groups (except "market") have private endpoints
    let needs_auth = service_groups.iter().any(|g| g != "market");
    let config = load_config()?;
    let env = Environment::resolve(None, config.as_ref(), None).unwrap_or(Environment::Production);
    let api_client = if needs_auth {
        if config.is_some() {
            if let Some(path) = cdcx_core::config::Config::default_path() {
                cdcx_core::config::check_config_permissions(&path)?;
            }
        }
        // Try to resolve credentials from environment or config
        match Credentials::resolve(config.as_ref(), None) {
            Ok(creds) => Some(cdcx_core::api_client::ApiClient::new(Some(creds), env)),
            Err(_) => {
                // If credentials cannot be resolved, continue without authentication
                // (public endpoints will work, private will fail with a clear error)
                None
            }
        }
    } else {
        None
    };

    let has_auth = api_client.is_some();
    let services_display = service_groups.join(", ");
    let server = CdcxMcpServer::new(api_client, service_groups, allow_dangerous, env)?;

    // Report startup status to stderr (stdout is reserved for MCP JSON-RPC)
    let tool_count =
        crate::mcp::tools::generate_tools(server.schema_registry(), server.service_groups()).len();
    let auth_status = if has_auth {
        "authenticated"
    } else {
        "public-only"
    };
    eprintln!("cdcx MCP server starting...");
    eprintln!("  services:  {}", services_display);
    eprintln!("  tools:     {}", tool_count);
    eprintln!("  auth:      {}", auth_status);
    if allow_dangerous {
        eprintln!("  dangerous: enabled");
    }
    eprintln!("  transport: stdio");
    eprintln!("Server ready.");

    // Start the MCP server using rmcp's ServiceExt trait
    let running = server.serve(rmcp::transport::io::stdio()).await?;
    running.waiting().await?;

    Ok(())
}
