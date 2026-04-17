# cdcx: Agent Integration Context

This document provides runtime context for AI agents and tools integrating with the `cdcx` CLI.

## Binary Information

- **Binary name:** `cdcx`
- **Version:** Run `cdcx --version` to check (workspace version defined in root Cargo.toml)
- **Purpose:** Agent-first CLI for Crypto.com Exchange API v1

## Invocation

```bash
cdcx [GLOBAL_FLAGS] [COMMAND] [SUBCOMMAND] [OPTIONS]
```

All output is JSON by default. Use `--output table` for human-readable tables (TTY-only).

## Global Flags

```
--output <FORMAT>           Output format: json (default), table
--json-input                Expect JSON from stdin
--env <ENV>                 Environment: production (default), uat
--profile <PROFILE>         Config profile name
--dry-run                   Preview changes without executing
--yes                       Skip safety confirmations
--verbose                   Verbose logging
```

## Authentication

### Credential Resolution Chain

1. `CDCX_API_KEY` and `CDCX_API_SECRET` environment variables (preferred)
   - Fallback: `CDC_API_KEY` and `CDC_API_SECRET`
2. Config file: `~/.config/cdcx/config.toml`

### Config File Format

```toml
[default]
api_key = "..."
api_secret = "..."
environment = "production"

[profiles.uat]
api_key = "..."
api_secret = "..."
environment = "uat"
```

### Verification

```bash
cdcx account info          # Requires auth; tests credentials
```

## Output Format

Default output is **JSON** (one object per line for streams, single object for commands):

```json
{"status":"ok","result":{"order_id":"123"}}
```

Errors use the standardized error envelope:

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

## Schema Introspection

Query endpoint schemas dynamically:

```bash
cdcx schema list                    # All endpoints
cdcx schema list --group market     # Endpoints in a group
cdcx schema show public/get-tickers # Full schema for endpoint
cdcx schema catalog                 # Full catalog as JSON array
```

## Safety Tiers

Each command is marked with a safety tier controlling when confirmation is required:

| Tier | CLI Behavior | MCP Behavior | Examples |
|------|------------|-------------|----------|
| `read` | No prompt | Always allowed | Market data, account info |
| `sensitive_read` | No prompt | Always allowed | Balances, positions |
| `mutate` | Prompt in table mode | Requires `acknowledged=true` | Orders, transfers |
| `dangerous` | Always prompts | Requires `--allow-dangerous` + `acknowledged=true` | Cancel-all, withdrawals |

### Safety Acknowledgment

When using MCP server or `--yes` flag, provide `acknowledged=true` in parameters for mutate-tier commands.

## MCP Server

Start the MCP server with specified service groups:

```bash
cdcx mcp --services market,trade,account
cdcx mcp --services market,trade --allow-dangerous
```

Default services: `market` (public endpoints only)

See `agents/AGENTS.md` for full MCP integration guide.

## Command Groups

- `market` — Ticker, book, trades, candlesticks, instruments, valuations, announcements, risk params, settlement, insurance
- `account` — Summary, info, subaccount balances, positions, leverage, settings, fee rates, networks
- `trade` — Order, amend, cancel, close position, open orders, order detail
- `advanced` — OCO, OTO, OTOCO orders
- `wallet` — Withdraw, networks, deposit/withdrawal addresses and history
- `fiat` — Deposit/withdraw fiat, accounts, currencies, channels
- `staking` — Stake/unstake, positions, instruments, rewards
- `margin` — Transfer, leverage
- `history` — Orders, trades, transactions
- `stream` — Ticker, book, trades, candlesticks, settlements, funding, mark, index, balance, orders, positions, user trades, account risk

## Further Reading

- **Agent Integration Guide:** See `agents/AGENTS.md` for authentication, error handling, rate limiting, safety flows, and example workflows
- **Skills:** See `skills/` for step-by-step example workflows
- **Tool Catalog:** See `agents/tool-catalog.json` for full endpoint metadata
- **Error Categories:** See `agents/error-catalog.json` for error handling guidance
