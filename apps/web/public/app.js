import { Connection, VersionedTransaction } from "@solana/web3.js";

const $ = (id) => document.getElementById(id);

let publicConfig = {
  debounceMs: 300,
  defaultSlippageBps: 100,
  rpcUrl: "https://api.mainnet-beta.solana.com",
  sponsorEnabled: false,
  sponsorAvailable: false,
};
let useSponsorPreference = false;
let quoteAbort = null;
let lastQuote = null;
let lastBuilt = null;
let expiresAtMs = 0;
let walletProvider = null;
let walletPubkey = null;
let debounceTimer = null;
let blockhashTimer = null;
let mintADebounceTimer = null;
let mintBDebounceTimer = null;
let mintAAbort = null;
let mintBAbort = null;
let tokenInfo = { mintA: null, mintB: null };
/** WSOL mint panels: ON = native SOL (wrap/unwrap), OFF = WSOL ATA. Both default ON. */
let payUseNativeSol = true;
let receiveAsNativeSol = true;

const BLOCKHASH_RETRIES = 3;
const SOL_MINT = "So11111111111111111111111111111111111111112";
const MIN_MINT_LEN = 32;
/** Keep native SOL for tx fee + wrap when using MAX. */
const SOL_MAX_RESERVE_LAMPORTS = 10_000_000n;

function getWalletProvider() {
  if (window.solana?.isPhantom) return window.solana;
  if (window.solflare?.isSolflare) return window.solflare;
  return window.solana ?? null;
}

function slippageBpsFromUi() {
  const pct = parseFloat($("slippage").value);
  if (!Number.isFinite(pct) || pct < 0) return publicConfig.defaultSlippageBps;
  return Math.round(pct * 100);
}

function isSolMint(mint) {
  return mint === SOL_MINT;
}

function getPayAssetMode() {
  if (!isSolMint($("mintA").value.trim())) return null;
  return payUseNativeSol ? "native_sol" : "wsol";
}

function getReceiveAssetMode() {
  if (!isSolMint($("mintB").value.trim())) return null;
  return receiveAsNativeSol ? "native_sol" : "wsol";
}

function updateMintLabels() {
  $("mintALabel").textContent = "Pay";
  $("mintBLabel").textContent = "Receive";
}

function solToggleMarkup(kind, enabled) {
  const isPay = kind === "pay";
  const title = isPay ? "Pay with SOL" : "Receive as SOL";
  const checked = enabled ? "checked" : "";
  return `
    <div class="token-panel-sol-toggle">
      <div class="token-panel-sol-toggle-head">
        <span class="token-panel-sol-toggle-title">${title}</span>
        <label class="sol-asset-switch" title="${title}">
          <input type="checkbox" data-sol-toggle="${kind}" role="switch" ${checked} />
          <span class="sol-asset-switch-slider" aria-hidden="true"></span>
        </label>
      </div>
    </div>`;
}

function applyPayNativeSolToggle(enabled = true) {
  payUseNativeSol = enabled;
  updateAmountLabel();
  updateBalancePctButtons();
  if (tokenInfo.mintA) renderTokenPanel("tokenAPanel", { info: tokenInfo.mintA });
  scheduleQuote();
}

function updateAmountLabel() {
  const mintA = $("mintA").value.trim();
  const el = $("amountLabel");
  if (!isSolMint(mintA)) {
    el.textContent = "Amount";
    $("inputAmount").placeholder = "Enter amount";
    return;
  }
  const mode = getPayAssetMode();
  if (mode === "native_sol") {
    el.textContent = "Amount (SOL)";
    $("inputAmount").placeholder = "Native SOL — tx will wrap to WSOL";
  } else {
    el.textContent = "Amount (WSOL)";
    $("inputAmount").placeholder = "WSOL in your ATA (So11… mint)";
  }
}

function formatRawToUi(raw, decimals) {
  if (decimals === 0) return raw.toString();
  const s = raw.toString().padStart(decimals + 1, "0");
  const whole = s.slice(0, -decimals) || "0";
  const frac = s.slice(-decimals).replace(/0+$/, "");
  return frac.length > 0 ? `${whole}.${frac}` : whole;
}

/** Pay-side balance for % buttons and pre-check (wallet must be connected). */
function getPayBalance() {
  if (!walletPubkey) return null;
  const info = tokenInfo.mintA;
  if (!info) return null;
  const mode = getPayAssetMode();
  try {
    if (mode === "native_sol") {
      if (info.balanceRaw == null) return null;
      const raw = BigInt(info.balanceRaw);
      return { raw, decimals: info.decimals, mint: info.mint, label: "SOL", mode };
    }
    if (mode === "wsol") {
      const raw = BigInt(info.wrappedBalanceRaw ?? "0");
      return { raw, decimals: info.decimals, mint: info.mint, label: "WSOL", mode };
    }
    if (info.balanceRaw == null) return null;
    const raw = BigInt(info.balanceRaw);
    if (raw <= 0n) return null;
    return { raw, decimals: info.decimals, mint: info.mint, label: info.label, mode: null };
  } catch {
    return null;
  }
}

