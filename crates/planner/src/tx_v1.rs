//! Solana v1 (SIMD-0385) transaction compile / serialize / partial-sign.
//!
//! Kept on the solana-sdk 3.x stack (for ifx-sdk compatibility) by compiling
//! account layout via empty-ALT v0 `try_compile`, then emitting the v1 wire
//! format (message config instead of ComputeBudget ixs; signatures at the tail).

use crate::tx_size::MAX_V1_TRANSACTION_SIZE;
use solana_message::v0::Message as V0Message;
use solana_message::{compiled_instruction::CompiledInstruction, MessageHeader};
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature};
use solana_sdk::signer::Signer;
use thiserror::Error;

/// SIMD-0296 / SIMD-0385: v1 serialized transaction ceiling.
pub const MAX_V1_TX_SIZE: usize = MAX_V1_TRANSACTION_SIZE;

/// v1 unset loaded-accounts cap is 0; 64 MiB matches the legacy/v0 default.
pub const DEFAULT_LOADED_ACCOUNTS_DATA_SIZE_LIMIT: u32 = 64 * 1024 * 1024;

/// Wire prefix for a v1 message / transaction (`MESSAGE_VERSION_PREFIX | 1`).
pub const V1_PREFIX: u8 = 0x81;

pub const SIGNATURE_SIZE: usize = 64;

const MAX_ADDRESSES: usize = 64;
const MAX_INSTRUCTIONS: usize = 64;

const PRIORITY_FEE_MASK: u32 = 0b11;
const COMPUTE_UNIT_LIMIT_MASK: u32 = 0b100;
const LOADED_ACCOUNTS_DATA_SIZE_MASK: u32 = 0b1000;
const HEAP_SIZE_MASK: u32 = 0b1_0000;

#[derive(Debug, Error)]
pub enum TxV1Error {
    #[error("compile: {0}")]
    Compile(String),
    #[error("serialize: {0}")]
    Serialize(String),
    #[error("sign: {0}")]
    Sign(String),
    #[error("{0}")]
    Tx(String),
}

#[derive(Debug, Clone, Copy)]
pub struct V1ResourceConfig {
    pub compute_unit_limit: u32,
    pub loaded_accounts_data_size_limit: u32,
    pub priority_fee_lamports: u64,
}

impl V1ResourceConfig {
    pub fn config_mask(self) -> u32 {
        let mut mask = 0u32;
        mask |= PRIORITY_FEE_MASK;
        mask |= COMPUTE_UNIT_LIMIT_MASK;
        mask |= LOADED_ACCOUNTS_DATA_SIZE_MASK;
        // heap_size left unset → runtime default 32 KiB
        let _ = HEAP_SIZE_MASK;
        mask
    }

    pub fn config_values_size(self) -> usize {
        8 + 4 + 4 // priority_fee u64 + cu_limit u32 + loaded_accounts u32
    }
}

#[derive(Debug, Clone)]
pub struct CompiledV1Tx {
    pub serialized: Vec<u8>,
    pub message_bytes: Vec<u8>,
    pub num_required_signatures: u8,
    pub account_keys: Vec<Pubkey>,
    pub config: V1ResourceConfig,
}

pub fn is_v1_wire(bytes: &[u8]) -> bool {
    bytes.first() == Some(&V1_PREFIX)
}

/// Compile instructions into a partially-unsigned v1 transaction.
///
/// Does **not** use ALTs — all accounts are inlined (max 64).
pub fn compile_v1_transaction(
    fee_payer: &Pubkey,
    instructions: &[Instruction],
    recent_blockhash: Hash,
    config: V1ResourceConfig,
) -> Result<CompiledV1Tx, TxV1Error> {
    if instructions.len() > MAX_INSTRUCTIONS {
        return Err(TxV1Error::Compile(format!(
            "too many instructions: {} (max {MAX_INSTRUCTIONS})",
            instructions.len()
        )));
    }

    // Reuse v0 key compilation with empty ALTs for identical account ordering.
    let v0 = V0Message::try_compile(fee_payer, instructions, &[], recent_blockhash)
        .map_err(|e| TxV1Error::Compile(format!("{e}")))?;

    if !v0.address_table_lookups.is_empty() {
        return Err(TxV1Error::Compile(
            "v1 transactions do not support address lookup tables".into(),
        ));
    }
    if v0.account_keys.len() > MAX_ADDRESSES {
        return Err(TxV1Error::Compile(format!(
            "too many account keys: {} (v1 max {MAX_ADDRESSES})",
            v0.account_keys.len()
        )));
    }

    let message_bytes = serialize_v1_message(
        &v0.header,
        &v0.account_keys,
        recent_blockhash,
        &v0.instructions,
        config,
    )?;

    let num_sigs = v0.header.num_required_signatures as usize;
    let mut serialized = message_bytes.clone();
    serialized.resize(message_bytes.len() + num_sigs * SIGNATURE_SIZE, 0);

    Ok(CompiledV1Tx {
        serialized,
        message_bytes,
        num_required_signatures: v0.header.num_required_signatures,
        account_keys: v0.account_keys,
        config,
    })
}

