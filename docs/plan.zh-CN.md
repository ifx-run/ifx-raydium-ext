# ifx-raydium-ext — 实现规格（与代码同步）

本文档描述 **当前仓库已实现** 的行为与配置，供开发、Review 与 AI 会话使用。

**关联项目：** [ifx-pumpfun-ext](https://github.com/ifx-run/ifx-pumpfun-ext)（Pump.fun + Ifx；platform fee / sponsor 模式来源）

**English summary:** [plan.md](./plan.md)

---

## 1. 项目定位

| 项 | 说明 |
|----|------|
| **Venue** | Raydium **CPMM** 直连池 swap（单池单 ix） |
| **编排** | **Ifx**（`ifx_let` / `ifx_patched_cpi` / `ifx_assert` / `ifx_if_else`） |
| **交付** | **Web UI** + Axum API（`apps/server` + `apps/web/public`） |
| **不做** | Trade API 成品 tx、Pump.fun、Jupiter |

---

## 2. 仓库结构

```text
ifx-raydium-ext/
├── Cargo.toml                 # workspace
├── config.toml.example
├── crates/
│   ├── raydium/               # API v3、RPC hydrate、CPMM quote、swap ix
│   ├── planner/               # 路由、Ifx build/finalize、sponsor、smart close
│   └── config/                # TOML 配置
├── apps/
│   ├── server/                # Axum API + 静态前端
│   └── web/public/            # HTML / JS / CSS
└── docs/
    ├── plan.zh-CN.md          # 本文档
    └── plan.md
```

**依赖：** `ifx-sdk`（Rust，path 或 crates.io 0.1.1+）、`solana-sdk` 3.x、`tokio`、`axum`、`reqwest`（含 `socks` 可选代理）。

---

## 3. 用户界面

### 3.1 左栏（交易表单）

- Pay / Receive mint、数量、滑点、Priority 档位
- 连接钱包（浏览器扩展）
- Refresh quote

### 3.2 右栏（Trade preview）

- 报价结果、路由说明、Simulate / Sign & Send
- **Sponsored gas** 开关（首次询价后可用；Bridge 或 Direct SOL 输出时可选）
- Transaction inspector（指令列表、fee payer、tx size、smart close 状态）

SOL 作为 Pay/Receive 时可选 **native SOL** 或 **WSOL ATA**（`payAsset` / `receiveAsset`）。

---

## 4. HTTP API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/health` | 健康检查 |
| GET | `/api/config/public` | 公开配置（debounce、默认 mint、 sponsor 是否可用等） |
| POST | `/api/quote` | 路由 + 询价；`build: true` 且带 `userPubkey` 时一并组装 tx |
| POST | `/api/tx/build` | 仅 build + finalize |
| POST | `/api/tx/simulate` | RPC 模拟 |
| POST | `/api/token/info` | Mint 元数据（UI 展示） |

静态资源：`/` → `apps/web/public/`。

---

## 5. 路由

### 5.1 两种候选（并行）

**Task A — Direct**

```text
select_cpmm_pool(mint_a, mint_b) → 单池 CPMM swap
```

**Task B — SOL Bridge**（仅当 **A、B 均不是 WSOL/SOL**）

```text
join!( select(A, WSOL), select(WSOL, B) )
```

若 `mint_a == SOL || mint_b == SOL`：**跳过 Task B**。

### 5.2 决策

```text
match (direct, bridge) {
  (Some(d), None)     → Direct
  (None, Some(b))     → Bridge
  (Some(d), Some(b))  → 两路 quote，比 expected_out；接近时优先 Direct（tx 更小）
  (None, None)        → NoRoute
}
```

### 5.3 Bridge 选池

- API v3 返回候选池；RPC 回退时按链上 reserve **排序**后再截断。
- Bridge：**8×8 池组合**交叉询价，取最终输出最大者（非两段独立贪心）。

### 5.4 流动性提示

若同 pair 存在 **Standard AMM v4** 且 TVL 远高于所选 CPMM，quote 返回 `liquidityNote`（仅供参考，仍只用 CPMM）。

---

## 6. Raydium 集成

### 6.1 使用

| 用 | 不用 |
|----|------|
| API v3 `/pools/info/mint` 选池 hint | Trade API `/transaction/swap-*` |
| RPC `getMultipleAccounts` hydrate 池子 | Trade API `/compute` 驱动 build |
| 本地 CPMM constant-product quote | Trade API `/compute` 驱动 build |
| 自拼 `swap_base_input` + Ifx patch | |

### 6.2 网络

`[network]` 段：

- `http_proxy` — Raydium API 代理（亦读 `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`）
- `raydium_api_timeout_secs` — API 超时后走 RPC 候选

---

## 7. Ifx Build 拓扑

### 7.1 公共前缀

```text
ComputeBudget (limit + price)
→ ifx_reset
```

### 7.2 Direct

**输出为 WSOL（Raydium SOL 侧）：**

```text
[ATA creates — 用户或 sponsor]
→ swap (→ WSOL ATA)
→ let WSOL SPL balance delta
→ [dynamic] platform fee (patched SPL transfer)
→ [optional] close WSOL → native SOL
→ [optional] sponsor repay (见 §8)
→ [optional] smart close 输入 token ATA
```

**输出为 SPL：** 单 swap + 用户付 ATA create；无 Ifx fee（除非 sponsor 等需 frame）。

**输入为 WSOL：** 可选 wrap native SOL；输入侧 platform fee 在 swap 前静态扣除（`split_gross_input`）。

### 7.3 Bridge

```text
[sponsor] append_sponsor_ata_bootstrap   # sponsor 付 rent，链上计量 ataCost
→ let WSOL baseline (leg1 前)
→ leg1 CPMM: A → WSOL
→ let WSOL delta → dynamic platform fee
→ [sponsor] bind repay = (ataCost + txFee) × buffer
→ [sponsor] assert proceeds ≥ fee + repay
→ leg2 CPMM (patched):
      amount_in = net_wsol − repay
      min_out   = quote_min_out × amount_in / net_wsol   # 链上 divFloor
→ close 中间 WSOL ATA（回收 rent / unwrap repay 部分）
→ [sponsor] patched SOL transfer → sponsor
→ [optional] smart close 输入 token ATA
```

Leg2 **必须** patch `amount_in`（offset +8）与 sponsored 时 patch `minimum_amount_out`（offset +16）。

### 7.4 Smart close

- **输入 token ATA：** 交易末尾 `ifx_if_else` — 余额为 0 则 `closeAccount`，rent 退用户。
- **Bridge 中间 WSOL ATA：** 无条件 `close_account`（hop 容器，leg2 后关闭）。
- Smart-close 指令在 **finalize** 时若导致 tx > 1232B 则跳过（`smartCloseApplied: false`）。

---

## 8. Sponsor（已实现）

### 8.1 启用条件

| 条件 | 说明 |
|------|------|
| `[sponsor].enabled` | 配置开关 |
| `[sponsor].keypair_path` | 服务端 co-sign（fee payer = sponsor） |
| 路由 eligible | **Bridge** 或 **Direct 输出 SOL** |

UI：开关在 **右侧 Trade preview**；首次询价前 disabled；询价后根据 `sponsorUi` 决定是否可点。

### 8.2 Fee payer 与签名

- Fee payer：**sponsor**
- 签名：**user + sponsor**（服务端 partial sign）
- `txFee = 2×5000 + ceil(compute_unit_limit × micro_lamports / 1e6)`（链下常量，按 priority 档位）

### 8.3 Repay（链上）

```text
ataCost  = Σ (ATA lamports after − before)   # sponsor bootstrap 前后 Ifx 测量
repay    = (ataCost + txFee) × (100 + repay_buffer_percent) / 100
```

**禁止**使用固定 rent 常数或 RPC 预查 missing rent 做 quote/build（避免 TOCTOU）。

### 8.4 还款来源

- **Bridge：** leg2 输入扣 `repay` 等值 WSOL → close WSOL → native SOL → `system_transfer` 给 sponsor。
- **Direct SOL 输出：** 全额 WSOL unwrap 后从用户 native SOL 转 repay（SOL 来自 swap proceeds，非用户原有余额）。

### 8.5 Assert

```text
quote_delta ≥ platform_fee + repay   # WSOL proceeds 必须覆盖
```

---

## 9. Platform fee

| 场景 | 计费 |
|------|------|
| 输出 WSOL / SPL | bps × **链上** proceeds delta（Ifx let） |
| 输入 WSOL（Direct） | bps × gross input，swap 前静态转 |

配置：`[service_fee] bps`、`pubkey`（fee recipient WSOL/SPL ATA 幂等创建）。

---

## 10. 交易体积

- 编译 **v0** message + `[solana].address_lookup_tables`
- 硬门控 **1232 bytes**；超出则 build 失败或 `fitsSizeGate: false`
- Bridge + sponsor 强依赖 ALT；Pump 上类似两跳 ~1339B，Raydium CPMM 账户更少，mainnet 可 fit

### ALT 规划工具

```bash
SOLANA_RPC_URL=https://… cargo run -p ifx-raydium --example alt_dump
```

输出 CPMM program、authority、常用池账户、fee ATA 等，供 `solana address-lookup-table extend --addresses "a,b,c"` 使用（**逗号分隔**，非 positional）。

---

## 11. 配置参考

完整示例：[config.toml.example](../config.toml.example)

| 段 | 用途 |
|----|------|
| `[solana]` | `rpc_url`、`commitment`、`address_lookup_tables` |
| `[ifx]` | `program_id`、`public_frames` |
| `[priority_fee]` | low/medium/high：`micro_lamports`、`compute_unit_limit` |
| `[service_fee]` | `bps`、`pubkey` |
| `[sponsor]` | `enabled`、`pubkey`、`keypair_path`、`repay_buffer_percent` |
| `[quote]` | `debounce_ms`、`default_slippage_bps` |
| `[trade]` | 默认 mint/amount、`sponsored` 默认 |
| `[server]` | `host`、`port`（默认 `127.0.0.1:8788`） |
| `[network]` | `http_proxy`、`raydium_api_timeout_secs` |
| `[wallet]` | 可选；Web UI 用浏览器钱包，此项仅作遗留/脚本 |

启动：

```bash
cargo run -p ifx-raydium-server
# 或指定配置
cargo run -p ifx-raydium-server -- /path/to/config.toml
```

日志：`RUST_LOG=debug` 可看路由/询价细节。

---

## 12. 明确未做（后续）

- USDC/USDT 作为 **bridge** 中间币（仅 SOL bridge）
- Sponsored 路径的 quote UI 精确扣 repay 预览（需 simulate；链上 repay 依 ataCost）

---

## 13. 决策记录

| 日期 | 决策 |
|------|------|
| 2026-06 | Rust workspace；Ifx 编排 |
| 2026-06 | CPMM；Direct ∥ SOL Bridge |
| 2026-06 | Web UI + Axum |
| 2026-06 | API v3 hint + RPC hydrate；Bridge 8×8 组合比价 |
| 2026-06 | Sponsor：链上 ataCost + 精确 txFee；WSOL 还款；leg2 min_out 链上缩放 |
| 2026-06 | Smart close 输入 ATA；Bridge 关闭 hop WSOL |
| 2026-06 | `[network].http_proxy` + API 超时 RPC 回退 |

---

## 14. 参考链接

- [Raydium API v3](https://docs.raydium.io/api-reference/api-v3/overview)
- [Ifx Rust SDK](https://github.com/ifx-run/ifx/tree/main/rust-sdk)
- [ifx-pumpfun-ext](https://github.com/ifx-run/ifx-pumpfun-ext)