function parseAmountToRaw(amountStr, decimals) {
  const amount = amountStr.trim();
  if (!amount) return null;
  const value = parseFloat(amount);
  if (!Number.isFinite(value) || value <= 0) return null;
  const scale = 10 ** decimals;
  const raw = BigInt(Math.round(value * scale));
  return raw > 0n ? raw : null;
}

function payAmountNeed(raw, bal) {
  if (bal.mode === "native_sol") {
    return raw + SOL_MAX_RESERVE_LAMPORTS;
  }
  return raw;
}

function maxPayAmountRaw(bal) {
  if (bal.mode === "native_sol") {
    let spendable = bal.raw;
    if (spendable > SOL_MAX_RESERVE_LAMPORTS) {
      return spendable - SOL_MAX_RESERVE_LAMPORTS;
    }
    return 0n;
  }
  return bal.raw;
}

function validatePayAmount(amountStr) {
  const mintA = $("mintA").value.trim();
  if (!walletPubkey || !amountStr) return null;
  const info = tokenInfo.mintA;
  if (!info) return null;
  const raw = parseAmountToRaw(amountStr, info.decimals);
  if (raw == null) return null;
  const bal = getPayBalance();
  if (!bal) {
    if (isSolMint(mintA) && getPayAssetMode() === "wsol") {
      const nativeRaw = info.balanceRaw != null ? BigInt(info.balanceRaw) : 0n;
      if (nativeRaw > 0n) {
        return 'Insufficient WSOL ATA. Turn on "Pay with SOL" in the WSOL panel above.';
      }
      return "Connect wallet and ensure WSOL ATA has balance (native SOL alone is not enough).";
    }
    return null;
  }
  let need = payAmountNeed(raw, bal);
  if (bal.raw < need) {
    return `Insufficient ${bal.label}: need ~${formatRawToUi(need, bal.decimals)}, have ${formatRawToUi(bal.raw, bal.decimals)}`;
  }
  return null;
}

function updateBalancePctButtons() {
  const bal = getPayBalance();
  const enabled = !!(bal && bal.raw > 0n);
  document.querySelectorAll(".pct-btn").forEach((btn) => {
    btn.disabled = !enabled;
  });
}

function applyBalancePercent(percent) {
  const bal = getPayBalance();
  if (!bal || bal.raw <= 0n) return;

  const pct = BigInt(percent);
  let amountRaw = pct >= 100n ? maxPayAmountRaw(bal) : (bal.raw * pct) / 100n;
  if (amountRaw <= 0n) return;

  $("inputAmount").value = formatRawToUi(amountRaw, bal.decimals);
  scheduleQuote();
}

function renderTokenPanel(panelId, opts) {
  const out = $(panelId);
  const isPayPanel = panelId === "tokenAPanel";
  if (!opts) {
    out.innerHTML = "";
    out.className = "token-panel hidden";
    return;
  }

  if (opts.error) {
    out.className = "token-panel token-panel--error";
    out.textContent = opts.error;
    out.classList.remove("hidden");
    return;
  }

  if (opts.loading) {
    out.className = "token-panel token-panel--loading";
    out.textContent = "Loading token info…";
    out.classList.remove("hidden");
    return;
  }

  const info = opts.info;
  if (!info) {
    out.innerHTML = "";
    out.className = "token-panel hidden";
    return;
  }

  let balanceBlock;
  if (isSolMint(info.mint)) {
    const wsolUi = info.wrappedBalanceUi ?? "0";
    const toggle = solToggleMarkup(
      isPayPanel ? "pay" : "receive",
      isPayPanel ? payUseNativeSol : receiveAsNativeSol
    );
    if (info.balanceUi != null) {
      balanceBlock = `
      <div class="token-panel-balance">
        <span>Native SOL</span>
        <strong>${info.balanceUi} SOL</strong>
      </div>
      <div class="token-panel-balance token-panel-balance--wsol">
        <span>WSOL (ATA)</span>
        <strong>${wsolUi} WSOL</strong>
      </div>
      ${toggle}`;
    } else if (walletPubkey) {
      balanceBlock = `
      <div class="token-panel-balance muted">
        <span>Your balance</span>
        <strong>0 SOL / WSOL</strong>
      </div>
      ${toggle}`;
    } else {
      balanceBlock = `
      <div class="token-panel-note muted">Connect wallet to see balances</div>
      ${toggle}`;
    }
  } else if (info.balanceUi != null) {
      balanceBlock = `
      <div class="token-panel-balance">
        <span>Balance</span>
        <strong>${info.balanceUi}</strong>
      </div>`;
  } else if (walletPubkey) {
    balanceBlock = `
      <div class="token-panel-balance muted">
        <span>Balance</span>
        <strong>0</strong>
      </div>`;
  } else {
    balanceBlock = `<div class="token-panel-note muted">Connect wallet to see balance</div>`;
  }

  if (!balanceBlock) {
    out.className = "token-panel hidden";
    out.innerHTML = "";
    return;
  }

  out.className = "token-panel";
  out.innerHTML = balanceBlock;
  out.classList.remove("hidden");
}

async function fetchTokenInfo(mint, signal) {
  return fetch("/api/token/info", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      mint,
      userPubkey: walletPubkey ?? undefined,
    }),
    signal,
  }).then(async (res) => {
    const data = await res.json();
    if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
    return data;
  });
}

