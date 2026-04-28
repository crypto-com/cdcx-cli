use crate::global::GlobalFlags;
use crate::groups::schema::SchemaCmd;
use crate::groups::stream::StreamCmd;
#[allow(unused_imports)]
use cdcx_core::schema::{EndpointSchema, ParamSchema, SchemaRegistry};
use clap::Args;

/// Leak a String to get a &'static str. Safe for CLI building — done once at startup.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Convert a ParamSchema into a clap::Arg.
/// - If `position` is set: positional arg (index = position + 1, clap is 1-based)
/// - If `position` is None: --long flag (snake_case name becomes kebab-case)
/// - If `default` is set: applies default_value, param becomes not required from CLI perspective
fn build_arg_from_param(param: &ParamSchema) -> clap::Arg {
    let help = if param.param_type == "json" {
        format!("{} (JSON)", param.description.trim())
    } else {
        param.description.clone()
    };
    let mut arg = clap::Arg::new(leak(param.name.clone())).help(leak(help));

    if let Some(pos) = param.position {
        // Positional arguments are never clap-required — when --json is provided,
        // the full payload comes from JSON and positional args are unnecessary.
        // Validation of required fields happens at the API level, not clap level.
        arg = arg.index(pos as usize + 1); // clap indices are 1-based
    } else {
        // Named --flag argument (snake_case → kebab-case)
        arg = arg.long(leak(param.name.replace('_', "-")));
        if param.required && param.default.is_none() {
            arg = arg.required(true);
        }
    }

    // enum_array: repeatable flag with possible values (e.g. --exec-inst POST_ONLY --exec-inst ISOLATED_MARGIN)
    if param.param_type == "enum_array" {
        arg = arg.action(clap::ArgAction::Append);
        if !param.values.is_empty() {
            let possible: Vec<&'static str> =
                param.values.iter().map(|v| leak(v.clone())).collect();
            arg = arg.value_parser(possible);
        }
    }

    if let Some(ref default) = param.default {
        arg = arg.default_value(leak(default.clone()));
    }

    arg
}

/// Build a clap::Command for a schema group (e.g., "market" → `cdcx market <subcommand>`).
/// Each endpoint becomes a subcommand with args generated from its params.
pub fn build_group_command(registry: &SchemaRegistry, group: &str) -> clap::Command {
    let description = registry.group_description(group).unwrap_or(group);
    let mut cmd = clap::Command::new(leak(group.to_string()))
        .about(leak(description.to_string()))
        .subcommand_required(true)
        .arg_required_else_help(true);

    for endpoint in registry.get_by_group(group) {
        let mut sub = clap::Command::new(leak(endpoint.command.clone()))
            .about(leak(endpoint.description.clone()));
        for param in &endpoint.params {
            sub = sub.arg(build_arg_from_param(param));
        }
        cmd = cmd.subcommand(sub);
    }

    cmd
}

/// Extract API parameters from clap ArgMatches, converting types according to schema.
/// - "string" / "enum" → JSON string
/// - "number" → JSON number (tries i64, then f64, then falls back to string)
/// - "json" → parsed JSON value (arrays, objects); falls back to string on parse error
/// - Omitted optional params are excluded from the result
pub fn extract_params(matches: &clap::ArgMatches, params: &[ParamSchema]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();

    for param in params {
        // enum_array: collect repeated values into a JSON array
        if param.param_type == "enum_array" {
            let values: Vec<&String> = matches
                .get_many::<String>(&param.name)
                .map(|v| v.collect())
                .unwrap_or_default();
            if !values.is_empty() {
                let arr: Vec<serde_json::Value> =
                    values.into_iter().map(|v| serde_json::json!(v)).collect();
                obj.insert(param.name.clone(), serde_json::Value::Array(arr));
            }
            continue;
        }

        let value: Option<&String> = matches.get_one::<String>(&param.name);
        let Some(value_str) = value else { continue };

        let json_value = match param.param_type.as_str() {
            "number" => {
                if let Ok(i) = value_str.parse::<i64>() {
                    serde_json::json!(i)
                } else if let Ok(f) = value_str.parse::<f64>() {
                    serde_json::json!(f)
                } else {
                    serde_json::json!(value_str)
                }
            }
            "json" => {
                serde_json::from_str(value_str).unwrap_or_else(|_| serde_json::json!(value_str))
            }
            _ => serde_json::json!(value_str),
        };
        obj.insert(param.name.clone(), json_value);
    }

    serde_json::Value::Object(obj)
}

