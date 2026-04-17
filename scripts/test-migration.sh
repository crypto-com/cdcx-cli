#!/usr/bin/env bash
# OpenAPI Sole Source Migration — Manual Test Script
# Tests ALL breaking changes from the migration.
# Run after: cargo build --release (or use cargo run --)
#
# Usage: ./scripts/test-migration.sh [path-to-cdcx-binary]
# Default: uses cargo run

set -euo pipefail

CDCX="${1:-cargo run --}"
PASS=0
FAIL=0
WARN=0

green() { printf "\033[32m✓ %s\033[0m\n" "$1"; PASS=$((PASS+1)); }
red()   { printf "\033[31m✗ %s\033[0m\n" "$1"; FAIL=$((FAIL+1)); }
yellow(){ printf "\033[33m⚠ %s\033[0m\n" "$1"; WARN=$((WARN+1)); }

# Run a command, check exit code
expect_success() {
    local desc="$1"; shift
    if eval "$CDCX $*" >/dev/null 2>&1; then
        green "$desc"
    else
        red "$desc"
    fi
}

expect_failure() {
    local desc="$1"; shift
    if eval "$CDCX $*" >/dev/null 2>&1; then
        red "$desc (should have failed)"
    else
        green "$desc"
    fi
}

# Check command output contains a string
expect_contains() {
    local desc="$1"; local needle="$2"; shift 2
    local output
    output=$(eval "$CDCX $*" 2>&1) || true
    if echo "$output" | grep -q "$needle"; then
        green "$desc"
    else
        red "$desc — expected '$needle' in output"
    fi
}

echo "============================================"
echo "  OpenAPI Sole Source Migration Test Suite"
echo "============================================"
echo ""

# ─────────────────────────────────────────────
echo "── 1. COLD START (no cached spec) ──"
echo "   Skipping: would require moving your cache."
echo "   Test manually: mv ~/Library/Caches/cdcx/openapi-spec.yaml /tmp/"
echo "   Then verify: cdcx setup --help works, cdcx market ticker fails gracefully."
echo ""

# ─────────────────────────────────────────────
echo "── 2. TOP-LEVEL GROUPS ──"
echo ""

expect_contains "market group exists"          "Commands:"  "market --help"
expect_contains "account group exists"         "Commands:"  "account --help"
expect_contains "trade group exists"           "Commands:"  "trade --help"
expect_contains "advanced group exists"        "Commands:"  "advanced --help"
expect_contains "wallet group exists"          "Commands:"  "wallet --help"
expect_contains "fiat group exists"            "Commands:"  "fiat --help"
expect_contains "staking group exists"         "Commands:"  "staking --help"
expect_contains "history group exists"         "Commands:"  "history --help"
expect_contains "margin group exists"          "Commands:"  "margin --help"
expect_contains "otc group exists (NEW)"       "Commands:"  "otc --help"
expect_contains "paper group exists"           "Commands:"  "paper --help"
echo ""

# ─────────────────────────────────────────────
echo "── 3. REMOVED: BOT COMMANDS ──"
echo ""

expect_failure "bot group is gone"             "bot --help"
expect_failure "bot create is gone"            "bot create --help"
expect_failure "bot list is gone"              "bot list --help"
echo ""

# ─────────────────────────────────────────────
echo "── 4. OCO COMMANDS ──"
echo "   create-oco removed (not in OpenAPI), cancel-oco restored (now in spec)"
echo ""

expect_failure "advanced create-oco is gone"   "advanced create-oco --help"
expect_success "advanced cancel-oco exists"    "advanced cancel-oco --help"
echo ""

# ─────────────────────────────────────────────
echo "── 5. RENAMED: TRADE COMMANDS ──"
echo "   Old → New"
echo ""

expect_failure "trade create-order is gone"    "trade create-order --help"
expect_success "trade order exists"            "trade order --help"

expect_failure "trade cancel-order is gone"    "trade cancel-order --help"
expect_success "trade cancel exists"           "trade cancel --help"

expect_failure "trade cancel-all-orders gone"  "trade cancel-all-orders --help"
expect_success "trade cancel-all exists"       "trade cancel-all --help"

expect_failure "trade amend-order is gone"     "trade amend-order --help"
expect_success "trade amend exists"            "trade amend --help"

expect_success "trade close-position exists"   "trade close-position --help"
expect_success "trade open-orders exists"      "trade open-orders --help"
expect_success "trade order-detail exists"     "trade order-detail --help"
echo ""

# ─────────────────────────────────────────────
echo "── 6. RENAMED: ACCOUNT COMMANDS ──"
echo ""

expect_failure "account balance is gone"       "account balance --help"
expect_success "account summary exists"        "account summary --help"

expect_failure "account accounts is gone"      "account accounts --help"
expect_success "account info exists"           "account info --help"

