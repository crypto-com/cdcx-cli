# Security

`cdcx` interacts with the live Crypto.com Exchange and can execute real financial transactions on your behalf. Treat your API key and secret with the same care as a password. This document outlines the baseline recommendations for using `cdcx` safely.

## Reporting a vulnerability

Please **do not** open public GitHub issues for security vulnerabilities.

Report suspected vulnerabilities privately via one of:

- GitHub's private vulnerability reporting: <https://github.com/crypto-com/cdcx-cli/security/advisories/new>

Include a clear description, reproduction steps, and the affected version (`cdcx --version`). We will acknowledge receipt within 3 business days and coordinate a fix and disclosure timeline with you.

## Credential handling

`cdcx` reads credentials from exactly two sources, in order of precedence:

1. Environment variables: `CDCX_API_KEY` and `CDCX_API_SECRET` (or legacy `CDC_API_KEY` / `CDC_API_SECRET`).
2. The config file at `~/.config/cdcx/config.toml`, organised by profile.

There is **no `--api-key` or `--api-secret` CLI flag by design.** Command-line flags are recorded by shell history, visible via `ps`, and readable by other processes — they are not a safe transport for secrets.

### Recommended practices

- **Prefer the config file.** `cdcx setup` writes credentials to `~/.config/cdcx/config.toml` interactively so the secret never appears in shell history or process arguments.
- **Lock down the config file.** Ensure permissions are `0600` (owner read/write only):

  ```bash
  chmod 600 ~/.config/cdcx/config.toml
  ```

- **If you must use environment variables, avoid inline exports.** A command like `CDCX_API_SECRET=... cdcx ...` is captured by your shell's history. Instead, export them from a file sourced outside history, or use a secrets manager (1Password CLI, `pass`, `aws secretsmanager`, `direnv` with a `.envrc` that is not committed).
- **Use read-only API keys when possible.** The Crypto.com Exchange lets you scope API keys — create a read-only key for market data and analytics workflows. Only create a key with trading/withdrawal permissions when you need it.
- **Use dedicated withdrawal allowlists.** If your key has withdrawal permissions, configure an IP allowlist and address whitelist on the exchange side. `cdcx` cannot enforce these for you.
- **Never commit credentials.** Do not add `config.toml`, `.env`, or any file containing keys to version control. Do not paste them into logs, issues, or chat.
- **Rotate keys regularly**, and immediately if you suspect compromise. Revoke old keys in the Crypto.com Exchange portal — deleting `config.toml` does not revoke server-side access.

## Safety model

`cdcx` classifies every API operation into one of four tiers. The MCP server enforces these tiers when exposing tools to AI agents:

| Tier | Examples | Protection |
|------|----------|------------|
| `read` | `market ticker`, `market book` | None required |
| `sensitive_read` | `account summary`, `trade open-orders` | None required (authenticated) |
| `mutate` | `trade order`, `trade cancel` | MCP requires `acknowledged: true` parameter |
| `dangerous` | `trade cancel-all`, `wallet withdraw` | Requires `cdcx mcp --allow-dangerous` flag at server startup |

When running the MCP server for an AI agent:

- Start with `--services market,account` to expose read-only capabilities first.
- Add `trade` only when the agent genuinely needs to place orders.
- Never pass `--allow-dangerous` unless you explicitly want the agent to be able to withdraw funds or bulk-cancel orders.

## Paper trading

Use `cdcx paper` to rehearse strategies and test agent workflows without touching real funds. Paper trading is fully local, requires no credentials, and produces no network requests to the exchange.

## Binary integrity

Release binaries are built in GitHub Actions from the `main` branch and published to <https://github.com/crypto-com/cdcx-cli/releases>. Each release includes SHA-256 checksums. The installer script verifies downloads against these checksums.

To verify manually:

```bash
curl -L -o cdcx.tar.gz https://github.com/crypto-com/cdcx-cli/releases/download/v1.0.2/cdcx-1.0.2-aarch64-apple-darwin.tar.gz
curl -L -o cdcx.tar.gz.sha256 https://github.com/crypto-com/cdcx-cli/releases/download/v1.0.2/cdcx-1.0.2-aarch64-apple-darwin.tar.gz.sha256
shasum -a 256 -c cdcx.tar.gz.sha256
```

If you build from source, you can pin to a specific tag:

```bash
cargo install --git https://github.com/crypto-com/cdcx-cli.git --tag v1.0.2 --bin cdcx
```

## Network

`cdcx` talks to the following hosts over TLS:

- `api.crypto.com` — REST API (production)
- `stream.crypto.com` — WebSocket streams (production)
- `uat-api.3ona.co` / `uat-stream.3ona.co` — UAT/testnet (when `--env uat`)

It also fetches the OpenAPI specification from the Crypto.com Exchange documentation endpoint on first run and once every 24 hours for schema refresh. No telemetry, analytics, or other outbound connections are made.

## Supported versions

Only the latest released version of `cdcx` receives security fixes. Upgrade before reporting a vulnerability if you are running an older version.

| Version | Supported |
|---------|-----------|
| `1.0.x` | Yes |
| `< 1.0` | No (pre-release) |
