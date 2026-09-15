//! Compile instructions into a versioned transaction (v0+ALT or v1) with size gate.

use crate::alt::AltCache;
use crate::inspect::{
    inspect_instructions, inspect_versioned_transaction, InspectOpts, TransactionConfigInspection,
    TxInspection,
};
use crate::sign::{read_keypair, SignError};
use crate::sponsor::priority_fee_lamports_for_tier;
use crate::tx_size::{
    assert_transaction_size_versioned, fits_transaction_size_versioned, max_transaction_size,
};
use crate::tx_v1::{
    compile_v1_transaction, sign_v1_with_keypairs, V1ResourceConfig,
    DEFAULT_LOADED_ACCOUNTS_DATA_SIZE_LIMIT,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ifx_raydium_config::{AppConfig, PriorityTier};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_message::v0::Message as V0Message;
use solana_message::{AddressLookupTableAccount, VersionedMessage};
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum FinalizeError {
    #[error("alt: {0}")]
    Alt(#[from] crate::alt::AltError),
    #[error("rpc: {0}")]
    Rpc(#[from] solana_client::client_error::ClientError),
    #[error("compile: {0}")]
    Compile(String),
    #[error("serialize: {0}")]
    Serialize(String),
    #[error("config: {0}")]
    Config(#[from] ifx_raydium_config::ConfigError),
    #[error("sign: {0}")]
    Sign(#[from] SignError),
    #[error("v1: {0}")]
    V1(#[from] crate::tx_v1::TxV1Error),
    #[error("{0}")]
    Tx(String),
}

#[derive(Debug, Clone)]
pub struct FinalizeRequest {
    pub instructions: Vec<Instruction>,
    /// Appended after core trade ixs when compile + size gate allow.
    pub smart_close_ixs: Vec<Instruction>,
    pub user: Pubkey,
    pub frame_used: Option<Pubkey>,
    pub use_sponsor: bool,
    pub sponsor_pubkey: Option<Pubkey>,
    pub sponsor_keypair_path: Option<String>,
    pub priority_tier: PriorityTier,
    /// When true, compile and return size even if tx exceeds the version size gate.
    pub allow_oversized: bool,
}

#[derive(Debug, Clone)]
pub struct FinalizeResult {
    pub transaction_base64: String,
    pub transaction_size_bytes: usize,
    pub fits_size_gate: bool,
    pub smart_close_applied: bool,
    pub fee_payer: Pubkey,
    pub recent_blockhash: Hash,
    pub last_valid_block_height: u64,
    pub partially_signed_by: Option<String>,
    pub inspection: TxInspection,
    pub transaction_version: u8,
}

pub struct Finalizer {
    config: Arc<AppConfig>,
    rpc: RpcClient,
    alt_cache: AltCache,
}

impl Finalizer {
    pub fn new(config: Arc<AppConfig>, commitment: CommitmentConfig) -> Self {
        let rpc_url = config.solana.rpc_url.clone();
        Self {
            rpc: RpcClient::new_with_commitment(rpc_url, commitment),
            config,
            alt_cache: AltCache::default(),
        }
    }

    pub fn rpc(&self) -> &RpcClient {
        &self.rpc
    }

    pub async fn latest_blockhash(&self) -> Result<(Hash, u64), FinalizeError> {
        let resp = self
            .rpc
            .get_latest_blockhash_with_commitment(self.rpc.commitment())
            .await?;
        Ok((resp.0, resp.1))
    }

    pub async fn finalize(&mut self, req: &FinalizeRequest) -> Result<FinalizeResult, FinalizeError> {
        self.finalize_inner(req).await
    }

    async fn finalize_inner(&mut self, req: &FinalizeRequest) -> Result<FinalizeResult, FinalizeError> {
        if self.config.uses_tx_v1() {
            self.finalize_v1(req).await
        } else {
            self.finalize_v0(req).await
        }
    }

    async fn finalize_v0(&mut self, req: &FinalizeRequest) -> Result<FinalizeResult, FinalizeError> {
        let ifx_program_id = self.config.ifx_program_id()?;
        let (recent_blockhash, last_valid_block_height) = self.latest_blockhash().await?;

        let sponsor_active = req.use_sponsor && req.sponsor_pubkey.is_some();
        let fee_payer = if sponsor_active {
            req.sponsor_pubkey.unwrap()
        } else {
            req.user
        };

        let lookup_tables = self
            .alt_cache
            .get_tables(&self.rpc, &self.config.solana.address_lookup_tables)
            .await?;

        let (instructions, smart_close_applied) =
            select_instructions_with_smart_close_v0(req, &fee_payer, recent_blockhash, &lookup_tables)?;

        let (tx, serialized) =
            compile_versioned_tx_v0(&instructions, &fee_payer, recent_blockhash, &lookup_tables)?;

        if !req.allow_oversized {
            assert_transaction_size_versioned(&serialized, 0).map_err(|e| {
                warn!(tx_bytes = serialized.len(), error = %e, "tx exceeds v0 size gate");
                FinalizeError::Tx(e)
            })?;
        }

        let inspection = inspect_versioned_transaction(
            &tx,
            &lookup_tables,
            &InspectOpts {
                ifx_program_id,
                frame_used: req.frame_used,
                fee_payer: Some(fee_payer),
                smart_close_applied: Some(smart_close_applied),
                transaction_size_bytes: Some(serialized.len()),
                address_lookup_table_addresses: self.config.solana.address_lookup_tables.clone(),
                transaction_config: None,
            },
        );

        info!(
            tx_bytes = serialized.len(),
            fits_size_gate = fits_transaction_size_versioned(&serialized, 0),
            ix_count = instructions.len(),
            smart_close_applied,
            fee_payer = %fee_payer,
            sponsor_active,
            alt_count = lookup_tables.len(),
            transaction_version = 0,
            "tx finalized"
        );

        Ok(FinalizeResult {
            transaction_base64: STANDARD.encode(&serialized),
            transaction_size_bytes: serialized.len(),
            fits_size_gate: fits_transaction_size_versioned(&serialized, 0),
            smart_close_applied,
            fee_payer,
            recent_blockhash,
            last_valid_block_height,
            partially_signed_by: None,
            inspection,
            transaction_version: 0,
        })
    }

    async fn finalize_v1(&mut self, req: &FinalizeRequest) -> Result<FinalizeResult, FinalizeError> {
        let ifx_program_id = self.config.ifx_program_id()?;
        let (recent_blockhash, last_valid_block_height) = self.latest_blockhash().await?;

        let sponsor_active = req.use_sponsor && req.sponsor_pubkey.is_some();
        let fee_payer = if sponsor_active {
            req.sponsor_pubkey.unwrap()
        } else {
            req.user
        };

        let resource = v1_resource_config(self.config.as_ref(), req.priority_tier);
        let (instructions, smart_close_applied) =
            select_instructions_with_smart_close_v1(req, &fee_payer, recent_blockhash, resource)?;

        let compiled =
            compile_v1_transaction(&fee_payer, &instructions, recent_blockhash, resource)?;
        let serialized = compiled.serialized.clone();

        if !req.allow_oversized {
            assert_transaction_size_versioned(&serialized, 1).map_err(|e| {
                warn!(tx_bytes = serialized.len(), error = %e, "tx exceeds v1 size gate");
                FinalizeError::Tx(e)
            })?;
        }

        let inspection = inspect_instructions(
            &instructions,
            &InspectOpts {
                ifx_program_id,
                frame_used: req.frame_used,
                fee_payer: Some(fee_payer),
                smart_close_applied: Some(smart_close_applied),
                transaction_size_bytes: Some(serialized.len()),
                address_lookup_table_addresses: vec![],
                transaction_config: Some(TransactionConfigInspection {
                    compute_unit_limit: resource.compute_unit_limit,
                    loaded_accounts_data_size_limit: resource.loaded_accounts_data_size_limit,
                    priority_fee_lamports: resource.priority_fee_lamports.to_string(),
                }),
            },
        );

        info!(
            tx_bytes = serialized.len(),
            max = max_transaction_size(1),
            fits_size_gate = fits_transaction_size_versioned(&serialized, 1),
            ix_count = instructions.len(),
            smart_close_applied,
            fee_payer = %fee_payer,
            sponsor_active,
            transaction_version = 1,
            "tx finalized"
        );

        Ok(FinalizeResult {
            transaction_base64: STANDARD.encode(&serialized),
            transaction_size_bytes: serialized.len(),
            fits_size_gate: fits_transaction_size_versioned(&serialized, 1),
            smart_close_applied,
            fee_payer,
            recent_blockhash,
            last_valid_block_height,
            partially_signed_by: None,
            inspection,
            transaction_version: 1,
        })
    }
}

fn v1_resource_config(config: &AppConfig, tier: PriorityTier) -> V1ResourceConfig {
    let t = config.priority_fee.tier(tier);
    V1ResourceConfig {
        compute_unit_limit: t.compute_unit_limit,
        loaded_accounts_data_size_limit: DEFAULT_LOADED_ACCOUNTS_DATA_SIZE_LIMIT,
        priority_fee_lamports: priority_fee_lamports_for_tier(config, tier),
    }
}

fn select_instructions_with_smart_close_v0(
    req: &FinalizeRequest,
    fee_payer: &Pubkey,
    recent_blockhash: Hash,
    lookup_tables: &[AddressLookupTableAccount],
) -> Result<(Vec<Instruction>, bool), FinalizeError> {
    if req.smart_close_ixs.is_empty() {
        return Ok((req.instructions.clone(), false));
    }

    let with_close: Vec<Instruction> = req
        .instructions
        .iter()
        .chain(req.smart_close_ixs.iter())
        .cloned()
        .collect();

    match compile_versioned_tx_v0(&with_close, fee_payer, recent_blockhash, lookup_tables) {
        Ok((_, serialized)) if fits_transaction_size_versioned(&serialized, 0) => {
            Ok((with_close, true))
        }
        _ => Ok((req.instructions.clone(), false)),
    }
}

fn select_instructions_with_smart_close_v1(
    req: &FinalizeRequest,
    fee_payer: &Pubkey,
    recent_blockhash: Hash,
    resource: V1ResourceConfig,
) -> Result<(Vec<Instruction>, bool), FinalizeError> {
    if req.smart_close_ixs.is_empty() {
        return Ok((req.instructions.clone(), false));
    }

    let with_close: Vec<Instruction> = req
        .instructions
        .iter()
        .chain(req.smart_close_ixs.iter())
        .cloned()
        .collect();

    match compile_v1_transaction(fee_payer, &with_close, recent_blockhash, resource) {
        Ok(compiled) if fits_transaction_size_versioned(&compiled.serialized, 1) => {
            Ok((with_close, true))
        }
        _ => Ok((req.instructions.clone(), false)),
    }
}

fn compile_versioned_tx_v0(
    instructions: &[Instruction],
    fee_payer: &Pubkey,
    recent_blockhash: Hash,
    lookup_tables: &[AddressLookupTableAccount],
) -> Result<(VersionedTransaction, Vec<u8>), FinalizeError> {
    let v0 = V0Message::try_compile(fee_payer, instructions, lookup_tables, recent_blockhash)
        .map_err(|e| FinalizeError::Compile(format!("{e}")))?;

    let num_sigs = v0.header.num_required_signatures as usize;
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default(); num_sigs.max(1)],
        message: VersionedMessage::V0(v0),
    };

    let serialized = bincode::serialize(&tx).map_err(|e| FinalizeError::Serialize(e.to_string()))?;

    Ok((tx, serialized))
}

pub fn partial_sign_sponsor(
    serialized: &[u8],
    sponsor_keypair_path: &str,
    transaction_version: u8,
) -> Result<(Vec<u8>, String), FinalizeError> {
    let sponsor = read_keypair(sponsor_keypair_path)?;
    let pubkey = sponsor.pubkey();
    let signed = if transaction_version == 1 {
        sign_v1_with_keypairs(serialized, &[&sponsor])?
    } else {
        crate::sign::sign_with_keypairs(serialized, &[&sponsor])?
    };
    Ok((signed, pubkey.to_string()))
}

pub async fn finalize_build(
    config: Arc<AppConfig>,
    commitment: CommitmentConfig,
    req: &FinalizeRequest,
) -> Result<FinalizeResult, FinalizeError> {
    let mut finalizer = Finalizer::new(config, commitment);
    let mut result = finalizer.finalize_inner(req).await?;

    if req.use_sponsor {
        if let Some(path) = &req.sponsor_keypair_path {
            let (signed, pubkey) = partial_sign_sponsor(
                &STANDARD
                    .decode(&result.transaction_base64)
                    .map_err(|e| FinalizeError::Tx(e.to_string()))?,
                path,
                result.transaction_version,
            )?;
            if !req.allow_oversized {
                assert_transaction_size_versioned(&signed, result.transaction_version)
                    .map_err(FinalizeError::Tx)?;
            }
            result.transaction_base64 = STANDARD.encode(&signed);
            result.transaction_size_bytes = signed.len();
            result.fits_size_gate =
                fits_transaction_size_versioned(&signed, result.transaction_version);
            result.partially_signed_by = Some(pubkey);
            result.inspection.transaction_size_bytes = Some(signed.len());
        }
    }

    Ok(result)
}
