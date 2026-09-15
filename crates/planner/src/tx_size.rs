//! Transaction size helpers (versioned 1232B / 4096B gates).

/// Legacy / v0 packet limit.
pub const MAX_V0_TRANSACTION_SIZE: usize = 1232;

/// SIMD-0296 / SIMD-0385 v1 serialized transaction ceiling.
pub const MAX_V1_TRANSACTION_SIZE: usize = 4096;

/// Back-compat alias for the v0 gate.
pub const MAX_TX_SIZE: usize = MAX_V0_TRANSACTION_SIZE;

pub fn max_transaction_size(version: u8) -> usize {
    if version == 1 {
        MAX_V1_TRANSACTION_SIZE
    } else {
        MAX_V0_TRANSACTION_SIZE
    }
}

pub fn tx_too_large_hint(version: u8) -> &'static str {
    if version == 1 {
        "Transaction exceeds Solana's 4096-byte v1 limit — reduce instructions or disable sponsor."
    } else {
        "Transaction exceeds Solana's 1232-byte limit — configure solana.address_lookup_tables, set transaction_version = 1, or disable sponsor."
    }
}

pub const TX_TOO_LARGE_HINT: &str =
    "Transaction exceeds Solana's 1232-byte limit — configure solana.address_lookup_tables, set transaction_version = 1, or disable sponsor.";

pub fn serialized_size(bytes: &[u8]) -> usize {
    bytes.len()
}

pub fn fits_transaction_size(bytes: &[u8]) -> bool {
    fits_transaction_size_versioned(bytes, 0)
}

pub fn fits_transaction_size_versioned(bytes: &[u8], version: u8) -> bool {
    bytes.len() <= max_transaction_size(version)
}

pub fn assert_transaction_size(bytes: &[u8]) -> Result<(), String> {
    assert_transaction_size_versioned(bytes, 0)
}

pub fn assert_transaction_size_versioned(bytes: &[u8], version: u8) -> Result<(), String> {
    let max = max_transaction_size(version);
    if bytes.len() <= max {
        Ok(())
    } else {
        Err(format!(
            "transaction too large: {} bytes (max {max}). {}",
            bytes.len(),
            tx_too_large_hint(version)
        ))
    }
}
