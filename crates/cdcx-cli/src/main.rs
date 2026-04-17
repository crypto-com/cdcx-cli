#![allow(
    clippy::clone_on_copy,
    clippy::redundant_closure,
    clippy::redundant_pattern_matching,
    clippy::result_large_err,
    clippy::to_string_in_format_args
)]

use cdcx_core::output::{format_error, format_success, OutputFormat};
use cdcx_core::schema::SchemaRegistry;
use clap::FromArgMatches;

mod cli_builder;
mod dispatch;
mod global;
mod groups;
mod mcp;

use global::GlobalFlags;
use groups::schema::SchemaCmd;
use groups::stream::StreamCmd;

#[tokio::main]
async fn main() {
    // Cold start: if we have no cached spec yet, fetch it synchronously so the
    // first invocation exposes the full command surface. Without this, a fresh
    // `cdcx --help` would show only the static commands (schema, stream, setup,
    // mcp, paper, tui) and silently omit the ~80 API commands — a bad first
    // impression that forced users to discover `cdcx schema update` manually.
    let registry = match SchemaRegistry::new() {
        Ok(r) => Some(r),
        Err(_) => fetch_schema_sync().await,
    };
    let app = if let Some(ref reg) = registry {
        cli_builder::build_cli(reg)
    } else {
        cli_builder::build_static_cli()
    };
    let matches = app.get_matches();

    // Background refresh of OpenAPI spec if the cache is stale (warm start).
    // Fire-and-forget: the refresh effective the next run, keeps this run fast.
    {
        let fetcher = cdcx_core::openapi::fetcher::SpecFetcher::default();
        if !fetcher.cache_is_fresh() {
            tokio::spawn(async move {
                match fetcher.fetch_remote().await {
                    Ok(spec) => {
                        if let Ok(parsed) = cdcx_core::openapi::parser::parse_openapi_spec(&spec) {
                            let count = parsed.endpoints.len();
                            if let Some(prev) = fetcher.previous_endpoint_count() {
                                if prev != count {
                                    eprintln!(
                                        "Schema updated: {} -> {} endpoints (effective next run)",
                                        prev, count
                                    );
                                }
                            }
                            let _ = fetcher.write_cache(&spec);
                            let _ = fetcher.write_meta(count);
                        }
                    }
                    Err(e) => {
                        if std::env::var("CDC_DEBUG").is_ok() {
                            eprintln!("Warning: failed to refresh OpenAPI spec: {}", e);
                        }
                    }
                }
            });
        }
    }

    let global = GlobalFlags::from_arg_matches(&matches).expect("Failed to parse global flags");
    let format = OutputFormat::resolve(global.output.as_deref());

    // Initialize tracing if verbose flag is set
    if global.verbose {
        tracing_subscriber::fmt()
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    }

    match matches.subcommand() {
        Some(("schema", sub)) => {
            let schema_cmd =
                SchemaCmd::from_arg_matches(sub).expect("Failed to parse schema command");
            match dispatch::run_schema(registry.as_ref(), &schema_cmd.command).await {
                Ok(data) => println!("{}", format_success(&data, format, None, None)),
                Err(e) => {
                    eprintln!("{}", format_error(&e.to_envelope(), format));
                    std::process::exit(1);
                }
            }
        }
        Some(("stream", sub)) => {
            let stream_cmd =
                StreamCmd::from_arg_matches(sub).expect("Failed to parse stream command");
            let env = match dispatch::resolve_environment(&global) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("{}", format_error(&e.to_envelope(), format));
                    std::process::exit(1);
                }
            };
            match dispatch::run_stream(&global, &stream_cmd.command, env, format.clone()).await {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("{}", format_error(&e.to_envelope(), format));
                    std::process::exit(1);
                }
            }
        }
        Some(("tui", sub)) => {
            let env = match dispatch::resolve_environment(&global) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("{}", format_error(&e.to_envelope(), format));
                    std::process::exit(1);
                }
            };
            let theme = sub.get_one::<String>("theme").cloned();
            let setup = sub.get_flag("setup");
            let opts = cdcx_tui::TuiOptions {
                env,
                profile: global.profile.clone(),
                theme,
                setup,
            };
            if let Err(e) = cdcx_tui::run(opts).await {
                eprintln!("TUI error: {}", e);
                std::process::exit(1);
            }
        }
        Some(("paper", sub)) => {
            let paper_cmd = groups::paper::PaperCmd::from_arg_matches(sub)
                .expect("Failed to parse paper command");
            let env = match dispatch::resolve_environment(&global) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("{}", format_error(&e.to_envelope(), format));
                    std::process::exit(1);
                }
            };
            if let Err(e) = groups::paper::run_paper(&paper_cmd.command, env).await {
                eprintln!("{}", format_error(&e.to_envelope(), format));
                std::process::exit(1);
            }
        }
        Some(("setup", _)) => {
            if let Err(e) = groups::setup::run_setup().await {
                eprintln!("{}", format_error(&e.to_envelope(), format));
                std::process::exit(1);
            }
        }
        Some(("mcp", sub)) => {
            let services = sub.get_one::<String>("services").unwrap().clone();
            let allow_dangerous = sub.get_flag("allow-dangerous");
            if let Err(e) = dispatch::run_mcp(services, allow_dangerous).await {
                eprintln!("MCP server error: {}", e);
                std::process::exit(1);
            }
        }
        Some((group, sub)) => {
            // Dynamic API group commands — require registry
            let reg = match &registry {
                Some(r) => r,
                None => {
                    eprintln!("Error: No API schema cached.");
                    eprintln!("Run 'cdcx setup' or 'cdcx schema update' to fetch the API schema.");
                    std::process::exit(1);
                }
            };
            match dispatch::dispatch_dynamic(group, sub, &global, reg, format.clone()).await {
                Ok((data, method)) => {
                    let response_schema = reg.get_response_schema(&method);
                    println!(
                        "{}",
                        format_success(&data, format, Some(&method), response_schema)
                    );
                }
                Err(e) => {
                    eprintln!("{}", format_error(&e.to_envelope(), format));
                    std::process::exit(1);
                }
            }
        }
        None => unreachable!("subcommand_required is set"),
    }
}