expect_success "account positions exists"      "account positions --help"
expect_success "account balance-history exists" "account balance-history --help"
echo ""

# ─────────────────────────────────────────────
echo "── 7. MOVED: ACCOUNT → TRADE (OpenAPI tags them under Trading) ──"
echo "   These were 'account' commands, now under 'trade' per OpenAPI spec"
echo ""

expect_failure "account leverage is gone"      "account leverage --help"
expect_success "trade leverage exists"         "trade leverage --help"

expect_failure "account settings is gone"      "account settings --help"
expect_success "trade settings exists"         "trade settings --help"

expect_failure "account fee-rate is gone"      "account fee-rate --help"
expect_success "trade fee-rate exists"         "trade fee-rate --help"
echo ""

# ─────────────────────────────────────────────
echo "── 8. RENAMED: WALLET COMMANDS ──"
echo ""

expect_failure "wallet withdrawal is gone"     "wallet withdrawal --help"
expect_success "wallet withdraw exists"        "wallet withdraw --help"

expect_failure "wallet currency-networks gone" "wallet currency-networks --help"
expect_success "wallet networks exists"        "wallet networks --help"
echo ""

# ─────────────────────────────────────────────
echo "── 9. RENAMED: FIAT COMMANDS ──"
echo ""

expect_failure "fiat transaction-quota gone"   "fiat transaction-quota --help"
expect_success "fiat quota exists"             "fiat quota --help"

expect_failure "fiat transaction-limit gone"   "fiat transaction-limit --help"
expect_success "fiat limit exists"             "fiat limit --help"

expect_failure "fiat bank-accounts is gone"    "fiat bank-accounts --help"
expect_success "fiat accounts exists"          "fiat accounts --help"

expect_success "fiat withdrawal-history exists" "fiat withdrawal-history --help"
echo ""

# ─────────────────────────────────────────────
echo "── 10. RENAMED: STAKING COMMANDS ──"
echo ""

expect_failure "staking staking-position gone" "staking staking-position --help"
expect_success "staking positions exists"      "staking positions --help"

expect_failure "staking staking-instruments"   "staking staking-instruments --help"
expect_success "staking instruments exists"    "staking instruments --help"

expect_failure "staking open-stake is gone"    "staking open-stake --help"
expect_success "staking open-stakes exists"    "staking open-stakes --help"
echo ""

# ─────────────────────────────────────────────
echo "── 11. RENAMED: MARKET COMMANDS ──"
echo ""

expect_failure "market risk-parameters gone"   "market risk-parameters --help"
expect_success "market risk-params exists"     "market risk-params --help"

expect_failure "market expired-settlement-*"   "market expired-settlement-price --help"
expect_success "market settlement-prices"      "market settlement-prices --help"

# announcements was in old TOML but NOT in OpenAPI spec
expect_failure "market announcements is gone"  "market announcements --help"
echo ""

# ─────────────────────────────────────────────
echo "── 12. RENAMED: HISTORY COMMANDS ──"
echo ""

expect_failure "history order-history gone"    "history order-history --help"
expect_success "history orders exists"         "history orders --help"
echo ""

# ─────────────────────────────────────────────
echo "── 13. RENAMED: ADVANCED COMMANDS ──"
echo ""

expect_success "advanced create-oto exists"    "advanced create-oto --help"
expect_success "advanced create-otoco exists"  "advanced create-otoco --help"
expect_success "advanced cancel-oto exists"    "advanced cancel-oto --help"
expect_success "advanced cancel-otoco exists"  "advanced cancel-otoco --help"
expect_success "advanced cancel exists"        "advanced cancel --help"
expect_success "advanced cancel-all exists"    "advanced cancel-all --help"
expect_success "advanced open-orders exists"   "advanced open-orders --help"
expect_success "advanced order-detail exists"  "advanced order-detail --help"
expect_success "advanced order-history exists" "advanced order-history --help"

# These were in old TOML but NOT in OpenAPI spec
expect_failure "advanced create-order gone"    "advanced create-order --help"
echo ""

# ─────────────────────────────────────────────
echo "── 14. NEW: OTC GROUP (from OpenAPI, not in old TOML) ──"
echo ""

expect_success "otc instruments exists"        "otc instruments --help"
expect_success "otc request-deal exists"       "otc request-deal --help"
expect_success "otc open-deals exists"         "otc open-deals --help"
expect_success "otc deal-history exists"       "otc deal-history --help"
echo ""

# ─────────────────────────────────────────────
echo "── 15. POSITIONAL ARGS ──"
echo "   trade order: side(0) instrument_name(1) quantity(2)"
echo "   Old: cdcx trade create-order BTC_USDT --side BUY --quantity 0.01"
echo "   New: cdcx trade order BUY BTC_USDT 0.01"
echo ""

