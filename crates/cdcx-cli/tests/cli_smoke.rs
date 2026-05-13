use assert_cmd::Command;
use predicates::str;
use std::fs;

/// Returns true if a cached OpenAPI spec exists.
/// Integration tests that exercise dynamic API groups require this.
fn has_cached_spec() -> bool {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("cdcx");
    cache_dir.join("openapi-spec.yaml").exists()
}

macro_rules! require_spec {
    () => {
        if !has_cached_spec() {
            eprintln!("SKIPPED: no cached OpenAPI spec (run 'cdcx schema update' first)");
            return;
        }
    };
}

#[test]
fn test_help_exits_0() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
}

#[test]
fn test_market_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["market", "--help"])
        .assert()
        .success();
}

#[test]
fn test_market_ticker_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["market", "ticker", "--help"])
        .assert()
        .success();
}

#[test]
fn test_schema_list() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["schema", "list"])
        .assert()
        .success()
        .stdout(str::contains("public/get-tickers"));
}

#[test]
fn test_schema_show() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["schema", "show", "public/get-tickers"])
        .assert()
        .success()
        .stdout(str::contains("instrument_name"));
}

#[test]
fn test_schema_catalog() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["schema", "catalog"])
        .assert()
        .success();
}

#[test]
fn test_market_book_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["market", "book", "--help"])
        .assert()
        .success();
}

#[test]
fn test_market_instruments_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["market", "instruments", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_ticker_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "ticker", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_book_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "book", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_trades_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "trades", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_candlestick_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "candlestick", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_index_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "index", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_mark_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "mark", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_settlement_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "settlement", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_funding_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "funding", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_orders_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "orders", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_user_trades_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "user-trades", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_balance_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "balance", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_positions_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "positions", "--help"])
        .assert()
        .success();
}

#[test]
fn test_stream_account_risk_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["stream", "account-risk", "--help"])
        .assert()
        .success();
}

#[test]
fn test_mcp_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "--help"])
        .assert()
        .success();
}

#[test]
fn test_mcp_server_initialize() {
    require_spec!();
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command as StdCommand, Stdio};

    let mut child = StdCommand::new(env!("CARGO_BIN_EXE_cdcx"))
        .args(["mcp", "--services", "market"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start MCP server");

    // Write initialize request
    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        let initialize_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test-client",
                    "version": "0.1.0"
                }
            }
        });
        writeln!(stdin, "{initialize_request}").expect("Failed to write to stdin");
        let _ = stdin; // Close stdin to signal EOF
    }

    // Read the response
    let stdout = child.stdout.take().expect("Failed to get stdout");
    let mut reader = BufReader::new(stdout);
    let mut response = String::new();
    let _ = reader.read_line(&mut response);

    // Terminate the server
    let _ = child.kill();
    let _ = child.wait();

    // Check that the response contains the server info
    assert!(
        response.contains("cdcx"),
        "Response should contain server name 'cdcx'. Got: {}",
        response
    );
    assert!(
        response.contains("protocolVersion"),
        "Response should contain protocolVersion. Got: {}",
        response
    );
}

#[test]
fn test_account_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["account", "--help"])
        .assert()
        .success();
}

#[test]
fn test_trade_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["trade", "--help"])
        .assert()
        .success();
}

#[test]
fn test_wallet_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["wallet", "--help"])
        .assert()
        .success();
}

#[test]
fn test_history_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["history", "--help"])
        .assert()
        .success();
}

#[test]
fn test_advanced_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["advanced", "--help"])
        .assert()
        .success();
}

#[test]
fn test_fiat_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["fiat", "--help"])
        .assert()
        .success();
}

#[test]
fn test_staking_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["staking", "--help"])
        .assert()
        .success();
}

#[test]
fn test_margin_help() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["margin", "--help"])
        .assert()
        .success();
}

#[test]
fn test_positional_args_work() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["market", "book", "ETH_USDT", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ETH_USDT"))
        .stdout(predicates::str::contains("public/get-book"));
}