/// Fetch the OpenAPI spec synchronously with a short timeout and user-visible
/// progress. Only called on true cold start (no cache). Returns `None` if the
/// network fetch fails or times out — caller falls back to the static CLI and
/// prints a hint so users know how to recover manually.
async fn fetch_schema_sync() -> Option<SchemaRegistry> {
    const TIMEOUT_SECS: u64 = 10;

    eprintln!("Fetching API schema (first run, one-time download)...");

    let fetcher = cdcx_core::openapi::fetcher::SpecFetcher::default();
    let fetch = fetcher.fetch_remote();
    let result = tokio::time::timeout(std::time::Duration::from_secs(TIMEOUT_SECS), fetch).await;

    match result {
        Ok(Ok(spec)) => {
            if let Ok(parsed) = cdcx_core::openapi::parser::parse_openapi_spec(&spec) {
                let _ = fetcher.write_cache(&spec);
                let _ = fetcher.write_meta(parsed.endpoints.len());
            }
            SchemaRegistry::new().ok()
        }
        Ok(Err(e)) => {
            eprintln!(
                "Warning: couldn't download API schema ({}). Only built-in commands are available.",
                e
            );
            eprintln!("Retry once you have network, or run `cdcx schema update` manually.");
            None
        }
        Err(_) => {
            eprintln!(
                "Warning: schema download timed out after {}s. Only built-in commands are available.",
                TIMEOUT_SECS
            );
            eprintln!("Retry once you have network, or run `cdcx schema update` manually.");
            None
        }
    }
}