async function resolveMint(side) {
  const inputId = side === "A" ? "mintA" : "mintB";
  const panelId = side === "A" ? "tokenAPanel" : "tokenBPanel";
  const key = side === "A" ? "mintA" : "mintB";
  const mint = $(inputId).value.trim();

  if (mint.length < MIN_MINT_LEN) {
    tokenInfo[key] = null;
    renderTokenPanel(panelId, null);
    if (side === "A") updateMintLabels();
    if (side === "B") updateMintLabels();
    return;
  }

  const prevAbort = side === "A" ? mintAAbort : mintBAbort;
  if (prevAbort) prevAbort.abort();
  const ac = new AbortController();
  if (side === "A") mintAAbort = ac;
  else mintBAbort = ac;

  renderTokenPanel(panelId, { loading: true });

  try {
    const info = await fetchTokenInfo(mint, ac.signal);
    tokenInfo[key] = info;
    renderTokenPanel(panelId, { info });
  } catch (e) {
    if (e.name === "AbortError") return;
    tokenInfo[key] = null;
    renderTokenPanel(panelId, { error: e.message ?? String(e) });
  } finally {
    if (side === "A" && mintAAbort === ac) mintAAbort = null;
    if (side === "B" && mintBAbort === ac) mintBAbort = null;
    updateMintLabels();
    if (side === "A") {
      updateBalancePctButtons();
      updateAmountLabel();
    }
  }
}

function scheduleMintResolve(side) {
  clearTimeout(side === "A" ? mintADebounceTimer : mintBDebounceTimer);
  const delay = publicConfig.debounceMs ?? 300;
  const t = setTimeout(() => resolveMint(side), delay);
  if (side === "A") mintADebounceTimer = t;
  else mintBDebounceTimer = t;
}

function refreshMintPanels() {
  const mintA = $("mintA").value.trim();
  const mintB = $("mintB").value.trim();
  if (mintA.length >= MIN_MINT_LEN) resolveMint("A");
  if (mintB.length >= MIN_MINT_LEN) resolveMint("B");
}

async function swapMintFields() {
  const mintAEl = $("mintA");
  const mintBEl = $("mintB");
  const tmp = mintAEl.value;
  mintAEl.value = mintBEl.value;
  mintBEl.value = tmp;

  lastQuote = null;
  lastBuilt = null;
  tokenInfo = { mintA: null, mintB: null };
  stopBlockhashCountdown();
  renderBanner(null);

  await Promise.all([resolveMint("A"), resolveMint("B")]);
  updateBalancePctButtons();
  updateMintLabels();
  updateAmountLabel();
  const amount = $("inputAmount").value.trim();
  if (amount) await doQuote();
}

function shortMint(m) {
  if (!m || m.length < 12) return m ?? "—";
  return `${m.slice(0, 4)}…${m.slice(-4)}`;
}

async function api(path, body) {
  const opts = body
    ? {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      }
    : {};
  const res = await fetch(path, opts);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error(data.error ?? `HTTP ${res.status}`);
  }
  return data;
}

async function loadPublicConfig() {
  publicConfig = await api("/api/config/public");
  $("slippage").value = (publicConfig.defaultSlippageBps / 100).toString();
  $("mintA").value = publicConfig.defaultMintA ?? "";
  $("mintB").value = publicConfig.defaultMintB ?? "";
  $("inputAmount").value = "";
  hideSponsorUi();
  refreshMintPanels();
  updateMintLabels();
  updateAmountLabel();
}

function hideSponsorUi() {
  $("sponsorRow").classList.add("hidden");
  $("sponsorHint").textContent = "";
  const toggle = $("sponsorToggle");
  if (toggle) {
    toggle.checked = false;
    toggle.disabled = true;
  }
}

function renderSponsorUi(ui) {
  const row = $("sponsorRow");
  const toggle = $("sponsorToggle");
  if (!ui?.visible) {
    hideSponsorUi();
    return;
  }
  row.classList.remove("hidden");
  row.classList.toggle("trade-sponsor-row--readonly", !!ui.readonly);
  toggle.disabled = !!ui.readonly;
  toggle.checked = !!ui.enabled;
  $("sponsorHint").textContent = ui.hint ?? "";
}

function quotePayload(build) {
  const mintA = $("mintA").value.trim();
  const payload = {
    mintA,
    mintB: $("mintB").value.trim(),
    inputAmount: $("inputAmount").value.trim(),
    slippageBps: slippageBpsFromUi(),
    userPubkey: walletPubkey ?? undefined,
    priorityTier: $("priorityTier").value,
    useSponsor: $("sponsorToggle")?.checked ?? useSponsorPreference,
    build: build && !!walletPubkey,
  };
  if (isSolMint(mintA)) {
    payload.payAsset = getPayAssetMode();
  }
  if (isSolMint($("mintB").value.trim())) {
    payload.receiveAsset = getReceiveAssetMode();
  }
  return payload;
}