expect_contains "positional args + default type" "private/create-order" \
    "trade order BUY BTC_USDT 0.01 --price 50000 --dry-run"

expect_contains "type defaults to MARKET" "MARKET" \
    "trade order BUY BTC_USDT 0.01 --price 50000 --dry-run"
echo ""

# ─────────────────────────────────────────────
echo "── 16. BREAKING: price IS NOW REQUIRED ──"
echo "   OpenAPI marks price as required. Old TOML had it optional."
echo "   For MARKET orders the exchange ignores it, but CLI now requires it."
echo ""

expect_failure "trade order without --price fails" \
    "trade order BUY BTC_USDT 0.01 --dry-run"
echo ""

# ─────────────────────────────────────────────
echo "── 17. DRY-RUN SMOKE TESTS (no auth needed) ──"
echo ""

expect_contains "market ticker dry-run"    "public/get-tickers"      "market ticker --dry-run"
expect_contains "market book dry-run"      "public/get-book"         "market book BTC_USDT --dry-run"
expect_contains "trade order dry-run"      "private/create-order"    "trade order BUY BTC_USDT 0.01 --price 50000 --dry-run"
expect_contains "wallet networks dry-run"  "private/get-currency-networks"  "wallet networks --dry-run"
expect_contains "staking instruments"      "private/staking/get-staking-instruments" "staking instruments --dry-run"
echo ""

# ─────────────────────────────────────────────
echo "── 18. LIVE SMOKE TESTS (public endpoints, no auth) ──"
echo ""

echo "   These hit the real exchange API. Skip with Ctrl+C if offline."
echo ""

expect_success "market ticker live"        "market ticker BTC_USDT"
expect_success "market instruments live"   "market instruments"
expect_success "market book live"          "market book BTC_USDT --depth 5"
echo ""

# ─────────────────────────────────────────────
echo "── 19. SCHEMA COMMANDS (work without API key) ──"
echo ""

expect_success "schema update works"       "schema update"
expect_success "schema status works"       "schema status"
expect_contains "schema list shows groups" "market"  "schema list"
echo ""

# ─────────────────────────────────────────────
echo "============================================"
echo ""
echo "  Results: $PASS passed, $FAIL failed, $WARN warnings"
echo ""
if [ "$FAIL" -gt 0 ]; then
    echo "  *** FAILURES DETECTED ***"
    echo ""
fi
echo "============================================"
echo ""
echo "MANUAL CHECKS (not automated above):"
echo ""
echo "  1. Cold start test:"
echo "     mv ~/Library/Caches/cdcx/openapi-spec.yaml /tmp/openapi-spec.yaml.bak"
echo "     cdcx setup --help          # Should show help"
echo "     cdcx schema update         # Should fetch spec"
echo "     cdcx market ticker         # Should work now"
echo "     mv /tmp/openapi-spec.yaml.bak ~/Library/Caches/cdcx/openapi-spec.yaml"
echo ""
echo "  2. MCP server test:"
echo "     echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0.1\"}}}' | cdcx mcp --services market"
echo ""
echo "  3. TUI test:"
echo "     cdcx tui   # Verify it launches, check Market tab has data"
echo ""
echo "BREAKING CHANGES SUMMARY:"
echo ""
echo "  REMOVED:"
echo "    - cdcx bot * (all bot commands)"
echo "    - cdcx advanced create-oco / cancel-oco"
echo "    - cdcx advanced create-order"
echo "    - cdcx market announcements"
echo ""
echo "  RENAMED:"
echo "    - trade: create-order→order, cancel-order→cancel, cancel-all-orders→cancel-all, amend-order→amend"
echo "    - account: balance→summary, accounts→info"
echo "    - wallet: withdrawal→withdraw, currency-networks→networks"
echo "    - fiat: transaction-quota→quota, transaction-limit→limit, bank-accounts→accounts"
echo "    - staking: staking-position→positions, staking-instruments→instruments, open-stake→open-stakes"
echo "    - market: risk-parameters→risk-params, expired-settlement-price→settlement-prices"
echo "    - history: order-history→orders"
echo ""
echo "  MOVED (account → trade, per OpenAPI spec tags):"
echo "    - leverage, settings, account-settings, fee-rate, instrument-fee-rate"
echo ""
echo "  BEHAVIOR:"
echo "    - trade order: --price is now REQUIRED (OpenAPI says so)"
echo "    - trade order: positional args are side(0) instrument(1) quantity(2)"
echo "    - Enum params now have values (e.g. side: BUY/SELL) — affects MCP tool schemas"
echo "    - Required fields changed for 5 params (OpenAPI is now authoritative)"
echo ""
echo "  ADDED:"
echo "    - cdcx otc * (9 OTC trading commands)"
echo "    - trade: order-list, cancel-order-list (LIST variants)"
echo "    - trade: account-settings, fee-rate, instrument-fee-rate (moved from account)"
