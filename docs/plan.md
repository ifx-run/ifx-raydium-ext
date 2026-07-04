# ifx-raydium-ext — Technical Overview (English)

Canonical detailed spec: **[plan.zh-CN.md](./plan.zh-CN.md)** (kept in sync with the codebase).

**Related:** [ifx-pumpfun-ext](https://github.com/ifx-run/ifx-pumpfun-ext) — Pump.fun + Ifx reference implementation.

---

## What this repo is

A **Rust + Axum web app** that builds **versioned (v0) Solana transactions** for Raydium **CPMM** swaps, orchestrated by **Ifx** (`ifx_let`, `ifx_patched_cpi`, `ifx_assert`, `ifx_if_else`).

- **No Trade API `/transaction/*`** — quotes and ix are local; vaults refreshed via RPC at build time.

---

## Routing

Two candidates run in parallel:

1. **Direct** — single CPMM pool for `(mintA, mintB)`.
2. **Bridge** — `mintA → WSOL → mintB` when **neither** mint is SOL.

Decision: quote both when available; pick higher `expected_out`; tie → prefer Direct (smaller tx).

Pool selection: Raydium API v3 hint, sorted by on-chain reserves when using RPC fallback; Bridge compares up to 8×8 pool combinations.

---

## Ifx build (summary)

**Direct (SOL output):** WSOL proceeds measured on SPL balance → dynamic fee via `UnwrapLamports` → optional unwrap → optional sponsor repay.

**Bridge:**

```text
ixReset
→ [sponsor] ATA bootstrap (sponsor payer, on-chain ataCost)
→ leg1 CPMM (A → WSOL)
→ let WSOL delta → dynamic platform fee (`UnwrapLamports` → native SOL)
→ [sponsor] repay bind + assert
→ leg2 CPMM patch amount_in = net_wsol − repay, min_out scaled on-chain
→ close hop WSOL ATA
→ [sponsor] SOL transfer repay to sponsor
→ [optional] smart close input ATA if balance = 0
```

**Sponsor eligibility:** Bridge routes; Direct when output is SOL/WSOL. Requires `[sponsor].keypair_path` on server for co-sign.

**Repay formula (on-chain):**

```text
repay = (ataCost + txFee) × (100 + repay_buffer_percent) / 100
txFee = 2 × 5000 + ceil(compute_unit_limit × micro_lamports / 1e6)
```

`ataCost` = sum of lamports deltas on bootstrap ATAs (Ifx before/after create). No hard-coded rent constants.

---

## Transaction constraints

- Target **≤ 1232 bytes** (v0 + configured ALTs).
- Smart-close ixs appended only if compile + size gate pass.
- Build returns `transactionSizeBytes`, `fitsSizeGate`, instruction inspection JSON.

---

## Out of scope (current)

- USDC/USDT bridge mint, Jupiter
- Using Trade API `/compute` to drive build

---

## Changelog

| Date | Note |
|------|------|
| 2026-06 | Web UI + Axum server; CPMM Direct ∥ SOL bridge |
| 2026-06 | Sponsored bridge: on-chain ataCost, WSOL repay, leg2 min_out patch |
| 2026-06 | Smart close input ATA; bridge WSOL hop close; pool RPC sort; HTTP proxy |