function setSummaryIdle(msg = "Enter amount on the left to preview.") {
  $("tradeSummary").className = "trade-summary trade-summary--idle";
  $("summaryPayAmount").textContent = "—";
  $("summaryReceiveAmount").textContent = "—";
  $("summarySub").textContent = msg;
  $("summaryStats").classList.add("hidden");
  $("summaryAlerts").innerHTML = "";
  hideSponsorUi();
  lastQuote = null;
  lastBuilt = null;
  stopBlockhashCountdown();
  renderBanner(null);
  renderTxInspector(null, "Enter amount to preview");
  updateActionButtons();
}

function setSummaryLoading() {
  $("tradeSummary").className = "trade-summary trade-summary--loading";
  $("summarySub").textContent = "Quoting & building…";
  $("summaryPayAmount").textContent = "…";
  $("summaryReceiveAmount").textContent = "…";
  $("summaryStats").classList.add("hidden");
  $("summaryAlerts").innerHTML = "";
  renderBanner(null);
  renderTxInspector(null, "Quoting & building…");
}

function setSummaryError(msg) {
  $("tradeSummary").className = "trade-summary trade-summary--error";
  $("summarySub").textContent = msg;
  const alerts = $("summaryAlerts");
  alerts.innerHTML = "";
  const needsNative =
    /switch pay asset to native SOL/i.test(msg) ||
    /Pay with.*native SOL/i.test(msg) ||
    /WSOL ATA/.test(msg);
  if (needsNative && isSolMint($("mintA").value.trim())) {
    alerts.innerHTML = `<p class="trade-alert trade-alert-warn">${
      getPayAssetMode() === "wsol"
        ? "Paying with WSOL but ATA balance is zero."
        : "Balance check failed."
    }</p>
      <div class="trade-alert-action">
        <button type="button" class="secondary" id="switchPayNativeBtn">Switch to native SOL</button>
      </div>`;
    $("switchPayNativeBtn")?.addEventListener("click", () => applyPayNativeSolToggle(true));
  }
  renderBanner(null);
  renderTxInspector(null, "Quote failed");
}

function setSummaryQuote(q) {
  $("tradeSummary").className = "trade-summary trade-summary--ready";
  $("summaryPayAmount").textContent = q.inputAmountUi;
  $("summaryReceiveAmount").textContent = q.expectedOutUi;
  let sub = q.routeMessage;
  if (isSolMint(q.mintB)) {
    sub += ` · Receive as ${getReceiveAssetMode() === "native_sol" ? "native SOL" : "WSOL"}`;
  }
  $("summarySub").textContent = sub;

  if (q.sponsorUi) {
    renderSponsorUi(q.sponsorUi);
    if (q.sponsorUi.visible && !q.sponsorUi.readonly) {
      useSponsorPreference = !!q.sponsorUi.enabled;
    }
  }

  const sponsorStat = (() => {
    if (!publicConfig.sponsorAvailable) return "";
    if (q.built?.useSponsor) {
      return `<div class="trade-stat"><dt>Sponsored</dt><dd>yes · payer <code>${shortMint(q.built.feePayer)}</code></dd></div>`;
    }
    if (q.sponsorEligible && q.sponsorUi?.enabled) {
      return `<div class="trade-stat"><dt>Sponsored</dt><dd>yes (preview)</dd></div>`;
    }
    if (q.sponsorEligible) {
      return `<div class="trade-stat"><dt>Sponsored</dt><dd>no</dd></div>`;
    }
    return `<div class="trade-stat"><dt>Sponsored</dt><dd>n/a for this route</dd></div>`;
  })();

  const smartCloseStat =
    q.built?.smartCloseApplied != null
      ? `<div class="trade-stat"><dt>Smart close</dt><dd>${q.built.smartCloseApplied ? "yes" : "skipped (tx size)"}</dd></div>`
      : "";

  const stats = $("summaryStats");
  stats.classList.remove("hidden");
  stats.innerHTML = `
    <div class="trade-stat"><dt>Route</dt><dd>${q.routeKind}</dd></div>
    <div class="trade-stat"><dt>Direct</dt><dd>${q.directAvailable ? "yes" : "no"}</dd></div>
    <div class="trade-stat"><dt>Bridge</dt><dd>${q.bridgeAvailable ? "yes" : "no"}</dd></div>
    ${sponsorStat}
    ${smartCloseStat}
    <div class="trade-stat"><dt>Pay mint</dt><dd><code>${shortMint(q.mintA)}</code></dd></div>
    <div class="trade-stat"><dt>Receive mint</dt><dd><code>${shortMint(q.mintB)}</code></dd></div>
  `;

  const alerts = $("summaryAlerts");
  alerts.innerHTML = "";
  if (q.built && !q.built.fitsSizeGate) {
    alerts.innerHTML = `<p class="trade-alert trade-alert-warn">Tx ${q.built.transactionSizeBytes} B — exceeds 1232 B gate</p>`;
  }
}

function updateActionButtons() {
  const canAct = !!lastBuilt?.transactionBase64 && !!walletPubkey;
  $("signSendBtn").disabled = !canAct;
  $("simulateBtn").disabled = !canAct;
}