#[test]
fn test_trade_order_rejects_missing_type() {
    require_spec!();
    // Regression: the `type` param used to default to MARKET via the trade
    // overlay, which silently promoted `--price` limit orders into market
    // fills. The overlay now omits that default — per the OpenAPI spec,
    // `type` is required with no server default. The CLI must refuse to
    // submit the command, not silently pick the more destructive enum value.
    Command::cargo_bin("cdcx")
        .unwrap()
        .args([
            "trade",
            "order",
            "BUY",
            "BTC_USDT",
            "0.01",
            "--price",
            "50000",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--type"));
}

#[test]
fn test_dry_run_public() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["market", "ticker", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains("dry_run"))
        .stdout(predicates::str::contains("public/get-tickers"));
}

#[test]
fn test_dry_run_private_no_creds() {
    require_spec!();
    // --dry-run should work even without credentials for private endpoints
    // Overlay: side=pos0, instrument_name=pos1, quantity=pos2
    Command::cargo_bin("cdcx")
        .unwrap()
        .args([
            "trade",
            "order",
            "BUY",
            "BTC_USDT",
            "0.01",
            "--type",
            "LIMIT",
            "--price",
            "50000",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("dry_run"))
        .stdout(predicates::str::contains("private/create-order"));
}

#[test]
fn test_json_merge_dry_run() {
    require_spec!();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args([
            "market",
            "ticker",
            "--json",
            r#"{"instrument_name":"BTC_USDT"}"#,
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("BTC_USDT"))
        .stdout(predicates::str::contains("dry_run"));
}

#[test]
fn test_update_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(str::contains("--check"))
        .stdout(str::contains("--disable"))
        .stdout(str::contains("--enable"));
}

#[test]
fn test_update_help_hides_global_flags() {
    let output = Command::cargo_bin("cdcx")
        .unwrap()
        .args(["update", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(!stdout.contains("--dry-run"), "should hide --dry-run");
    assert!(!stdout.contains("--json"), "should hide --json");
    assert!(!stdout.contains("--profile"), "should hide --profile");
    assert!(!stdout.contains("--output"), "should hide --output");
    assert!(!stdout.contains("--env"), "should hide --env");
    assert!(!stdout.contains("--yes"), "should hide --yes");
}

#[test]
#[cfg(unix)]
fn test_update_disable_writes_config() {
    let dir = tempfile::tempdir().unwrap();

    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["update", "--disable"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("disabled"));

    let content = fs::read_to_string(dir.path().join(".config/cdcx/config.toml")).unwrap();
    assert!(content.contains("disable_update_check = true"));
}

#[test]
#[cfg(unix)]
fn test_update_enable_after_disable() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join(".config/cdcx");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "disable_update_check = true\n",
    )
    .unwrap();

    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["update", "--enable"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("enabled"));

    let content = fs::read_to_string(config_dir.join("config.toml")).unwrap();
    assert!(content.contains("disable_update_check = false"));
}

// --- MCP config tests ---

#[test]
fn test_mcp_config_help() {
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config", "--help"])
        .assert()
        .success()
        .stdout(str::contains("--enable"))
        .stdout(str::contains("--disable"))
        .stdout(str::contains("--reset"));
}

#[test]
#[cfg(unix)]
fn test_mcp_config_show_default() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("market"))
        .stderr(str::contains("allow_dangerous: false"));
}

#[test]
#[cfg(unix)]
fn test_mcp_config_enable_disable() {
    let dir = tempfile::tempdir().unwrap();

    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config", "--enable", "trade"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("Enabled service: trade"));

    let content = fs::read_to_string(dir.path().join(".config/cdcx/mcp.toml")).unwrap();
    assert!(content.contains("trade"));

    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config", "--disable", "trade"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("Disabled service: trade"));

    let content = fs::read_to_string(dir.path().join(".config/cdcx/mcp.toml")).unwrap();
    assert!(!content.contains("trade"));
}

#[test]
#[cfg(unix)]
fn test_mcp_config_allow_dangerous() {
    let dir = tempfile::tempdir().unwrap();

    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config", "--allow-dangerous"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("enabled"));

    let content = fs::read_to_string(dir.path().join(".config/cdcx/mcp.toml")).unwrap();
    assert!(content.contains("allow_dangerous = true"));

    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config", "--no-dangerous"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("disabled"));

    let content = fs::read_to_string(dir.path().join(".config/cdcx/mcp.toml")).unwrap();
    assert!(content.contains("allow_dangerous = false"));
}

#[test]
#[cfg(unix)]
fn test_mcp_config_reset() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join(".config/cdcx");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("mcp.toml"),
        "services = [\"market\", \"trade\"]\nallow_dangerous = true\n",
    )
    .unwrap();

    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config", "--reset"])
        .env("HOME", dir.path())
        .assert()
        .success()
        .stderr(str::contains("reset to defaults"));

    assert!(!config_dir.join("mcp.toml").exists());
}

#[test]
#[cfg(unix)]
fn test_mcp_config_invalid_service() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("cdcx")
        .unwrap()
        .args(["mcp", "config", "--enable", "bogus"])
        .env("HOME", dir.path())
        .assert()
        .failure()
        .stderr(str::contains("Unknown service group"));
}
