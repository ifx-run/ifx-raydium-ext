//! RPC transaction simulation (dry-run).

use crate::tx_v1::is_v1_wire;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use serde_json::json;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_client::rpc_request::RpcRequest;
use solana_client::rpc_response::RpcSimulateTransactionResult;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::transaction::VersionedTransaction;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum SimulateError {
    #[error("rpc: {0}")]
    Rpc(#[from] solana_client::client_error::ClientError),
    #[error("decode: {0}")]
    Decode(String),
}

#[derive(Debug, Clone)]
pub struct SimulateRequest {
    pub transaction_base64: String,
    pub replace_recent_blockhash: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateResult {
    pub err: Option<String>,
    pub logs: Option<Vec<String>>,
    pub units_consumed: Option<u64>,
}

impl SimulateResult {
    pub fn ok(&self) -> bool {
        self.err.is_none()
    }
}

pub async fn simulate_transaction(
    rpc_url: &str,
    commitment: CommitmentConfig,
    req: &SimulateRequest,
) -> Result<SimulateResult, SimulateError> {
    let rpc = RpcClient::new_with_commitment(rpc_url.to_string(), commitment);
    let bytes = STANDARD
        .decode(&req.transaction_base64)
        .map_err(|e| SimulateError::Decode(e.to_string()))?;

    debug!(
        tx_bytes = bytes.len(),
        replace_recent_blockhash = req.replace_recent_blockhash,
        is_v1 = is_v1_wire(&bytes),
        "simulate transaction start"
    );

    let result = if is_v1_wire(&bytes) {
        // solana-client 3.x cannot deserialize v1 wire into VersionedTransaction;
        // submit the base64 payload directly over JSON-RPC.
        simulate_raw_base64(&rpc, &req.transaction_base64, req.replace_recent_blockhash).await?
    } else {
        let tx: VersionedTransaction =
            bincode::deserialize(&bytes).map_err(|e| SimulateError::Decode(e.to_string()))?;
        let resp = rpc
            .simulate_transaction_with_config(
                &tx,
                RpcSimulateTransactionConfig {
                    sig_verify: false,
                    replace_recent_blockhash: req.replace_recent_blockhash,
                    commitment: Some(commitment),
                    ..RpcSimulateTransactionConfig::default()
                },
            )
            .await?;
        SimulateResult {
            err: resp.value.err.map(|e| format!("{e:?}")),
            logs: resp.value.logs,
            units_consumed: resp.value.units_consumed,
        }
    };

    if result.ok() {
        info!(
            units_consumed = ?result.units_consumed,
            log_lines = result.logs.as_ref().map(|l| l.len()).unwrap_or(0),
            "simulate ok"
        );
    } else {
        warn!(
            err = result.err.as_deref().unwrap_or("unknown"),
            units_consumed = ?result.units_consumed,
            log_lines = result.logs.as_ref().map(|l| l.len()).unwrap_or(0),
            "simulate failed"
        );
        if let Some(logs) = &result.logs {
            for line in logs.iter().rev().take(8).rev() {
                debug!(log = %line, "simulate log");
            }
        }
    }

    Ok(result)
}

async fn simulate_raw_base64(
    rpc: &RpcClient,
    transaction_base64: &str,
    replace_recent_blockhash: bool,
) -> Result<SimulateResult, SimulateError> {
    let config = json!({
        "encoding": "base64",
        "sigVerify": false,
        "replaceRecentBlockhash": replace_recent_blockhash,
    });
    let resp: solana_client::rpc_response::Response<RpcSimulateTransactionResult> = rpc
        .send(
            RpcRequest::SimulateTransaction,
            json!([transaction_base64, config]),
        )
        .await?;

    Ok(SimulateResult {
        err: resp.value.err.map(|e| format!("{e:?}")),
        logs: resp.value.logs,
        units_consumed: resp.value.units_consumed,
    })
}
