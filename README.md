# ifx-raydium-ext

Rust web stack: **Ifx** orchestration over Raydium **CPMM** direct pool swaps (no Raydium Router).

## Quick start

```bash
cp config.toml.example config.toml
# Edit RPC, ifx frames, ALT, service fee, optional sponsor keypair.

cargo run -p ifx-raydium-server
# open http://127.0.0.1:8788
```

Connect a wallet (Phantom / Solflare), enter mints and amount, preview quote + assembled v0 tx in the right panel, then **Simulate** or **Sign & Send**.

## Features

| Area | Status |
|------|--------|
| **Direct** CPMM swap (any pair with a pool) | ✅ |
| **Bridge** A → WSOL → B (when neither side is SOL) | ✅ |
| Dynamic platform fee (bps × **on-chain** proceeds) | ✅ |
| **Sponsored gas** (Bridge + Direct SOL output) | ✅ |
| Smart close empty input ATA | ✅ |
| Bridge hop WSOL ATA close | ✅ |
| v0 tx + ALT, 1232 B size gate | ✅ |
| Raydium API v3 pool hint + RPC hydrate / fallback | ✅ |
| Optional HTTP proxy for Raydium API | ✅ |
| Web UI (wallet, quote, inspector, simulate) | ✅ |

## Workspace

| Path | Role |
|------|------|
| `crates/raydium` | API v3 client, RPC pool hydrate, CPMM quote, `swap_base_input` ix |
| `crates/planner` | Route (Direct ∥ SOL bridge), Ifx build/finalize, sponsor, smart close |
| `crates/config` | TOML config loader |
| `apps/server` | Axum API + static web UI |
| `apps/web/public` | Swap UI |

## API (server)

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/api/health` | Liveness |
| GET | `/api/config/public` | Debounce, defaults, sponsor flags |
| POST | `/api/quote` | Route + quote; optional `build: true` with wallet |
| POST | `/api/tx/build` | Build + finalize v0 tx |
| POST | `/api/tx/simulate` | RPC simulate |
| POST | `/api/token/info` | Mint metadata for UI |

## Config highlights

See [config.toml.example](config.toml.example). Key sections:

- `[solana]` — RPC, commitment, **address_lookup_tables** (required for Bridge / sponsor)
- `[ifx]` — program id, **public_frames**
- `[service_fee]` — bps + recipient
- `[sponsor]` — `enabled`, `pubkey`, `keypair_path` (server co-sign), `repay_buffer_percent`
- `[network]` — optional `http_proxy`, `raydium_api_timeout_secs`
- `[server]` — bind host/port

## Tools

```bash
# Print pool accounts / PDAs for ALT extend planning
SOLANA_RPC_URL=https://… cargo run -p ifx-raydium --example alt_dump
```

## Docs

- **Canonical spec (中文):** [docs/plan.zh-CN.md](docs/plan.zh-CN.md)
- English summary: [docs/plan.md](docs/plan.md)

## Related

- [ifx-pumpfun-ext](https://github.com/ifx-run/ifx-pumpfun-ext) — Pump.fun + Ifx (fee / sponsor patterns ported here)
- [Ifx Rust SDK](https://github.com/ifx-run/ifx/tree/main/rust-sdk)
