# cdcx Agent Integration Guide

This guide provides comprehensive integration documentation for AI agents using the `cdcx` CLI and MCP server.

## Table of Contents

1. [Authentication](#authentication)
2. [Error Handling](#error-handling)
3. [Rate Limiting](#rate-limiting)
4. [Safety Tiers](#safety-tiers)
5. [MCP Server Setup](#mcp-server-setup)
6. [Example Workflows](#example-workflows)
7. [Best Practices](#best-practices)

## Authentication

### Credential Resolution

The CLI resolves credentials in this order:

1. **Environment variables** (highest priority)
   ```bash
   export CDC_API_KEY="ABC123"
   export CDC_API_SECRET="XYZ789"
   cdcx account info
   ```

2. **Config file** (lowest priority)
   ```toml
   # ~/.config/cdcx/config.toml
   [profiles.default]
   api_key = "ABC123"
   api_secret = "XYZ789"
   ```

### Verification

Before executing trading or account operations, verify authentication:

```bash
cdcx account info
```

Expected response on success:
```json
{"status":"ok","result":{...account data...}}
```

Expected response on auth failure:
```json
{
  "status":"error",
  "error":{
    "category":"auth",
    "code":10002,
    "message":"Invalid API key",
    "retryable":false
  }
}
```

### For Agents Using MCP

When running the MCP server, provide credentials in the initial setup:

```bash
cdcx mcp --services market,trade,account
```

The server will attempt to resolve credentials from the environment or config file. If credentials are not available for private endpoints, the MCP server will reject requests with an auth error.

## Error Handling

### Error Categories

All errors returned by cdcx fall into well-defined categories. Each error envelope includes:

- `category` — Error classification (see table below)
- `code` — CDC exchange error code (0 for non-CDC errors)
- `message` — Human-readable error message
- `retryable` — Whether the operation should be retried

| Category | Description | CDC Codes | Retryable | Guidance |
|----------|-------------|-----------|-----------|----------|
| `auth` | Authentication or authorization failure | 10002, 10003, 10006, 10007, 40101, 40102 | No | Check API key and secret. Verify permissions on the key. |
| `validation` | Invalid parameters or request format | 10001, 10004, 10005, 10008, 10009 | No | Check parameter types and values. Use `cdcx schema show <method>` to verify. |
| `safety` | Operation blocked by safety rails | N/A | No | Set `acknowledged=true` in parameters for mutate-tier commands. |
| `rate_limit` | Too many requests (HTTP 429) | 42901 | Yes | Implement exponential backoff (start with 1s, max 32s). |
| `insufficient_funds` | Account balance too low | 20001, 306 | No | Check balance with `cdcx account summary` before retrying. |
| `order_error` | Order rejection or processing failure | 20002, 20005, 20006, 20007, 318 | Maybe | Verify order parameters. Check if order was partially filled. |
| `network` | Connection, timeout, or I/O error | N/A | Yes | Retry with exponential backoff. |
| `api` | API returned non-zero code (unmapped) | Varies | Depends | Check `code` and `message` for guidance. |
| `config` | Configuration or setup error | N/A | No | Fix configuration. Check file paths and syntax. |
| `io` | File or stream I/O error | N/A | Maybe | Check file permissions and disk space. |

### Retry Strategy

Implement exponential backoff for retryable errors:

```python
import time
import subprocess
import json

def run_with_retry(command, max_retries=5):
    delay = 1  # Start with 1 second
    for attempt in range(max_retries):
        result = subprocess.run(command, capture_output=True, text=True)
        output = json.loads(result.stdout)
        
        if output.get("status") == "ok":
            return output["result"]
        
        error = output.get("error", {})
        if not error.get("retryable"):
            raise Exception(f"Non-retryable error: {error}")
        
        if attempt < max_retries - 1:
            print(f"Retry in {delay}s...")
            time.sleep(delay)
            delay = min(delay * 2, 32)  # Cap at 32 seconds
    
    raise Exception("Max retries exceeded")
```

## Rate Limiting

### Limits

The Crypto.com Exchange API enforces rate limits:

- Public endpoints: 100 requests per second per IP
- Private endpoints: 10 requests per second per account

Exceeding limits returns HTTP 429 with error code `42901`.

### Handling

When rate limited:

1. **Exponential Backoff:** Start with 1s delay, double up to 32s max
2. **Batch Operations:** Group multiple operations when possible
3. **Async Patterns:** Use `cdcx stream` for subscriptions instead of polling

Example backoff implementation:

```bash
#!/bin/bash
attempt=0
delay=1
max_retries=5

while [ $attempt -lt $max_retries ]; do
  output=$(cdcx trade order buy BTC_USDT 0.1 --type limit --price 50000)
  status=$(echo "$output" | jq -r '.status')
  
  if [ "$status" = "ok" ]; then
    echo "$output"
    exit 0
  fi
  
  error_code=$(echo "$output" | jq -r '.error.code')
  if [ "$error_code" = "42901" ]; then
    echo "Rate limited, waiting ${delay}s..." >&2
    sleep $delay
    delay=$((delay * 2))
    [ $delay -gt 32 ] && delay=32
    attempt=$((attempt + 1))
  else
    echo "$output" >&2
    exit 1
  fi
done

echo "Max retries exceeded" >&2
exit 1
```

## Safety Tiers

### Tier Definitions

Every endpoint is assigned a safety tier that determines confirmation requirements:

| Tier | Examples | CLI Behavior | MCP Behavior | Requires Acknowledgment |
|------|----------|-------------|-------------|------------------------|
| `read` | Market data, account info | None | Always allowed | No |
| `sensitive_read` | Balances, positions | None | Always allowed | No |
| `mutate` | Place orders, transfer margin | Prompt in table mode | Requires parameter | Yes |
| `dangerous` | Cancel-all, withdrawals | Always prompts | Requires flag | Yes |

### Confirmation Flow

For `mutate` and `dangerous` operations:

1. **CLI with `--yes` flag:** Requires `acknowledged=true` in parameters
   ```bash
   cdcx trade cancel-all BTC_USDT --json-input <<< '{"acknowledged":true}'
   ```

2. **MCP Server:** Tool requires `acknowledged=true` parameter
   ```json
   {
     "tool": "trade_cancel_all",
     "input": {
       "instrument_name": "BTC_USDT",
       "acknowledged": true
     }
   }
   ```

3. **Dangerous operations on MCP:** Require `--allow-dangerous` flag on server startup
   ```bash
   cdcx mcp --services market,trade,wallet --allow-dangerous
   ```

## MCP Server Setup

### Starting the Server

```bash
cdcx mcp --services <GROUP1>,<GROUP2>... [--allow-dangerous]
```

### Service Groups

| Service | CLI Groups | Default | Safety Tier | Description |
|---------|-----------|---------|-------------|-------------|
| `market` | market | Yes | read | Public market data |
| `account` | account, history | No | sensitive_read | Private account data |
| `trade` | trade | No | mutate | Order placement/management |
| `advanced` | advanced | No | mutate | OCO, OTO, OTOCO orders |
| `margin` | margin | No | mutate | Margin transfers and leverage |
| `staking` | staking | No | mutate | Staking operations |
| `funding` | wallet | No | dangerous | Withdrawals and fund transfers |
| `fiat` | fiat | No | dangerous | Fiat deposits/withdrawals |
| `stream` | stream | No | mixed | WebSocket subscriptions |

### Example Configurations

**Read-only (market data only):**
```bash
cdcx mcp --services market
```

**Trading agent (no withdrawals):**
```bash
cdcx mcp --services market,trade,account,history
```

**Full access (dangerous operations enabled):**
```bash
cdcx mcp --services market,trade,account,advanced,margin,staking,wallet,fiat --allow-dangerous
```

### Tool Registration

Each service group exposes tools via MCP. For example, the `trade` service provides:

- `trade_order` — Place an order
- `trade_amend` — Modify an order
- `trade_cancel` — Cancel an order
- `trade_cancel_all` — Cancel all orders

Query the schema for full tool catalog:

```bash
cdcx schema catalog > agents/tool-catalog.json
```

## Example Workflows

### Workflow 1: Check Account Balance

**Objective:** Retrieve current account balance for BTC

**Steps:**

```bash
# Check authentication
cdcx account info -o json

# Get account summary (all currencies)
cdcx account summary -o json

# Get account summary for specific currency
cdcx account summary --currency BTC -o json
```

**Parsing balance:**

```bash
cdcx account summary --currency BTC -o json | jq '.result.accounts[] | select(.currency=="BTC") | .available'
```

### Workflow 2: Place a Limit Order with Validation

**Objective:** Buy 0.1 BTC at limit price 50,000 USDT

**Steps:**

1. **Check current price:**
   ```bash
   cdcx market ticker BTC_USDT -o json | jq '.result[0].m'
   ```

2. **Verify balance:**
   ```bash
   cdcx account summary --currency USDT -o json | jq '.result.accounts[0].available'
   ```

3. **Dry-run (preview) the order:**
   ```bash
   cdcx trade order buy BTC_USDT 0.1 --type limit --price 50000 --dry-run -o json
   ```

4. **Execute the order:**
   ```bash
   cdcx trade order buy BTC_USDT 0.1 --type limit --price 50000 -o json
   ```

5. **Check order status:**
   ```bash
   cdcx trade order-detail --order-id <ORDER_ID> -o json
   ```

### Workflow 3: Monitor Market Data Stream

**Objective:** Stream live ticker data for BTC_USDT and ETH_USDT

**Steps:**

```bash
# Start streaming
cdcx stream ticker BTC_USDT ETH_USDT

# Output (NDJSON):
# {"method":"subscribe",...}
# {"instrument_name":"BTC_USDT","last":"49500.0","h24":"51000.0",...}
# {"instrument_name":"ETH_USDT","last":"3000.0",...}
```

**Parsing with jq:**

```bash
cdcx stream ticker BTC_USDT | jq 'select(.instrument_name=="BTC_USDT") | {price: .last, volume: .v}'
```

### Workflow 4: Cancel All Orders (with safety)

**Objective:** Cancel all open orders for BTC_USDT

**Steps:**

1. **List open orders first:**
   ```bash
   cdcx trade open-orders --instrument-name BTC_USDT -o json
   ```

2. **Cancel all (with confirmation):**
   ```bash
   # Without --yes: prompts for confirmation
   cdcx trade cancel-all BTC_USDT
   
   # With --yes: requires acknowledged flag
   cdcx trade cancel-all BTC_USDT --yes
   ```

3. **Verify cancellation:**
   ```bash
   cdcx trade open-orders --instrument-name BTC_USDT -o json
   ```

## Best Practices

### 1. Always Use `--dry-run` Before Mutating

```bash
# Preview before executing
cdcx trade order buy BTC_USDT 0.1 --type limit --price 50000 --dry-run -o json

# Then execute
cdcx trade order buy BTC_USDT 0.1 --type limit --price 50000 -o json
```

### 2. Validate Parameters with Schema

```bash
# Check endpoint schema
cdcx schema show private/create-order

# Verify parameter names and types before building requests
```

### 3. Parse JSON Strictly

```bash
# Always check for errors first
output=$(cdcx account info -o json)
status=$(echo "$output" | jq -r '.status')

if [ "$status" != "ok" ]; then
  error=$(echo "$output" | jq -r '.error.message')
  echo "Error: $error" >&2
  exit 1
fi

# Then access result
echo "$output" | jq '.result'
```

### 4. Implement Retry Logic for Rate Limits

```bash
# Use exponential backoff for retryable errors
cdcx trade order ... --retry 5 --backoff exponential
```

### 5. Use Profile-Based Configuration for Multi-Account Scenarios

```toml
# ~/.config/cdcx/config.toml
[profiles.trading]
api_key = "..."
api_secret = "..."

[profiles.analytics]
api_key = "..."
api_secret = "..."
```

```bash
cdcx --profile trading account summary
cdcx --profile analytics market ticker
```

### 6. Stream for Real-Time Data

Instead of polling, use WebSocket streams:

```bash
# Inefficient: polling
for i in {1..10}; do
  cdcx market ticker BTC_USDT
  sleep 1
done

# Efficient: streaming
cdcx stream ticker BTC_USDT | head -100
```

### 7. Use Acknowledgment for Safety-Critical Operations

```bash
# For mutate/dangerous operations, always provide acknowledgment
cdcx trade cancel-all BTC_USDT --json-input <<< '{
  "acknowledged": true,
  "instrument_name": "BTC_USDT"
}'
```

### 8. Log All Trades and Orders

```bash
# Retrieve trade history
cdcx history trades --instrument-name BTC_USDT -o json > trades.json

# Retrieve order history
cdcx history orders --instrument-name BTC_USDT -o json > orders.json
```

### 9. Implement Circuit Breakers

```bash
# Example: Stop if losses exceed threshold
position_pnl=$(cdcx account positions --instrument-name BTC_USDT | jq '.result.positions[0].pnl')
if [ $(echo "$position_pnl < -1000" | bc) -eq 1 ]; then
  cdcx trade cancel-all BTC_USDT --yes
  exit 1
fi
```

### 10. Handle Timeouts Gracefully

```bash
# Use --verbose to see connection details
cdcx account info --verbose

# Timeout after 30 seconds
timeout 30 cdcx stream ticker BTC_USDT || echo "Stream timeout"
```

## Integration Checklist

- [ ] Credentials configured (env vars or config file)
- [ ] Authentication verified with `cdcx account info`
- [ ] Error categories understood and handled appropriately
- [ ] Retry logic implemented for rate-limited and network errors
- [ ] Safety tiers understood for all operations
- [ ] `--dry-run` used before executing mutations
- [ ] JSON output parsed correctly
- [ ] Exponential backoff implemented for retries
- [ ] MCP server started with correct service groups (if using MCP)
- [ ] Acknowledgment provided for mutate/dangerous operations
- [ ] Rate limits understood and respected
- [ ] Logging and monitoring in place
