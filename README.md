# ifx-raydium-ext

Rust web stack: **Ifx** orchestration over Raydium **CPMM** direct pool swaps (no Raydium Router).

## Quick start

```bash
cp config.toml.example config.toml
# Edit RPC, ifx frames, ALT, service fee, optional sponsor keypair.

cargo run -p ifx-raydium-server
# open http://127.0.0.1:8788
```

Connect a wallet (Phantom / Solflare), enter mints and amount, preview quote + assembled tx in the right panel, then **Simulate** or **Sign & Send**.

## Features

| Area | Status |
|------|--------|
| **Direct** CPMM swap (any pair with a pool) | ✅ |
| **Bridge** A → WSOL → B (when neither side is SOL) | ✅ |
| Dynamic platform fee (bps × **on-chain** proceeds) | ✅ |
| **Sponsored gas** (Bridge + Direct SOL output) | ✅ |
| Smart close empty input ATA | ✅ |
| Bridge hop WSOL ATA close | ✅ |
| v0 tx + ALT, 1232 B size gate (default) | ✅ |
| Solana **v1** tx (4096 B, message config; `transaction_version = 1`) | ✅ |
| Raydium API v3 pool hint + RPC hydrate / fallback | ✅ |
| Optional HTTP proxy for Raydium API | ✅ |
| Web UI (wallet, quote, inspector, simulate) | ✅ |

## Example transactions (mainnet)

Transactions assembled by this stack on Solana mainnet:

| Flow | Solscan |
|------|---------|
| **Two-hop bridge** — A → WSOL → B; smart close empty input ATA | [2wNX5c8zSrW3xgmYKdzpsoD2Bd7dgfBnprWWKQXycn4YjbZriEJKrs3sTbwEaM3ycnwjTZ8dbCRTxk36sC79gAjo](https://solscan.io/tx/2wNX5c8zSrW3xgmYKdzpsoD2Bd7dgfBnprWWKQXycn4YjbZriEJKrs3sTbwEaM3ycnwjTZ8dbCRTxk36sC79gAjo) |
| **Sponsored bridge** — sponsor co-signs fee payer + rent; on-chain repay from WSOL proceeds; smart close input ATA | [2mEavKoZT39PtSXYwwze7WzgvuRkeXddNXjcequEdAH3DVEE9zovXDvE9QAzHFcT8iSJBzjaWkhMyiD9bEaDAdyz](https://solscan.io/tx/2mEavKoZT39PtSXYwwze7WzgvuRkeXddNXjcequEdAH3DVEE9zovXDvE9QAzHFcT8iSJBzjaWkhMyiD9bEaDAdyz) |

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

- `[solana]` — RPC, commitment, **transaction_version** (`0` default / `1` for v1), **address_lookup_tables** (v0 only; required for Bridge / sponsor under 1232 B)
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
