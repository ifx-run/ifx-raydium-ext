//! Ifx transaction planner — routing, quote, Direct / Bridge build.

pub mod close_ata;
pub mod alt;
pub mod build;
pub mod finalize;
pub mod inspect;
pub mod pay_asset;
pub mod platform_fee;
pub mod route;
pub mod service_fee;
pub mod sign;
pub mod simulate;
pub mod sponsor;
pub use sponsor::{compute_tx_fee_lamports, bind_sponsored_leg2_min_out};
pub mod tx_size;
pub mod tx_v1;

pub use pay_asset::{sol_pay_asset_for_mint, SolPayAsset};
pub use build::{BuildPlan, BuildResult, Planner};
pub use finalize::{finalize_build, FinalizeRequest, FinalizeResult, Finalizer};
pub use inspect::{
    inspect_instructions, inspect_versioned_transaction, InspectOpts, TransactionConfigInspection,
    TxAccountInspection, TxInspection, TxInstructionInspection,
};
pub use route::{RouteDecision, RouteKind, RoutePlan, RouteQuote, Router};
pub use sign::{read_keypair, SignError};
pub use simulate::{simulate_transaction, SimulateRequest, SimulateResult};