function startBlockhashCountdown(lastValidBlockHeight) {
  stopBlockhashCountdown();
  if (!lastValidBlockHeight) return;
  const connection = new Connection(publicConfig.rpcUrl, "confirmed");
  expiresAtMs = Date.now() + 55_000;
  const el = $("blockhashCountdown");
  el.classList.remove("hidden");
  blockhashTimer = setInterval(async () => {
    const left = Math.max(0, Math.ceil((expiresAtMs - Date.now()) / 1000));
    el.textContent = left > 0 ? `Blockhash ~${left}s` : "Blockhash expired";
    el.classList.toggle("expired", left === 0);
    if (left === 0) {
      stopBlockhashCountdown();
    }
  }, 1000);
}

function stopBlockhashCountdown() {
  if (blockhashTimer) clearInterval(blockhashTimer);
  blockhashTimer = null;
  $("blockhashCountdown").classList.add("hidden");
}

async function doQuote({ silent = false } = {}) {
  const mintA = $("mintA").value.trim();
  const mintB = $("mintB").value.trim();
  const amount = $("inputAmount").value.trim();
  if (mintA.length < MIN_MINT_LEN || mintB.length < MIN_MINT_LEN) {
    if (!silent) setSummaryIdle("Enter mint addresses to preview.");
    return;
  }
  if (!amount) {
    if (!silent) setSummaryIdle();
    return;
  }

  const balanceErr = validatePayAmount(amount);
  if (balanceErr) {
    if (!silent) setSummaryError(balanceErr);
    return;
  }

  if (quoteAbort) quoteAbort.abort();
  quoteAbort = new AbortController();
  const signal = quoteAbort.signal;

  if (!silent) setSummaryLoading();

  try {
    const q = await fetch("/api/quote", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(quotePayload(true)),
      signal,
    }).then(async (res) => {
      const data = await res.json();
      if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
      return data;
    });

    lastQuote = q;
    lastBuilt = q.built ?? null;
    setSummaryQuote(q);
    updateActionButtons();
    renderBanner(null);

    if (lastBuilt) {
      startBlockhashCountdown(lastBuilt.lastValidBlockHeight);
      renderTxInspector(
        lastBuilt.inspection,
        `${lastBuilt.transactionSizeBytes} B · ${lastBuilt.routeKind} · ready`
      );
    } else if (walletPubkey) {
      renderTxInspector(null, "Quote OK — connect wallet or build failed");
    } else {
      renderTxInspector(null, "Quote OK — connect wallet to build tx");
    }
  } catch (e) {
    if (e.name === "AbortError") return;
    console.error(e);
    if (!silent) setSummaryError(e.message ?? String(e));
  } finally {
    quoteAbort = null;
  }
}

function scheduleQuote() {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => doQuote(), publicConfig.debounceMs ?? 300);
}

function acctFlags(isSigner, isWritable) {
  const parts = [];
  if (isSigner) parts.push("signer");
  if (isWritable) parts.push("writable");
  return parts.length ? parts.join(", ") : "readonly";
}

function acctResolveLabel(a) {
  if (a.altLoaded) return a.resolution ?? "ALT";
  if (a.inAltTableUnused) return "static†";
  return "static";
}

function wireCopyButtons(root) {
  root.querySelectorAll("[data-copy]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const text = btn.getAttribute("data-copy") ?? "";
      try {
        await navigator.clipboard.writeText(text);
        const prev = btn.textContent;
        btn.textContent = "Copied";
        setTimeout(() => {
          btn.textContent = prev;
        }, 1200);
      } catch {
        /* ignore */
      }
    });
  });
}

function renderTxInspector(inspection, statusText) {
  const status = $("inspectorStatus");
  const meta = $("inspectorMeta");
  const list = $("inspectorInstructions");
  const rawWrap = $("inspectorRaw");
  const rawPre = $("inspectorRawPre");

  status.textContent = statusText;

  if (!inspection) {
    meta.classList.add("hidden");
    rawWrap.classList.add("hidden");
    list.innerHTML = "";
    return;
  }

  meta.classList.remove("hidden");
  rawWrap.classList.remove("hidden");

  meta.innerHTML = `
    <div class="meta-kv"><span>Version</span><code>v${inspection.version}</code></div>
    <div class="meta-kv"><span>Instructions</span><code>${inspection.numInstructions}</code></div>
    <div class="meta-kv"><span>Tx size</span><code>${inspection.transactionSizeBytes ?? "—"} B</code></div>
    <div class="meta-kv"><span>Account keys</span><code>${inspection.totalAccountKeys} (${inspection.staticAccountKeys} static + ${inspection.loadedWritableAccounts}W/${inspection.loadedReadonlyAccounts}R ALT)</code></div>
    <div class="meta-kv"><span>Frame</span><code>${inspection.frameUsed ?? "—"}</code></div>
    <div class="meta-kv"><span>Fee payer</span><code>${inspection.feePayer ?? "—"}</code></div>
    <div class="meta-kv"><span>Smart close</span><code>${
      inspection.smartCloseApplied == null
        ? "—"
        : inspection.smartCloseApplied
          ? "applied"
          : "skipped"
    }</code></div>
    <div class="meta-kv"><span>ALTs</span><code>${inspection.addressLookupTables?.length ?? 0}</code></div>
  `;

  list.innerHTML = inspection.instructions
    .map(
      (ix) => `
    <article class="ix-card">
      <header class="ix-card-head">
        <span class="ix-index">#${ix.index}</span>
        <span class="ix-program">${ix.programLabel}</span>
        ${ix.hint ? `<span class="ix-hint">${ix.hint}</span>` : ""}
        <span class="ix-program-id">${ix.programId}</span>
      </header>
      <table class="ix-accounts">
        <thead>
          <tr><th>#</th><th>Account</th><th>Flags</th><th>Resolve</th></tr>
        </thead>
        <tbody>
          ${ix.accounts
            .map(
              (a, i) => `
            <tr>
              <td>${i}</td>
              <td><code>${a.pubkey}</code></td>
              <td class="acct-flags">${acctFlags(a.isSigner, a.isWritable)}</td>
              <td class="acct-resolve ${a.altLoaded ? "resolve-alt" : a.inAltTableUnused ? "resolve-static-unused" : "resolve-static"}">${acctResolveLabel(a)}</td>
            </tr>`
            )
            .join("")}
        </tbody>
      </table>
      <div class="ix-data">
        <div class="ix-data-label">
          <span>Data · ${ix.dataLength} bytes</span>
          <button type="button" class="secondary copy-btn" data-copy="${ix.dataHex}">Copy hex</button>
        </div>
        <pre>${ix.dataHex}</pre>
      </div>
    </article>`
    )
    .join("");

  rawPre.textContent = JSON.stringify(inspection, null, 2);
  wireCopyButtons(list);
}