/// Build the static CLI command tree without schema-dependent commands.
/// Used when no API schema is cached (first install).
/// Includes: schema, stream, setup, mcp, paper, tui
pub fn build_static_cli() -> clap::Command {
    let mut app = clap::Command::new("cdcx")
        .about("Agent-first CLI for Crypto.com Exchange API")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_required(true)
        .arg_required_else_help(true);

    // Add global flags from derive struct
    app = GlobalFlags::augment_args(app);

    // Static subcommands (derive-based, non-API dispatch)
    app = app.subcommand(
        <SchemaCmd as clap::CommandFactory>::command()
            .name("schema")
            .about("Schema introspection"),
    );
    app = app.subcommand(
        <StreamCmd as clap::CommandFactory>::command()
            .name("stream")
            .about("Stream real-time market and user data"),
    );
    app =
        app.subcommand(clap::Command::new("setup").about("Configure API credentials and profiles"));
    app = app.subcommand(
        clap::Command::new("mcp")
            .about("Start MCP server (stdio transport)")
            .arg(
                clap::Arg::new("services")
                    .long("services")
                    .default_value("market")
                    .help("Comma-separated service groups to expose"),
            )
            .arg(
                clap::Arg::new("allow-dangerous")
                    .long("allow-dangerous")
                    .action(clap::ArgAction::SetTrue)
                    .help("Allow dangerous operations"),
            ),
    );

    // Paper trading subcommand
    app = app.subcommand(
        <crate::groups::paper::PaperCmd as clap::CommandFactory>::command()
            .name("paper")
            .about("Paper trading — local engine with live prices, no auth required"),
    );

    // TUI subcommand
    app = app.subcommand(
        clap::Command::new("tui")
            .about("Launch the interactive terminal dashboard")
            .arg(
                clap::Arg::new("theme")
                    .long("theme")
                    .help("Theme: terminal-pro, cyber-midnight, monochrome, neon, micky-d, amber"),
            )
            .arg(
                clap::Arg::new("setup")
                    .long("setup")
                    .action(clap::ArgAction::SetTrue)
                    .help("Run the setup wizard"),
            ),
    );

    // Update subcommand
    app = app.subcommand(
        clap::Command::new("update")
            .about("Check for and install updates")
            .arg(
                clap::Arg::new("check")
                    .long("check")
                    .action(clap::ArgAction::SetTrue)
                    .help("Only check for updates, don't install"),
            ),
    );

    app
}

