//! Keypair loading and partial versioned-transaction signing (sponsor co-sign).

use crate::tx_v1::{is_v1_wire, sign_v1_with_keypairs};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignError {
    #[error("io: {0}")]
    Io(String),
    #[error("json: {0}")]
    Json(String),
    #[error("keypair: {0}")]
    Keypair(String),
    #[error("serialize: {0}")]
    Serialize(String),
    #[error("{0}")]
    Tx(String),
}

pub fn read_keypair(path: &str) -> Result<Keypair, SignError> {
    let data = std::fs::read_to_string(path).map_err(|e| SignError::Io(e.to_string()))?;
    let bytes: Vec<u8> = serde_json::from_str(&data).map_err(|e| SignError::Json(e.to_string()))?;
    Keypair::try_from(bytes.as_slice()).map_err(|e| SignError::Keypair(e.to_string()))
}

pub fn sign_with_keypairs(
    serialized: &[u8],
    keypairs: &[&Keypair],
) -> Result<Vec<u8>, SignError> {
    if is_v1_wire(serialized) {
        return sign_v1_with_keypairs(serialized, keypairs)
            .map_err(|e| SignError::Tx(e.to_string()));
    }

    let mut tx: VersionedTransaction =
        bincode::deserialize(serialized).map_err(|e| SignError::Serialize(e.to_string()))?;
    let message_bytes = tx.message.serialize();
    let keys = tx.message.static_account_keys();

    for kp in keypairs {
        let pubkey = kp.pubkey();
        let idx = keys
            .iter()
            .position(|k| *k == pubkey)
            .ok_or_else(|| SignError::Tx(format!("signer {pubkey} not in transaction accounts")))?;
        if idx >= tx.signatures.len() {
            return Err(SignError::Tx(format!(
                "signature slot {idx} missing (have {})",
                tx.signatures.len()
            )));
        }
        tx.signatures[idx] = kp.sign_message(&message_bytes);
    }

    bincode::serialize(&tx).map_err(|e| SignError::Serialize(e.to_string()))
}