fn serialize_v1_message(
    header: &MessageHeader,
    account_keys: &[Pubkey],
    recent_blockhash: Hash,
    instructions: &[CompiledInstruction],
    config: V1ResourceConfig,
) -> Result<Vec<u8>, TxV1Error> {
    let mut out = Vec::with_capacity(estimate_message_size(
        account_keys.len(),
        instructions,
        config,
    ));

    out.push(V1_PREFIX);
    out.push(header.num_required_signatures);
    out.push(header.num_readonly_signed_accounts);
    out.push(header.num_readonly_unsigned_accounts);
    out.extend_from_slice(&config.config_mask().to_le_bytes());
    out.extend_from_slice(recent_blockhash.as_ref());
    out.push(instructions.len() as u8);
    out.push(account_keys.len() as u8);

    for key in account_keys {
        out.extend_from_slice(key.as_ref());
    }

    out.extend_from_slice(&config.priority_fee_lamports.to_le_bytes());
    out.extend_from_slice(&config.compute_unit_limit.to_le_bytes());
    out.extend_from_slice(&config.loaded_accounts_data_size_limit.to_le_bytes());

    for ix in instructions {
        if ix.accounts.len() > u8::MAX as usize {
            return Err(TxV1Error::Compile(
                "instruction accounts too large for v1 wire".into(),
            ));
        }
        if ix.data.len() > u16::MAX as usize {
            return Err(TxV1Error::Compile(
                "instruction data too large for v1 wire".into(),
            ));
        }
        out.push(ix.program_id_index);
        out.push(ix.accounts.len() as u8);
        out.extend_from_slice(&(ix.data.len() as u16).to_le_bytes());
    }

    for ix in instructions {
        out.extend_from_slice(&ix.accounts);
        out.extend_from_slice(&ix.data);
    }

    Ok(out)
}

fn estimate_message_size(
    num_keys: usize,
    instructions: &[CompiledInstruction],
    config: V1ResourceConfig,
) -> usize {
    1 + 3 + 4 + 32 + 2 + num_keys * 32 + config.config_values_size()
        + instructions.len() * 4
        + instructions
            .iter()
            .map(|ix| ix.accounts.len() + ix.data.len())
            .sum::<usize>()
}

/// Split a serialized v1 transaction into (message_bytes, signatures).
pub fn split_v1_transaction(bytes: &[u8]) -> Result<(&[u8], &[u8]), TxV1Error> {
    if !is_v1_wire(bytes) {
        return Err(TxV1Error::Serialize(
            "not a v1 transaction (missing 0x81 prefix)".into(),
        ));
    }
    let message_len = measure_v1_message_len(bytes)?;
    if bytes.len() < message_len {
        return Err(TxV1Error::Serialize("truncated v1 message".into()));
    }
    let num_sigs = bytes[1] as usize; // num_required_signatures
    let expected = message_len + num_sigs * SIGNATURE_SIZE;
    if bytes.len() != expected {
        return Err(TxV1Error::Serialize(format!(
            "v1 tx length mismatch: got {} expected {expected}",
            bytes.len()
        )));
    }
    Ok((&bytes[..message_len], &bytes[message_len..]))
}

