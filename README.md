# cdcx

A CLI, MCP server, and terminal dashboard for the [Crypto.com Exchange API](https://exchange-docs.crypto.com/exchange/v1/rest-ws/index.html). Single binary, zero runtime dependencies.

86 REST endpoints across 10 API groups, dynamically generated from the Crypto.com Exchange OpenAPI spec. Real-time WebSocket streaming. Full-screen TUI dashboard. Paper trading. Works as a standalone CLI, an MCP tool server for AI agents, or an interactive terminal.

> **Caution:** This software interacts with the live Crypto.com Exchange and can execute real financial transactions. Test with `cdcx paper` before using real funds.

## Install

```bash
curl -sSfL https://raw.githubusercontent.com/crypto-com/cdcx-cli/main/install.sh | sh
```

**From source:**

```bash
cargo install --git https://github.com/crypto-com/cdcx-cli.git --bin cdcx
```

---

## Why cdcx?

### For AI Agents

Every response is structured JSON. 86 MCP tools with typed parameters, enum validation, safety enforcement, and schema discovery — all generated from the OpenAPI spec at runtime. Your LLM can trade, analyze markets, and manage positions without custom tooling.

```bash
cdcx mcp --services market,account,trade
```

```json
{
  "mcpServers": {
    "cdcx": {
      "command": "cdcx",
      "args": ["mcp", "--services", "market,account,trade"]
    }
  }
}
```

Compatible with Claude Code, Cursor, Claude Desktop, Codex, Github Copilot, Gemini CLI, and other MCP clients. Includes 13 agent skill files in `skills/` for guided workflows.

Things you can ask your AI agent:

> *"What's the current BTC price and 24h volume?"*

> *"Paper trade BTC for a few rounds and show me the P&L"*

> *"Place an OTOCO bracket on BTC_USDT: entry at 70000, stop-loss at 65000, take-profit at 75000"*

### For the Command Line

Every endpoint is a command. `--help` on everything. `--dry-run` to preview. `--output json` for scripting. Tab completion. Profiles for multi-account.

```bash
cdcx market ticker BTC_USDT -o table
cdcx trade order BUY BTC_USDT 0.001 --dry-run
cdcx stream ticker BTC_USDT ETH_USDT
cdcx paper buy BTC_USDT --quantity 0.01
```

### For the Dashboard

Full-screen terminal trading interface. 6 tabs, real-time streaming, candlestick charts, heatmap mode, split screen, order workflows. Zero flicker.

```bash
cdcx tui
```

Press `p` to toggle paper mode. Press `O` for OTOCO bracket orders. Press `?` for all shortcuts.

---

## Quick Start

Public market data works without credentials:

```bash
cdcx market ticker BTC_USDT -o table
cdcx market book BTC_USDT --depth 10 -o table
cdcx market candlestick BTC_USDT --timeframe 1h -o table
```

Paper trading works without credentials:

```bash
cdcx paper init --balance 50000
cdcx paper buy BTC_USDT --quantity 0.01
cdcx paper positions
cdcx paper balance
```

For trading, set credentials:

```bash
export CDCX_API_KEY="your-key"
export CDCX_API_SECRET="your-secret"
cdcx account summary
```

---

## MCP Server

Expose the Exchange API as MCP tools for AI agents:

```bash
cdcx mcp --services market,account,trade
cdcx mcp --services all --allow-dangerous
```

Service groups (MCP): `market`, `account`, `trade`, `advanced`, `margin`, `staking`, `funding`, `fiat`, `otc`, `stream`, `all`

> **Note:** `account` also exposes historical endpoints (orders, trades, transactions). `funding` covers wallet deposit/withdrawal endpoints. Paper trading is a CLI-only feature and has no MCP tools.

### Safety Model

| Tier | Behavior | Examples |
|------|----------|---------|
| **read** | No confirmation | `market ticker`, `market book` |
| **sensitive_read** | No confirmation | `account summary`, `trade open-orders` |
| **mutate** | Requires `acknowledged: true` | `trade order`, `trade cancel` |
| **dangerous** | Requires `--allow-dangerous` | `trade cancel-all`, `wallet withdraw` |

### Agent Skills

13 skill files in `skills/` covering:

| Skill | Purpose |
|-------|---------|
| `cdcx-market-intel` | Market analysis and price discovery |
| `cdcx-portfolio-intel` | Portfolio analysis and risk assessment |
| `cdcx-execution` | Order placement with safety checks |
| `cdcx-advanced` | OCO, OTO, OTOCO contingency orders |
| `cdcx-paper-strategy` | Paper trading strategy testing |
| `cdcx-wallet-ops` | Deposits, withdrawals, network management |
| `cdcx-auth-setup` | Credential configuration |
| `cdcx-autonomy-levels` | Safety tier configuration |
| `cdcx-check-balance` | Balance and credential verification |
| `cdcx-place-limit-order` | Limit order workflow with preflight checks |
| `cdcx-isolated-margin` | Isolated margin trading (equity/RWA perpetuals) |
| `recipe-morning-brief` | Daily market briefing workflow |
| `recipe-emergency-flatten` | Emergency position flattening |

### Plugin Installation

Install cdcx as a one-click plugin from your AI coding tool's marketplace.

**Claude Code:**

```bash
claude plugin marketplace add crypto-com/cdcx-cli
claude plugin install cdcx-cli@cdcx-cli
```

**Codex CLI:**

```bash
codex plugin marketplace add crypto-com/cdcx-cli
```

**Gemini CLI:**

```bash
gemini extensions install https://github.com/crypto-com/cdcx-cli
```

**Other:** Open Settings > MCP Servers, add:

```json
{
  "cdcx": {
    "command": "cdcx",
    "args": ["mcp", "--services", "market"]
  }
}
```

**Any MCP client:** Drop `.mcp.json` in your project root (included in this repo).

To expand services beyond market data, update the `--services` flag:

```
market,trade,account          # Trading agent
market,trade,account,advanced # With OCO/OTOCO
all --allow-dangerous         # Full access (withdrawals enabled)
```

---

## CLI Reference

### Market Data (public)

```bash
cdcx market ticker                     # All tickers
cdcx market ticker BTC_USDT            # Single instrument
cdcx market book BTC_USDT --depth 20   # Order book
cdcx market trades BTC_USDT            # Recent trades
cdcx market candlestick BTC_USDT --timeframe 1h
cdcx market instruments                # All instruments
```

### Trading (requires auth)

```bash
cdcx trade order BUY BTC_USDT 0.001 --type LIMIT --price 50000
cdcx trade open-orders
cdcx trade cancel --order-id ORDER_ID
cdcx trade cancel-all
```

### Advanced Orders

```bash
cdcx advanced create-oto --instrument-name BTC_USDT ...
cdcx advanced create-otoco --instrument-name BTC_USDT ...
cdcx advanced open-orders
```

### Paper Trading

Local paper trading engine with live market prices. No auth required.

```bash
cdcx paper init --balance 50000        # Create account
cdcx paper buy BTC_USDT --quantity 0.01   # Market buy
cdcx paper sell BTC_USDT --quantity 0.01  # Market sell
cdcx paper buy BTC_USDT --quantity 0.01 --price 65000  # Limit buy
cdcx paper positions                   # Portfolio + P&L
cdcx paper history                     # Trade history
cdcx paper balance                     # Account balance
cdcx paper reset --balance 100000      # Reset account
```

### Streaming (WebSocket)

```bash
cdcx stream ticker BTC_USDT ETH_USDT  # Real-time tickers
cdcx stream book BTC_USDT             # Order book updates
cdcx stream trades BTC_USDT           # Trade executions
cdcx stream orders                    # Your order updates (auth)
cdcx stream positions                 # Position changes (auth)
```

### Account

```bash
cdcx account summary                   # Balances
cdcx account positions                 # Open positions
cdcx trade fee-rate                    # Fee rates
```

### History / Wallet / Staking / Fiat / Margin

```bash
cdcx history orders                    cdcx wallet deposit-address --currency BTC
cdcx history trades                    cdcx wallet deposit-history
cdcx history transactions              cdcx wallet withdrawal-history
cdcx staking instruments               cdcx fiat accounts
cdcx staking positions                 cdcx margin transfer --dry-run
```

---

## Interactive Dashboard

```bash
cdcx tui                               # Launch
cdcx tui --theme amber                 # With theme
cdcx tui --setup                       # Setup wizard
```

### Features

- **6 tabs:** Market, Portfolio, Orders, History, Watchlist, Positions
- **Real-time streaming:** WebSocket ticker, order book, candlestick, trade channels
- **Sparklines:** 24h Braille-dot price charts inline per instrument
- **Heatmap mode:** rows glow red/green by 24h performance
- **Candlestick charts:** volume bars, 9 timeframes, streaming updates, time axis
- **Multi-chart compare:** up to 4 instruments side by side
- **Split screen:** table + chart, auto-updates on selection
- **Order book detail:** cumulative depth bars, buy/sell pressure bar
- **Order workflows:** place order, OCO, OTOCO, cancel
- **Live portfolio P&L:** session P&L in status bar, cash vs position breakdown in Portfolio tab
- **Paper mode:** toggle `p` to trade against local paper engine with unrealized/realized P&L
- **Instrument picker:** search-as-you-type overlay
- **Price alerts:** set thresholds with terminal bell notification
- **Ticker tape:** scrolling top movers banner
- **6 themes:** terminal-pro, cyber-midnight, monochrome, neon, micky-d, amber + custom TOML
- **Mouse support:** click to select, scroll, double-click to detail
- **Export:** `y` copies table as CSV to clipboard

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `1`-`6` / `Tab` | Switch tabs |
| `Enter` | Instrument detail (book + trades) |
| `k` | Candlestick chart |
| `m` | Compare charts (up to 4) |
| `h` | Toggle heatmap |
| `i` | Instrument spotlight |
| `\` | Split screen (table + chart) |
| `[ / ]` | Cycle chart timeframe |
| `s / S` | Sort / reverse sort |
| `/` | Search instruments |
| `t` | Place order |
| `o` | OCO order (stop-loss + take-profit) |
| `O` | OTOCO order (entry + SL + TP) |
| `c` | Cancel orders |
| `p` | Toggle LIVE / PAPER mode |
| `!` | Set price alert |
| `y` | Copy to clipboard (CSV) |
| `?` | Help overlay |
| `q` | Quit |

---

## Configuration

### Credentials

Resolved in order: flags > `CDCX_API_KEY`/`CDCX_API_SECRET` env > `CDC_API_KEY`/`CDC_API_SECRET` env > `~/.config/cdcx/config.toml` profile.

```bash
cdcx setup                             # Interactive credential setup
cdcx --profile uat account summary     # Use named profile
```

### TUI Config

`~/.config/cdcx/tui.toml`:

```toml
theme = "terminal-pro"
tick_rate_ms = 250
watchlist = ["BTC_USDT", "ETH_USDT", "SOL_USDT", "CRO_USDT"]

[themes.my-theme]
bg = "#1a1a2e"
accent = "#00d4ff"
positive = "#00ff88"
negative = "#ff4444"
```

### Global Flags

| Flag | Description |
|------|-------------|
| `-o, --output` | Output format: `json` (default), `table`, `ndjson` |
| `--dry-run` | Preview request without executing |
| `--env` | Environment: `production` (default), `uat` |
| `--profile` | Config profile name |
| `--yes` | Skip confirmation prompts |
| `-v, --verbose` | Verbose output |

---

## Architecture

```
crates/cdcx-core/       # API client, auth, signing, schema, OpenAPI parser, paper engine
crates/cdcx-cli/        # CLI binary, dispatch, MCP server, setup
crates/cdcx-tui/        # Terminal dashboard (ratatui + crossterm)
schemas/                # CLI overlay files (command aliases, positional args, defaults)
skills/                 # Agent skill files for guided workflows
site/                   # Marketing site (single HTML file)
```

- **OpenAPI sole source:** All API commands and MCP tools are generated from the exchange's OpenAPI spec at runtime (24h cache). Thin TOML overlay files in `schemas/` add CLI-only metadata (positional args, defaults, command aliases). No hand-maintained endpoint definitions.
- **Single binary:** ~11MB, no runtime dependencies
- **Zero flicker:** ratatui double-buffer character-level diffing

## Development

```bash
cargo test                             # Run all tests
cargo build --release                  # Build release binary
cargo run -- market ticker BTC_USDT    # Run from source
cargo run -- tui                       # TUI from source
```

### Git hooks

Install once per clone to catch CI failures locally:

```bash
./hooks/install.sh
```

- `pre-commit` runs `cargo fmt --check` + `cargo clippy -- -D warnings`
- `pre-push` runs the full test suite (skipped for doc-only pushes)

The toolchain is pinned to `stable` via `rust-toolchain.toml`; run `rustup update stable` if your local clippy is older than CI's.

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).
