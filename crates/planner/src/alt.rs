//! Address Lookup Table fetch + simple TTL cache.

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_message::AddressLookupTableAccount;
use solana_sdk::pubkey::Pubkey;
use std::time::{Duration, Instant};
use thiserror::Error;

const LOOKUP_TABLE_META_SIZE: usize = 56;

#[derive(Debug, Error)]
pub enum AltError {
    #[error("rpc: {0}")]
    Rpc(#[from] solana_client::client_error::ClientError),
    #[error("alt not found: {0}")]
    NotFound(Pubkey),
    #[error("invalid alt data for {0}: {1}")]
    InvalidData(Pubkey, String),
}

struct CacheEntry {
    at: Instant,
    tables: Vec<AddressLookupTableAccount>,
}

pub struct AltCache {
    ttl: Duration,
    entry: Option<(String, CacheEntry)>,
}

impl Default for AltCache {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(120),
            entry: None,
        }
    }
}

impl AltCache {
    pub fn with_ttl_ms(ttl_ms: u64) -> Self {
        Self {
            ttl: Duration::from_millis(ttl_ms),
            entry: None,
        }
    }

    pub async fn get_tables(
        &mut self,
        rpc: &RpcClient,
        addresses: &[String],
    ) -> Result<Vec<AddressLookupTableAccount>, AltError> {
        if addresses.is_empty() {
            return Ok(vec![]);
        }
        let key = addresses.join(",");
        if let Some((cached_key, entry)) = &self.entry {
            if cached_key == &key && entry.at.elapsed() < self.ttl {
                return Ok(entry.tables.clone());
            }
        }
        let tables = fetch_address_lookup_tables(rpc, addresses).await?;
        self.entry = Some((
            key,
            CacheEntry {
                at: Instant::now(),
                tables: tables.clone(),
            },
        ));
        Ok(tables)
    }
}

pub async fn fetch_address_lookup_tables(
    rpc: &RpcClient,
    addresses: &[String],
) -> Result<Vec<AddressLookupTableAccount>, AltError> {
    let mut out = Vec::with_capacity(addresses.len());
    for addr in addresses {
        let key: Pubkey = addr
            .parse()
            .map_err(|_| AltError::InvalidData(Pubkey::default(), format!("bad pubkey: {addr}")))?;
        let account = rpc.get_account(&key).await.map_err(|_| AltError::NotFound(key))?;
        let addresses = parse_alt_addresses(&account.data)
            .map_err(|e| AltError::InvalidData(key, e))?;
        out.push(AddressLookupTableAccount { key, addresses });
    }
    Ok(out)
}

fn parse_alt_addresses(data: &[u8]) -> Result<Vec<Pubkey>, String> {
    if data.len() < LOOKUP_TABLE_META_SIZE {
        return Err("account too short".into());
    }
    let rest = &data[LOOKUP_TABLE_META_SIZE..];
    if !rest.len().is_multiple_of(32) {
        return Err("address bytes not aligned".into());
    }
    rest.chunks_exact(32)
        .map(|chunk| {
            let bytes: [u8; 32] = chunk.try_into().map_err(|_| "chunk")?;
            Ok(Pubkey::new_from_array(bytes))
        })
        .collect()
}