fn measure_v1_message_len(bytes: &[u8]) -> Result<usize, TxV1Error> {
    // Fixed prelude: prefix(1) + header(3) + mask(4) + hash(32) + n_ix(1) + n_keys(1) = 42
    if bytes.len() < 42 {
        return Err(TxV1Error::Serialize("v1 message too short".into()));
    }
    let mask = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let num_instructions = bytes[40] as usize;
    let num_addresses = bytes[41] as usize;

    let mut offset = 42 + num_addresses * 32;
    if mask & PRIORITY_FEE_MASK == PRIORITY_FEE_MASK {
        offset += 8;
    } else if mask & PRIORITY_FEE_MASK != 0 {
        return Err(TxV1Error::Serialize(
            "invalid partial priority-fee mask bits".into(),
        ));
    }
    if mask & COMPUTE_UNIT_LIMIT_MASK != 0 {
        offset += 4;
    }
    if mask & LOADED_ACCOUNTS_DATA_SIZE_MASK != 0 {
        offset += 4;
    }
    if mask & HEAP_SIZE_MASK != 0 {
        offset += 4;
    }

    if bytes.len() < offset + num_instructions * 4 {
        return Err(TxV1Error::Serialize(
            "truncated v1 instruction headers".into(),
        ));
    }

    let headers_start = offset;
    offset += num_instructions * 4;

    for i in 0..num_instructions {
        let h = headers_start + i * 4;
        let accounts_len = bytes[h + 1] as usize;
        let data_len = u16::from_le_bytes(bytes[h + 2..h + 4].try_into().unwrap()) as usize;
        offset += accounts_len + data_len;
    }

    Ok(offset)
}

/// Partially sign a serialized v1 transaction with the given keypairs.
pub fn sign_v1_with_keypairs(serialized: &[u8], keypairs: &[&Keypair]) -> Result<Vec<u8>, TxV1Error> {
    let (message_bytes, sig_bytes) = split_v1_transaction(serialized)?;
    let num_sigs = message_bytes[1] as usize;
    let num_keys = message_bytes[41] as usize;
    let keys_start = 42;
    let keys_end = keys_start + num_keys * 32;
    if message_bytes.len() < keys_end {
        return Err(TxV1Error::Sign("truncated account keys".into()));
    }

    let mut signatures: Vec<Signature> = sig_bytes
        .chunks_exact(SIGNATURE_SIZE)
        .map(|c| {
            Signature::try_from(c).unwrap_or_else(|_| Signature::default())
        })
        .collect();
    if signatures.len() != num_sigs {
        return Err(TxV1Error::Sign(format!(
            "signature count {} != required {num_sigs}",
            signatures.len()
        )));
    }

    for kp in keypairs {
        let pubkey = kp.pubkey();
        let idx = (0..num_keys)
            .find(|&i| {
                let start = keys_start + i * 32;
                &message_bytes[start..start + 32] == pubkey.as_ref()
            })
            .ok_or_else(|| {
                TxV1Error::Sign(format!("signer {pubkey} not in transaction accounts"))
            })?;
        if idx >= num_sigs {
            return Err(TxV1Error::Sign(format!(
                "signer {pubkey} at index {idx} is not a required signature slot"
            )));
        }
        signatures[idx] = kp.sign_message(message_bytes);
    }

    let mut out = message_bytes.to_vec();
    for sig in &signatures {
        out.extend_from_slice(sig.as_ref());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_system_interface::instruction as system_instruction;

    #[test]
    fn compiles_and_roundtrips_size_under_limit() {
        let payer = Pubkey::new_unique();
        let dest = Pubkey::new_unique();
        let ix = system_instruction::transfer(&payer, &dest, 42);
        let compiled = compile_v1_transaction(
            &payer,
            &[ix],
            Hash::new_unique(),
            V1ResourceConfig {
                compute_unit_limit: 200_000,
                loaded_accounts_data_size_limit: DEFAULT_LOADED_ACCOUNTS_DATA_SIZE_LIMIT,
                priority_fee_lamports: 5_000,
            },
        )
        .unwrap();
        assert_eq!(compiled.serialized[0], V1_PREFIX);
        assert!(compiled.serialized.len() <= MAX_V1_TX_SIZE);
        let (msg, sigs) = split_v1_transaction(&compiled.serialized).unwrap();
        assert_eq!(msg, compiled.message_bytes.as_slice());
        assert_eq!(sigs.len(), compiled.num_required_signatures as usize * SIGNATURE_SIZE);
    }
}