let bannerDismissTimer = null;

function clearBanner() {
  if (bannerDismissTimer) {
    clearTimeout(bannerDismissTimer);
    bannerDismissTimer = null;
  }
  const el = $("inspectorBanner");
  el.className = "inspector-banner hidden";
  el.innerHTML = "";
}

function renderBanner(opts) {
  if (bannerDismissTimer) {
    clearTimeout(bannerDismissTimer);
    bannerDismissTimer = null;
  }
  if (!opts) {
    clearBanner();
    return;
  }

  const { kind, title, message, signature, built, execution } = opts;
  const el = $("inspectorBanner");
  el.classList.remove("hidden", "banner-success", "banner-error", "banner-progress", "banner-cancelled");
  if (kind === "success") el.classList.add("banner-success");
  if (kind === "error") el.classList.add("banner-error");
  if (kind === "progress") el.classList.add("banner-progress");
  if (kind === "cancelled") el.classList.add("banner-cancelled");

  let body = "";
  if (signature && (kind === "success" || kind === "error")) {
    const url = solscanTxUrl(signature);
    const result = executionResultLabel(execution);
    const errHint = execution?.succeeded === false
      ? executionErrorHint(execution.err, built?.inspection)
      : null;
    const errLine = execution?.succeeded === false && execution.err
      ? `<code class="error">${execution.err}</code>${errHint ? `<div class="muted" style="margin-top:0.25rem">${errHint}</div>` : ""}`
      : "";
    body = `
      <a class="banner-link" href="${url}" target="_blank" rel="noopener">View on Solscan →</a>
      <div class="banner-grid">
        <div>Result<code class="${result.tone}">${result.text}</code>${errLine}</div>
        <div>Signature<code>${signature}</code></div>
        <div>Route<code>${built?.routeKind ?? "—"}</code></div>
        <div>Fee payer<code>${built?.feePayer ?? walletPubkey ?? "—"}</code></div>
        <div>Tx size<code>${built?.transactionSizeBytes ?? "—"} B</code></div>
        ${
          built?.partiallySignedBy
            ? `<div>Sponsor<code>${shortMint(built.partiallySignedBy)}</code></div>`
            : ""
        }
      </div>`;
  } else if (message) {
    body = `<div class="muted">${message}</div>`;
  }

  el.innerHTML = `
    <div class="banner-title ${kind === "success" ? "ok" : kind === "error" ? "error" : ""}">${title}</div>
    ${body}
  `;

  if (kind === "cancelled") {
    bannerDismissTimer = setTimeout(() => clearBanner(), 5000);
  }
}

async function doSimulate() {
  if (!lastBuilt?.transactionBase64) return;
  renderBanner({ kind: "progress", title: "Simulating", message: "RPC dry-run…" });
  try {
    const result = await api("/api/tx/simulate", {
      transactionBase64: lastBuilt.transactionBase64,
      replaceRecentBlockhash: true,
    });
    if (result.err) {
      const hint = simulateErrorHint(result.err, lastBuilt.inspection);
      const logs = result.logs?.length
        ? `<details style="margin-top:0.5rem"><summary>Program logs</summary><pre style="max-height:8rem;overflow:auto;font-size:0.72rem">${result.logs.join("\n")}</pre></details>`
        : "";
      renderBanner({
        kind: "error",
        title: "Simulation failed",
        message: `${hint}<br/><code>${result.err}</code>${logs}`,
      });
    } else {
      const units = result.unitsConsumed ?? "?";
      renderBanner({
        kind: "success",
        title: "Simulation OK",
        message: `Compute units: ${units}`,
      });
    }
  } catch (e) {
    renderBanner({ kind: "error", title: "Simulate error", message: e.message });
  }
}

