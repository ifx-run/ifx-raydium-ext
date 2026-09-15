//! HTTP API handlers.

use crate::amount::{format_raw_amount, mint_decimals, parse_human_amount};
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ifx_raydium::api::{ApiClient, HttpClientOptions};
use ifx_raydium::constants::NATIVE_MINT;
use ifx_raydium::rpc::RpcPoolLoader;
use ifx_raydium_config::{AppConfig, PriorityTier};
use ifx_raydium_planner::build::{BuildPlan, Planner};
use ifx_raydium_planner::route::{RouteKind, RouteQuote, Router};
use ifx_raydium_planner::finalize::{finalize_build, FinalizeRequest};
use ifx_raydium_planner::simulate::{simulate_transaction, SimulateRequest};
use ifx_raydium_planner::sol_pay_asset_for_mint;
use ifx_raydium_planner::TxInspection;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use tracing::{info, warn};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub ok: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicConfigResponse {
    pub debounce_ms: u64,
    pub default_slippage_bps: u16,
    pub service_fee_bps: u16,
    pub service_fee_recipient_pubkey: String,
    pub sponsor_enabled: bool,
    /// True when sponsor is configured for server-side co-sign (`enabled` + `keypair_path`).
    pub sponsor_available: bool,
    pub sponsor_pubkey: String,
    pub public_frame_count: usize,
    pub priority_tiers: Vec<&'static str>,
    pub default_priority_tier: String,
    pub rpc_url: String,
    pub address_lookup_table_count: usize,
    /// 0 = v0 + ALT (1232 B). 1 = Solana tx v1 (4096 B).
    pub transaction_version: u8,
    pub default_mint_a: String,
    pub default_mint_b: String,
    pub default_amount: String,
    pub raydium_api_timeout_secs: u64,
    pub http_proxy_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRequest {
    pub mint_a: String,
    pub mint_b: String,
    pub input_amount: String,
    #[serde(default)]
    pub slippage_bps: Option<u16>,
    #[serde(default)]
    pub user_pubkey: Option<String>,
    #[serde(default)]
    pub priority_tier: Option<String>,
    #[serde(default)]
    pub use_sponsor: Option<bool>,
    /// `wsol` (default) or `native_sol` when mint A is WSOL (`So11…`).
    #[serde(default)]
    pub pay_asset: Option<String>,
    /// `wsol` (default) or `native_sol` when mint B is WSOL (`So11…`).
    #[serde(default)]
    pub receive_asset: Option<String>,
    /// When true and user_pubkey is set, also build the transaction.
    #[serde(default)]
    pub build: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildRequest {
    pub mint_a: String,
    pub mint_b: String,
    pub input_amount: String,
    pub user_pubkey: String,
    #[serde(default)]
    pub slippage_bps: Option<u16>,
    #[serde(default)]
    pub priority_tier: Option<String>,
    #[serde(default)]
    pub use_sponsor: Option<bool>,
    #[serde(default)]
    pub pay_asset: Option<String>,
    #[serde(default)]
    pub receive_asset: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateRequestBody {
    pub transaction_base64: String,
    #[serde(default = "default_replace_blockhash")]
    pub replace_recent_blockhash: bool,
}

fn default_replace_blockhash() -> bool {
    true
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltTxResponse {
    pub transaction_base64: String,
    pub transaction_size_bytes: usize,
    pub fits_size_gate: bool,
    /// 0 = v0 + ALT. 1 = Solana tx v1.
    pub transaction_version: u8,
    pub fee_payer: String,
    pub recent_blockhash: String,
    pub last_valid_block_height: u64,
    pub partially_signed_by: Option<String>,
    pub inspection: TxInspection,
    pub route_kind: String,
    pub route_message: String,
    pub input_amount_raw: String,
    pub expected_out_raw: String,
    pub use_sponsor: bool,
    pub smart_close_applied: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SponsorUiResponse {
    pub visible: bool,
    /// `hidden` | `optional` | `readonly_off`
    pub mode: String,
    pub enabled: bool,
    pub readonly: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteResponse {
    pub route_kind: String,
    pub route_message: String,
    pub input_amount_raw: String,
    pub input_amount_ui: String,
    pub expected_out_raw: String,
    pub expected_out_ui: String,
    pub mint_a: String,
    pub mint_b: String,
    pub direct_available: bool,
    pub bridge_available: bool,
    pub sponsor_eligible: bool,
    pub sponsor_ui: SponsorUiResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub built: Option<BuiltTxResponse>,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

pub async fn public_config(State(state): State<AppState>) -> Json<PublicConfigResponse> {
    let cfg = &state.config;
    Json(PublicConfigResponse {
        debounce_ms: cfg.quote.debounce_ms,
        default_slippage_bps: cfg.quote.default_slippage_bps,
        service_fee_bps: cfg.service_fee.bps,
        service_fee_recipient_pubkey: cfg.service_fee.pubkey.clone(),
        sponsor_enabled: cfg.sponsor.enabled,
        sponsor_available: sponsor_available(cfg),
        sponsor_pubkey: cfg.sponsor.pubkey.clone(),
        public_frame_count: cfg.ifx.public_frames.len(),
        priority_tiers: vec!["low", "medium", "high"],
        default_priority_tier: format!("{:?}", cfg.priority_fee.default_tier).to_lowercase(),
        rpc_url: cfg.solana.rpc_url.clone(),
        address_lookup_table_count: cfg.solana.address_lookup_tables.len(),
        transaction_version: cfg.solana.transaction_version.as_u8(),
        default_mint_a: cfg.trade.mint_a.clone(),
        default_mint_b: cfg.trade.mint_b.clone(),
        default_amount: cfg.trade.amount.clone(),
        raydium_api_timeout_secs: cfg.network.raydium_api_timeout_secs,
        http_proxy_enabled: cfg.network.effective_http_proxy().is_some(),
    })
}

pub async fn quote(
    State(state): State<AppState>,
    Json(body): Json<QuoteRequest>,
) -> Result<Json<QuoteResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!(
        mint_a = %body.mint_a,
        mint_b = %body.mint_b,
        input_amount = %body.input_amount,
        slippage_bps = ?body.slippage_bps,
        build = ?body.build,
        user = ?body.user_pubkey.as_ref().map(|_| "set"),
        "POST /api/quote"
    );
    match quote_inner(&state, &body, body.build.unwrap_or(false)).await {
        Ok(resp) => {
            info!(
                route_kind = %resp.route_kind,
                expected_out = %resp.expected_out_raw,
                direct = resp.direct_available,
                bridge = resp.bridge_available,
                "quote ok"
            );
            Ok(Json(resp))
        }
        Err(e) => {
            warn!(error = %e, "quote failed");
            Err(api_error(e))
        }
    }
}

pub async fn build_tx(
    State(state): State<AppState>,
    Json(body): Json<BuildRequest>,
) -> Result<Json<BuiltTxResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!(
        mint_a = %body.mint_a,
        mint_b = %body.mint_b,
        input_amount = %body.input_amount,
        user = %body.user_pubkey,
        slippage_bps = ?body.slippage_bps,
        use_sponsor = ?body.use_sponsor,
        "POST /api/tx/build"
    );
    match build_inner(&state, &body).await {
        Ok(resp) => {
            info!(
                route_kind = %resp.route_kind,
                tx_bytes = resp.transaction_size_bytes,
                fits_size_gate = resp.fits_size_gate,
                "build ok"
            );
            Ok(Json(resp))
        }
        Err(e) => {
            warn!(error = %e, "build failed");
            Err(api_error(e))
        }
    }
}

pub async fn simulate_tx(
    State(state): State<AppState>,
    Json(body): Json<SimulateRequestBody>,
) -> Result<Json<ifx_raydium_planner::SimulateResult>, (StatusCode, Json<ErrorResponse>)> {
    info!(
        replace_recent_blockhash = body.replace_recent_blockhash,
        tx_b64_len = body.transaction_base64.len(),
        "POST /api/tx/simulate"
    );
    match simulate_transaction(
        &state.config.solana.rpc_url,
        state.commitment,
        &SimulateRequest {
            transaction_base64: body.transaction_base64,
            replace_recent_blockhash: body.replace_recent_blockhash,
        },
    )
    .await
    {
        Ok(resp) => {
            if resp.ok() {
                info!(units = ?resp.units_consumed, "simulate api ok");
            } else {
                warn!(err = ?resp.err, units = ?resp.units_consumed, "simulate api failed");
            }
            Ok(Json(resp))
        }
        Err(e) => {
            warn!(error = %e, "simulate rpc error");
            Err(api_error(e.to_string()))
        }
    }
}

async fn quote_inner(
    state: &AppState,
    body: &QuoteRequest,
    do_build: bool,
) -> Result<QuoteResponse, String> {
    let mint_a = parse_pubkey(&body.mint_a, "mintA")?;
    let mint_b = parse_pubkey(&body.mint_b, "mintB")?;
    let amount_in = parse_human_amount(&mint_a, &body.input_amount)?;
    let slippage_bps = body
        .slippage_bps
        .unwrap_or(state.config.quote.default_slippage_bps);

    let router = make_router(state);

    let service_fee_bps = state.config.service_fee.bps;
    let decision = router
        .resolve(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps)
        .await
        .map_err(|e| e.to_string())?;

    let route = decision.chosen.clone();
    let dec_a = mint_decimals(&mint_a);
    let dec_b = mint_decimals(&mint_b);

    let eligible = sponsor_eligible_for_route(&route);
    let use_sponsor = resolve_use_sponsor(&state.config, body.use_sponsor, eligible);
    let sponsor_ui = sponsor_ui_for_route(&state.config, &route, use_sponsor);

    let built = if do_build {
        let user = body
            .user_pubkey
            .as_ref()
            .ok_or_else(|| "userPubkey required when build=true".to_string())?;
        let user = parse_pubkey(user, "userPubkey")?;
        Some(
            build_from_route(
                state,
                body,
                user,
                amount_in,
                route.clone(),
                decision.message.clone(),
                use_sponsor,
            )
            .await?,
        )
    } else {
        None
    };

    Ok(QuoteResponse {
        route_kind: route_kind_str(route.kind).to_string(),
        route_message: decision.message,
        input_amount_raw: amount_in.to_string(),
        input_amount_ui: format_raw_amount(amount_in, dec_a),
        expected_out_raw: route.expected_out.to_string(),
        expected_out_ui: format_raw_amount(route.expected_out, dec_b),
        mint_a: mint_a.to_string(),
        mint_b: mint_b.to_string(),
        direct_available: decision.direct.is_some(),
        bridge_available: decision.bridge.is_some(),
        sponsor_eligible: eligible,
        sponsor_ui,
        built,
    })
}

async fn build_inner(state: &AppState, body: &BuildRequest) -> Result<BuiltTxResponse, String> {
    let mint_a = parse_pubkey(&body.mint_a, "mintA")?;
    let mint_b = parse_pubkey(&body.mint_b, "mintB")?;
    let user = parse_pubkey(&body.user_pubkey, "userPubkey")?;
    let amount_in = parse_human_amount(&mint_a, &body.input_amount)?;
    let slippage_bps = body
        .slippage_bps
        .unwrap_or(state.config.quote.default_slippage_bps);

    let router = make_router(state);

    let service_fee_bps = state.config.service_fee.bps;
    let decision = router
        .resolve(mint_a, mint_b, amount_in, slippage_bps, service_fee_bps)
        .await
        .map_err(|e| e.to_string())?;

    build_from_route(
        state,
        &QuoteRequest {
            mint_a: body.mint_a.clone(),
            mint_b: body.mint_b.clone(),
            input_amount: body.input_amount.clone(),
            slippage_bps: body.slippage_bps,
            user_pubkey: Some(body.user_pubkey.clone()),
            priority_tier: body.priority_tier.clone(),
            use_sponsor: body.use_sponsor,
            pay_asset: body.pay_asset.clone(),
            receive_asset: body.receive_asset.clone(),
            build: Some(true),
        },
        user,
        amount_in,
        decision.chosen.clone(),
        decision.message,
        resolve_use_sponsor(
            &state.config,
            body.use_sponsor,
            sponsor_eligible_for_route(&decision.chosen),
        ),
    )
    .await
}

async fn build_from_route(
    state: &AppState,
    body: &QuoteRequest,
    user: Pubkey,
    amount_in: u64,
    route: RouteQuote,
    route_message: String,
    use_sponsor: bool,
) -> Result<BuiltTxResponse, String> {
    let config = state.config.clone();
    let mint_a = parse_pubkey(&body.mint_a, "mintA")?;
    let mint_b = parse_pubkey(&body.mint_b, "mintB")?;
    let sol_pay_asset = sol_pay_asset_for_mint(&mint_a, body.pay_asset.as_deref());
    let sol_receive_asset = sol_pay_asset_for_mint(&mint_b, body.receive_asset.as_deref());
    if let Some(pay) = sol_pay_asset {
        crate::token::validate_sol_pay_balance(
            &state.config.solana.rpc_url,
            state.commitment,
            user,
            pay,
            amount_in,
            config.service_fee.bps,
        )
        .await?;
    }

    let use_sponsor = use_sponsor && sponsor_available(&config) && sponsor_eligible_for_route(&route);
    let sponsor_pubkey = if use_sponsor {
        Some(
            config
                .parse_pubkey("sponsor.pubkey", &config.sponsor.pubkey)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    let priority_tier = parse_priority_tier(body.priority_tier.as_deref())
        .unwrap_or(config.priority_fee.default_tier);

    let planner = Planner::new(config.clone());
    let build = planner
        .build(&BuildPlan {
            user,
            route: route.clone(),
            service_fee_bps: config.service_fee.bps,
            fee_recipient: config
                .service_fee_pubkey()
                .map_err(|e| e.to_string())?,
            use_sponsor,
            sponsor_pubkey,
            sponsor_repay_buffer_percent: config.sponsor.repay_buffer_percent,
            priority_tier,
            sol_pay_asset,
            sol_receive_asset,
        })
        .map_err(|e| e.to_string())?;

    let finalized = finalize_build(
        config,
        state.commitment,
        &FinalizeRequest {
            instructions: build.instructions,
            smart_close_ixs: build.smart_close_ixs,
            user,
            frame_used: build.frame_used,
            use_sponsor,
            sponsor_pubkey,
            sponsor_keypair_path: if use_sponsor {
                state.config.sponsor.keypair_path.clone()
            } else {
                None
            },
            priority_tier,
            allow_oversized: false,
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(BuiltTxResponse {
        transaction_base64: finalized.transaction_base64,
        transaction_size_bytes: finalized.transaction_size_bytes,
        fits_size_gate: finalized.fits_size_gate,
        transaction_version: finalized.transaction_version,
        fee_payer: finalized.fee_payer.to_string(),
        recent_blockhash: finalized.recent_blockhash.to_string(),
        last_valid_block_height: finalized.last_valid_block_height,
        partially_signed_by: finalized.partially_signed_by,
        inspection: finalized.inspection,
        route_kind: route_kind_str(route.kind).to_string(),
        route_message,
        input_amount_raw: amount_in.to_string(),
        expected_out_raw: route.expected_out.to_string(),
        use_sponsor,
        smart_close_applied: finalized.smart_close_applied,
    })
}

fn sponsor_available(cfg: &AppConfig) -> bool {
    cfg.sponsor.enabled && cfg.sponsor.keypair_path.is_some()
}

fn sponsor_eligible_for_route(route: &RouteQuote) -> bool {
    match route.kind {
        RouteKind::Bridge => true,
        RouteKind::Direct => route.leg1.output_mint == NATIVE_MINT,
    }
}

fn resolve_use_sponsor(cfg: &AppConfig, requested: Option<bool>, eligible: bool) -> bool {
    if !sponsor_available(cfg) || !eligible {
        return false;
    }
    requested.unwrap_or(cfg.trade.sponsored)
}

fn sponsor_ui_for_route(cfg: &AppConfig, route: &RouteQuote, use_sponsor: bool) -> SponsorUiResponse {
    if !sponsor_available(cfg) {
        return SponsorUiResponse {
            visible: false,
            mode: "hidden".into(),
            enabled: false,
            readonly: true,
            hint: None,
        };
    }

    let eligible = sponsor_eligible_for_route(route);
    if !eligible {
        let hint = match route.kind {
            RouteKind::Bridge => None,
            RouteKind::Direct => Some(
                "Sponsored gas needs SOL proceeds (sell to SOL or use bridge route)".into(),
            ),
        };
        return SponsorUiResponse {
            visible: true,
            mode: "readonly_off".into(),
            enabled: false,
            readonly: true,
            hint,
        };
    }

    let hint = match route.kind {
        RouteKind::Bridge => Some(format!(
            "Bridge route — sponsor pays gas; repay from leg-1 WSOL proceeds · {}",
            short_pk(&cfg.sponsor.pubkey)
        )),
        RouteKind::Direct => Some(format!(
            "Direct SOL output — sponsor pays gas; repay from swap proceeds · {}",
            short_pk(&cfg.sponsor.pubkey)
        )),
    };

    SponsorUiResponse {
        visible: true,
        mode: "optional".into(),
        enabled: use_sponsor,
        readonly: false,
        hint,
    }
}

fn short_pk(s: &str) -> String {
    if s.len() <= 10 {
        return s.to_string();
    }
    format!("{}…{}", &s[..4], &s[s.len() - 4..])
}

fn route_kind_str(kind: RouteKind) -> &'static str {
    match kind {
        RouteKind::Direct => "direct",
        RouteKind::Bridge => "bridge",
    }
}

fn parse_pubkey(s: &str, field: &str) -> Result<Pubkey, String> {
    s.parse()
        .map_err(|_| format!("invalid {field}: {s}"))
}

fn parse_priority_tier(s: Option<&str>) -> Option<PriorityTier> {
    match s? {
        "low" => Some(PriorityTier::Low),
        "medium" => Some(PriorityTier::Medium),
        "high" => Some(PriorityTier::High),
        _ => None,
    }
}

fn api_error(msg: impl ToString) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

pub async fn token_info(
    State(state): State<AppState>,
    Json(body): Json<crate::token::TokenInfoRequest>,
) -> Result<Json<crate::token::TokenInfoResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!(mint = %body.mint, user = ?body.user_pubkey, "POST /api/token/info");
    let mint = crate::token::parse_mint_pubkey(&body.mint).map_err(|e| {
        warn!(error = %e, "token info: invalid mint");
        api_error(e)
    })?;
    let user = crate::token::parse_user_pubkey(body.user_pubkey.as_ref()).map_err(|e| {
        warn!(error = %e, "token info: invalid user");
        api_error(e)
    })?;
    match crate::token::fetch_token_info(
        &state.config.solana.rpc_url,
        state.commitment,
        mint,
        user,
    )
    .await
    {
        Ok(resp) => {
            info!(mint = %body.mint, decimals = resp.decimals, "token info ok");
            Ok(Json(resp))
        }
        Err(e) => {
            warn!(mint = %body.mint, error = %e, "token info failed");
            Err(api_error(e))
        }
    }
}

fn make_router(state: &AppState) -> Router {
    let timeout_secs = state.config.network.raydium_api_timeout_secs.max(1);
    let api = ApiClient::with_options(
        ifx_raydium::constants::API_V3_BASE,
        HttpClientOptions {
            connect_timeout: std::time::Duration::from_secs(5),
            timeout: std::time::Duration::from_secs(timeout_secs),
            proxy: state.config.network.effective_http_proxy(),
        },
    )
    .unwrap_or_else(|e| {
        warn!(error = %e, "invalid Raydium API client config, falling back without explicit proxy");
        ApiClient::with_timeout(
            ifx_raydium::constants::API_V3_BASE,
            std::time::Duration::from_secs(timeout_secs),
        )
    });
    Router::new(
        api,
        RpcPoolLoader::new(&state.config.solana.rpc_url, state.commitment),
    )
    .with_api_timeout(timeout_secs)
}
