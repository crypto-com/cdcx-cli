# cdcx CLI

Agent-first CLI for the Crypto.com Exchange API with a built-in MCP server exposing tools.

## Quick Start

The plugin starts with **market data only** — no API keys needed:

```
cdcx market ticker BTC_USDT
```

## Enable Trading

To unlock trading and account tools, update the MCP server configuration:

1. Edit the `mcpServers.cdcx.args` in your MCP settings (project `.claude/settings.json` or user `~/.claude/settings.json`)
2. Change the args to include additional services:

```json
["mcp", "--services", "market,trade,account"]
```

3. Add your credentials in the `env` section:

```json
{
  "CDC_API_KEY": "your-key",
  "CDC_API_SECRET": "your-secret"
}
```

Or run `cdcx setup` to configure credentials interactively.

## Available Services

| Service | Auth | Description |
|---------|:----:|-------------|
| `market` | — | Tickers, orderbook, candles, trades |
| `account` | Yes | Balances, positions, account info |
| `trade` | Yes | Place, amend, cancel orders |
| `advanced` | Yes | OCO, OTO, OTOCO compound orders |
| `margin` | Yes | Margin transfers, leverage |
| `staking` | Yes | Stake/unstake operations |
| `funding` | Yes | Withdrawals (requires `--allow-dangerous`) |
| `fiat` | Yes | Fiat operations (requires `--allow-dangerous`) |

## Example Configurations

**Read-only (default):**
```json
["mcp", "--services", "market"]
```

**Trading agent:**
```json
["mcp", "--services", "market,trade,account"]
```

**Full access:**
```json
["mcp", "--services", "market,trade,account,advanced,margin,staking,funding,fiat", "--allow-dangerous"]
```

## Links

- [GitHub](https://github.com/crypto-com/cdcx-cli)
- [Agent Integration Guide](https://github.com/crypto-com/cdcx-cli/blob/main/agents/AGENTS.md)
- [Crypto.com Exchange](https://crypto.com/exchange)
