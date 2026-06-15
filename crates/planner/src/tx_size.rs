//! Transaction size helpers (1232B gate).

pub const MAX_TX_SIZE: usize = 1232;

pub const TX_TOO_LARGE_HINT: &str =
    "Transaction exceeds Solana's 1232-byte limit — configure solana.address_lookup_tables, or disable sponsor.";

pub fn serialized_size(bytes: &[u8]) -> usize {
    bytes.len()
}

pub fn fits_transaction_size(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_TX_SIZE
}

pub fn assert_transaction_size(bytes: &[u8]) -> Result<(), String> {
    if fits_transaction_size(bytes) {
        Ok(())
    } else {
        Err(format!(
            "transaction too large: {} bytes (max {MAX_TX_SIZE})",
            bytes.len()
        ))
    }
}