function solscanTxUrl(signature) {
  const rpc = publicConfig.rpcUrl ?? "";
  if (rpc.includes("devnet")) return `https://solscan.io/tx/${signature}?cluster=devnet`;
  if (rpc.includes("testnet")) return `https://solscan.io/tx/${signature}?cluster=testnet`;
  return `https://solscan.io/tx/${signature}`;
}

function formatTxErr(err) {
  if (err == null) return null;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

async function fetchTxExecutionResult(connection, signature) {
  for (let attempt = 0; attempt < 8; attempt++) {
    const { value } = await connection.getSignatureStatuses([signature], {
      searchTransactionHistory: true,
    });
    const status = value[0];
    if (status) {
      return {
        landed: true,
        succeeded: status.err == null,
        err: formatTxErr(status.err),
      };
    }
    if (attempt < 7) {
      await new Promise((resolve) => setTimeout(resolve, 400));
    }
  }
  return { landed: false, succeeded: null, err: null };
}

function executionResultLabel(execution) {
  if (!execution?.landed) return { text: "Unknown", tone: "muted" };
  if (execution.succeeded) return { text: "Success", tone: "ok" };
  return { text: "Failed", tone: "error" };
}

function isBlockhashSendError(err) {
  const msg = String(err?.message ?? err).toLowerCase();
  return (
    msg.includes("blockhash") ||
    msg.includes("block height exceeded") ||
    msg.includes("transaction expired")
  );
}

function executionErrorHint(errRaw, inspection) {
  if (!errRaw) return null;
  const hint = simulateErrorHint(errRaw, inspection);
  return hint === "Transaction would fail on-chain." ? null : hint;
}

function simulateErrorHint(err, inspection) {
  const ixMatch = err.match(/InstructionError\((\d+),\s*Custom\((\d+)\)\)/);
  if (!ixMatch) return "Transaction would fail on-chain.";
  const ixIndex = Number(ixMatch[1]);
  const code = Number(ixMatch[2]);
  const ix = inspection?.instructions?.[ixIndex];
  const label = ix ? `#${ixIndex} ${ix.programLabel}${ix.hint ? ` (${ix.hint})` : ""}` : `#${ixIndex}`;
  if (code === 1) {
    return `Instruction ${label} failed: insufficient token balance (SPL Token error 1). For SOL swaps, ensure wallet SOL covers amount + fee + rent.`;
  }
  if (code >= 6000) {
    return `Instruction ${label} failed: Raydium CPMM error ${code} (e.g. slippage exceeded if 6005).`;
  }
  return `Instruction ${label} failed with program error ${code}.`;
}

async function signAndSend(connection, built) {
  const bytes = Uint8Array.from(atob(built.transactionBase64), (c) => c.charCodeAt(0));
  const tx = VersionedTransaction.deserialize(bytes);
  const signed = await walletProvider.signTransaction(tx);
  const signature = await connection.sendRawTransaction(signed.serialize(), {
    skipPreflight: false,
    preflightCommitment: "confirmed",
  });

  const confirmTarget =
    built.recentBlockhash && built.lastValidBlockHeight
      ? {
          signature,
          blockhash: built.recentBlockhash,
          lastValidBlockHeight: built.lastValidBlockHeight,
        }
      : signature;
  await connection.confirmTransaction(confirmTarget, "confirmed");

  const execution = await fetchTxExecutionResult(connection, signature);
  return { signature, execution };
}

async function doSignSend() {
  if (!walletPubkey || !lastBuilt) return;

  const connection = new Connection(publicConfig.rpcUrl, "confirmed");

  for (let attempt = 1; attempt <= BLOCKHASH_RETRIES; attempt++) {
    if (attempt > 1) {
      await doQuote({ silent: true });
      if (!lastBuilt) return;
    }

    renderBanner({
      kind: "progress",
      title: "Awaiting signature",
      message: `${lastBuilt.transactionSizeBytes ?? "?"} B · approve in wallet`,
    });

    try {
      const { signature, execution } = await signAndSend(connection, lastBuilt);
      stopBlockhashCountdown();
      const failedOnChain = execution.succeeded === false;
      const succeeded = execution.succeeded === true;
      renderBanner({
        kind: failedOnChain ? "error" : "success",
        title: failedOnChain
          ? "Transaction failed on-chain"
          : succeeded
            ? "Transaction succeeded"
            : "Transaction confirmed",
        built: lastBuilt,
        signature,
        execution,
      });
      renderTxInspector(
        lastBuilt.inspection,
        failedOnChain
          ? "Failed on-chain — inspect instructions below"
          : succeeded
            ? "Succeeded — inspect instructions below"
            : "Confirmed — inspect instructions below"
      );
      return;
    } catch (e) {
      if (isBlockhashSendError(e) && attempt < BLOCKHASH_RETRIES) continue;
      if (e.code === 4001 || String(e.message).includes("User rejected")) {
        renderBanner({
          kind: "cancelled",
          title: "Cancelled in wallet",
          message: "Transaction not sent.",
        });
        return;
      }
      if (attempt < BLOCKHASH_RETRIES) continue;
      renderBanner({ kind: "error", title: "Send failed", message: e.message ?? String(e) });
    }
  }
}

function applyWalletSession(provider, pubkey) {
  walletProvider = provider;
  walletPubkey =
    typeof pubkey === "string"
      ? pubkey
      : pubkey?.toString?.() ?? provider.publicKey?.toString?.() ?? null;
}

function updateWalletUi() {
  const status = $("walletStatus");
  if (walletPubkey) {
    status.textContent = `Connected: ${shortMint(walletPubkey)}`;
    status.className = "wallet-status ok";
    $("connectBtn").classList.add("hidden");
    $("disconnectBtn").classList.remove("hidden");
  } else {
    status.textContent = "Wallet not connected";
    status.className = "wallet-status muted";
    $("connectBtn").classList.remove("hidden");
    $("disconnectBtn").classList.add("hidden");
  }
  updateBalancePctButtons();
  updateActionButtons();
}

async function onWalletConnected() {
  updateWalletUi();
  refreshMintPanels();
  await doQuote();
}

function onWalletDisconnected() {
  walletProvider = null;
  walletPubkey = null;
  lastBuilt = null;
  updateWalletUi();
  refreshMintPanels();
  doQuote();
}

/** Restore session if extension still connected, or silently reconnect a trusted site. */
async function tryAutoConnectWallet() {
  const provider = getWalletProvider();
  if (!provider) return false;

  if (provider.isConnected && provider.publicKey) {
    applyWalletSession(provider, provider.publicKey);
    return true;
  }

  try {
    const resp = await provider.connect({ onlyIfTrusted: true });
    applyWalletSession(provider, resp.publicKey);
    return true;
  } catch {
    return false;
  }
}

async function connectWallet() {
  const provider = getWalletProvider();
  if (!provider) {
    alert("Install Phantom or Solflare");
    return;
  }
  const resp = await provider.connect();
  applyWalletSession(provider, resp.publicKey);
  await onWalletConnected();
}

function disconnectWallet() {
  try {
    walletProvider?.disconnect?.();
  } catch {
    /* ignore */
  }
  onWalletDisconnected();
}

function wireSolPanelToggle(panelId, kind) {
  $(panelId).addEventListener("change", (e) => {
    const input = e.target;
    if (input?.dataset?.solToggle !== kind) return;
    if (kind === "pay") {
      payUseNativeSol = input.checked;
      updateAmountLabel();
      updateBalancePctButtons();
    } else {
      receiveAsNativeSol = input.checked;
    }
    scheduleQuote();
  });
}

function wireEvents() {
  wireSolPanelToggle("tokenAPanel", "pay");
  wireSolPanelToggle("tokenBPanel", "receive");

  $("mintA").addEventListener("input", () => {
    updateMintLabels();
    updateAmountLabel();
    scheduleMintResolve("A");
    scheduleQuote();
  });
  $("mintA").addEventListener("change", () => {
    updateMintLabels();
    updateAmountLabel();
    scheduleMintResolve("A");
    scheduleQuote();
  });
  $("mintB").addEventListener("input", () => {
    updateMintLabels();
    scheduleMintResolve("B");
    scheduleQuote();
  });
  $("mintB").addEventListener("change", () => {
    updateMintLabels();
    scheduleMintResolve("B");
    scheduleQuote();
  });
  $("swapMintsBtn").addEventListener("click", () => {
    swapMintFields().catch((e) => console.error("swap mints", e));
  });
  ["inputAmount", "slippage", "priorityTier"].forEach((id) => {
    $(id).addEventListener("input", scheduleQuote);
    $(id).addEventListener("change", scheduleQuote);
  });
  document.querySelectorAll(".pct-btn").forEach((btn) => {
    btn.addEventListener("click", () => applyBalancePercent(Number(btn.dataset.pct)));
  });
  $("sponsorToggle")?.addEventListener("change", () => {
    if ($("sponsorToggle").disabled) return;
    useSponsorPreference = $("sponsorToggle").checked;
    scheduleQuote();
  });
  $("refreshQuoteBtn").addEventListener("click", () => doQuote());
  $("connectBtn").addEventListener("click", connectWallet);
  $("disconnectBtn").addEventListener("click", disconnectWallet);
  $("simulateBtn").addEventListener("click", doSimulate);
  $("signSendBtn").addEventListener("click", doSignSend);
}

async function init() {
  try {
    await loadPublicConfig();
  } catch (e) {
    console.error("config load failed", e);
  }
  wireEvents();
  updateMintLabels();
  updateAmountLabel();

  const provider = getWalletProvider();
  provider?.on?.("connect", async (pk) => {
    const next = pk?.toString?.() ?? provider.publicKey?.toString?.();
    if (!next || next === walletPubkey) return;
    applyWalletSession(provider, next);
    await onWalletConnected();
  });
  provider?.on?.("disconnect", () => {
    onWalletDisconnected();
  });

  updateWalletUi();
  if (await tryAutoConnectWallet()) {
    updateWalletUi();
    refreshMintPanels();
    if ($("inputAmount").value.trim()) await doQuote();
  }
}

init();