/// Build the complete CLI command tree.
/// - Global flags from GlobalFlags derive struct
/// - Static subcommands: schema, stream, mcp (derive-based)
/// - Dynamic subcommands: all API groups from schema registry (builder-based)
pub fn build_cli(registry: &SchemaRegistry) -> clap::Command {
    let mut app = build_static_cli();

    // Dynamic subcommands from schema registry
    for group in registry.groups() {
        app = app.subcommand(build_group_command(registry, group));
    }

    app
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_param(
        name: &str,
        param_type: &str,
        required: bool,
        position: Option<u32>,
        default: Option<&str>,
    ) -> ParamSchema {
        ParamSchema {
            name: name.to_string(),
            param_type: param_type.to_string(),
            required,
            description: format!("Test {}", name),
            values: vec![],
            position,
            default: default.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_positional_required_arg() {
        // Positional args are never clap-required (--json bypasses them).
        // Validation of required fields happens at the API level.
        let param = make_param("instrument_name", "string", true, Some(0), None);
        let arg = build_arg_from_param(&param);
        assert_eq!(arg.get_id().as_str(), "instrument_name");
        assert!(!arg.is_required_set());
        assert!(arg.get_long().is_none());
    }

    #[test]
    fn test_positional_optional_arg() {
        let param = make_param("instrument_name", "string", false, Some(0), None);
        let arg = build_arg_from_param(&param);
        assert!(!arg.is_required_set());
        assert!(arg.get_long().is_none());
    }

    #[test]
    fn test_flag_arg() {
        let param = make_param("page_size", "number", false, None, None);
        let arg = build_arg_from_param(&param);
        assert_eq!(arg.get_long(), Some("page-size"));
        assert!(!arg.is_required_set());
    }

    #[test]
    fn test_arg_with_default() {
        let param = make_param("type", "enum", true, None, Some("MARKET"));
        let arg = build_arg_from_param(&param);
        assert!(!arg.is_required_set());
        assert_eq!(arg.get_long(), Some("type"));
    }

    const TEST_SPEC: &str = include_str!("../../../tests/fixtures/test-spec.yaml");

    fn test_registry() -> SchemaRegistry {
        SchemaRegistry::from_openapi(TEST_SPEC).expect("Failed to parse test fixture")
    }

    #[test]
    fn test_build_group_command_market() {
        let registry = test_registry();
        let cmd = build_group_command(&registry, "market");
        let subs: Vec<_> = cmd
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();
        assert!(subs.contains(&"ticker".to_string()));
        assert!(subs.contains(&"book".to_string()));
        assert!(subs.contains(&"instruments".to_string()));
        assert!(
            subs.len() >= 3,
            "Expected at least 3 market subcommands, got {}",
            subs.len()
        );
    }

    #[test]
    fn test_build_group_command_has_about() {
        let registry = test_registry();
        let cmd = build_group_command(&registry, "market");
        assert!(cmd.get_about().is_some());
    }

    #[test]
    fn test_build_group_command_endpoint_has_args() {
        let registry = test_registry();
        let cmd = build_group_command(&registry, "trade");
        let order_sub = cmd
            .get_subcommands()
            .find(|s| s.get_name() == "order")
            .expect("TOML schema should have 'order' subcommand");
        let arg_names: Vec<_> = order_sub
            .get_arguments()
            .map(|a| a.get_id().as_str().to_string())
            .collect();
        assert!(arg_names.contains(&"instrument_name".to_string()));
        assert!(arg_names.contains(&"quantity".to_string()));
    }

    #[test]
    fn test_extract_params_string() {
        let params = vec![make_param("instrument_name", "string", true, Some(0), None)];
        let cmd = clap::Command::new("test").arg(build_arg_from_param(&params[0]));
        let matches = cmd.try_get_matches_from(["test", "BTC_USDT"]).unwrap();
        let result = extract_params(&matches, &params);
        assert_eq!(result["instrument_name"], "BTC_USDT");
    }

    #[test]
    fn test_extract_params_number() {
        let params = vec![make_param("depth", "number", false, None, None)];
        let cmd = clap::Command::new("test").arg(build_arg_from_param(&params[0]));
        let matches = cmd.try_get_matches_from(["test", "--depth", "50"]).unwrap();
        let result = extract_params(&matches, &params);
        assert_eq!(result["depth"], 50);
    }

    #[test]
    fn test_extract_params_float() {
        let params = vec![make_param("leverage", "number", true, None, None)];
        let cmd = clap::Command::new("test").arg(build_arg_from_param(&params[0]));
        let matches = cmd
            .try_get_matches_from(["test", "--leverage", "5.5"])
            .unwrap();
        let result = extract_params(&matches, &params);
        assert_eq!(result["leverage"], 5.5);
    }

    #[test]
    fn test_extract_params_omits_absent_optional() {
        let params = vec![
            make_param("instrument_name", "string", true, Some(0), None),
            make_param("depth", "number", false, None, None),
        ];
        let cmd = clap::Command::new("test")
            .arg(build_arg_from_param(&params[0]))
            .arg(build_arg_from_param(&params[1]));
        let matches = cmd.try_get_matches_from(["test", "BTC_USDT"]).unwrap();
        let result = extract_params(&matches, &params);
        assert_eq!(result["instrument_name"], "BTC_USDT");
        assert!(result.get("depth").is_none());
    }

    #[test]
    fn test_extract_params_with_default() {
        let params = vec![make_param("type", "enum", true, None, Some("MARKET"))];
        let cmd = clap::Command::new("test").arg(build_arg_from_param(&params[0]));
        let matches = cmd.try_get_matches_from(["test"]).unwrap();
        let result = extract_params(&matches, &params);
        assert_eq!(result["type"], "MARKET");
    }

    #[test]
    fn test_build_cli_has_all_groups() {
        let registry = SchemaRegistry::from_fixture().expect("fixture spec should parse");
        let app = build_cli(&registry);
        let sub_names: Vec<_> = app
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();
        assert!(sub_names.contains(&"market".to_string()));
        assert!(sub_names.contains(&"account".to_string()));
        assert!(sub_names.contains(&"trade".to_string()));
        assert!(sub_names.contains(&"advanced".to_string()));
        assert!(sub_names.contains(&"wallet".to_string()));
        assert!(sub_names.contains(&"fiat".to_string()));
        assert!(sub_names.contains(&"staking".to_string()));
        // Static subcommands (always present)
        assert!(sub_names.contains(&"schema".to_string()));
        assert!(sub_names.contains(&"stream".to_string()));
        assert!(sub_names.contains(&"mcp".to_string()));
    }

    /// Regression: `schemas/trade.toml` used to carry `default = "MARKET"` for
    /// the `type` param, which silently turned `cdcx trade order BUY INST QTY
    /// --price P` into a MARKET order with an ignored price field. This test
    /// pins the fix: without `--type`, the CLI must reject the command; with
    /// `--type LIMIT`, it must parse and the payload must contain `LIMIT`.
    #[test]
    fn test_trade_order_requires_explicit_type() {
        let registry =
            SchemaRegistry::from_fixture_with_overlays().expect("fixture + overlays should parse");
        let app = build_cli(&registry);

        let missing_type = app.clone().try_get_matches_from([
            "cdcx", "trade", "order", "BUY", "BTC_USDT", "0.0001", "--price", "710",
        ]);
        assert!(
            missing_type.is_err(),
            "trade order without --type must fail to parse — otherwise a missing flag becomes a market order"
        );

        let explicit_limit = app
            .clone()
            .try_get_matches_from([
                "cdcx", "trade", "order", "BUY", "BTC_USDT", "0.0001", "--price", "710", "--type",
                "LIMIT",
            ])
            .expect("trade order with --type LIMIT should parse");

        let trade_sub = explicit_limit
            .subcommand_matches("trade")
            .and_then(|m| m.subcommand_matches("order"))
            .expect("subcommand path");
        let trade_endpoint = registry
            .get_by_method("private/create-order")
            .expect("private/create-order must be in registry");
        let payload = extract_params(trade_sub, &trade_endpoint.params);
        assert_eq!(
            payload["type"], "LIMIT",
            "payload type must echo the explicit --type value, not fall back to MARKET"
        );
    }

    #[test]
    fn test_build_cli_has_global_flags() {
        let registry = SchemaRegistry::from_fixture().expect("fixture spec should parse");
        let app = build_cli(&registry);
        let arg_names: Vec<_> = app
            .get_arguments()
            .map(|a| a.get_id().as_str().to_string())
            .collect();
        assert!(arg_names.contains(&"output".to_string()));
        assert!(arg_names.contains(&"dry_run".to_string()));
        assert!(arg_names.contains(&"verbose".to_string()));
        assert!(arg_names.contains(&"env".to_string()));
    }

    #[test]
    fn test_global_flags_round_trip() {
        use crate::global::GlobalFlags;
        use clap::FromArgMatches;

        let registry = SchemaRegistry::from_fixture().expect("fixture spec should parse");
        let app = build_cli(&registry);
        let matches = app
            .try_get_matches_from([
                "cdcx",
                "--output",
                "table",
                "--dry-run",
                "--verbose",
                "market",
                "ticker",
            ])
            .unwrap();

        let global = GlobalFlags::from_arg_matches(&matches).unwrap();
        assert_eq!(global.output.as_deref(), Some("table"));
        assert!(global.dry_run);
        assert!(global.verbose);
        assert!(!global.yes);
    }
}
