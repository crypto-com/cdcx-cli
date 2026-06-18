# cdcx CLI

Agent-first CLI for the Crypto.com Exchange API with a built-in MCP server exposing tools.

## Quick Start

The plugin starts with **market data only** — no API keys needed:

```
cdcx market ticker BTC_USDT
```

## Enable Trading

To unlock trading and account tools, use `cdcx mcp config`:

```bash
cdcx mcp config --enable trade
cdcx mcp config --enable account
```

Then set up your API credentials:

```bash
cdcx setup
```

Or set `CDC_API_KEY` and `CDC_API_SECRET` environment variables.

Your configuration is saved to `~/.config/cdcx/mcp.toml` and persists across plugin updates.

## Managing Services

```bash
cdcx mcp config                      # Show current configuration
cdcx mcp config --enable trade       # Enable a service group
cdcx mcp config --disable funding    # Disable a service group
cdcx mcp config --allow-dangerous    # Enable dangerous operations
cdcx mcp config --no-dangerous       # Disable dangerous operations
cdcx mcp config --reset              # Reset to defaults
```

## Available Services

| Service | Auth | Description |
|---------|:----:|-------------|
| `market` | — | Tickers, orderbook, candles, trades |
| `account` | Yes | Balances, positions, account info |
| `trade` | Yes | Place, amend, cancel orders |
| `advanced` | Yes | OCO, OTO, OTOCO compound orders |
| `margin` | Yes | Margin transfers, leverage |
| `staking` | Yes | Stake/unstake operations |
| `bot` | Yes | Trading bot management (DCA, TWAP, GRID, FUNDING_ARBITRAGE) |
| `funding` | Yes | Withdrawals (requires `--allow-dangerous`) |
| `fiat` | Yes | Fiat operations (requires `--allow-dangerous`) |

## Links

- [GitHub](https://github.com/crypto-com/cdcx-cli)
- [Agent Integration Guide](https://github.com/crypto-com/cdcx-cli/blob/main/agents/AGENTS.md)
- [Crypto.com Exchange](https://crypto.com/exchange)
