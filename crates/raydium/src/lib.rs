//! Raydium CPMM integration — API v3 pool discovery, on-chain state, quote, swap ix.

pub mod api;
pub mod constants;
pub mod curve;
pub mod discover;
pub mod fees;
pub mod pool;
pub mod pool_select;
pub mod quote;
pub mod rpc;
pub mod swap;

pub use api::{ApiClient, HttpClientOptions};
pub use constants::*;
pub use pool::{CpmmPool, MintInfo, PoolRef, PoolSide};
pub use pool_select::{CpmmPoolChoice, PoolCandidateList, PoolSelectError, PoolSelector, SwapPoolSelect};
pub use quote::{min_amount_out, quote_swap_base_input, SwapQuote};
pub use rpc::{parse_commitment, RpcPoolLoader};

pub use solana_commitment_config::CommitmentConfig;
pub use swap::{SwapBuildParams, SwapDirection};
