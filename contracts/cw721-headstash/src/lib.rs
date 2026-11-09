//! The Headstash airdrop collection.
//!
//! This contract is the cw721 collection. Eligible accounts are tokens whose
//! traits carry `eligible_address`, `registered`, and later `bandersnatch_y`.
//! Transfer, approval, and mint stay the cw721 implementation. The extension
//! messages are the airdrop: register a claim key, rotate `rseed` commitments
//! for accounts that have not registered, and apply one signed batch.
//!
//! The batch message is the token transfer. It records each nullifier once and
//! mints the destination amounts. It does not verify a Halo2 proof or a
//! Ligerito proof. The sidecar already did that, and the batch signature is
//! what this contract checks.

pub mod contract;
pub mod error;
pub mod msg;
pub mod state;

pub use contract::entry;
pub use msg::{DropMsg, DropQuery, ExecuteMsg, InstantiateMsg, QueryMsg};

pub const TRAIT_ADDRESS: &str = "eligible_address";
pub const TRAIT_REGISTERED: &str = "registered";
pub const TRAIT_Y: &str = "bandersnatch_y";
pub const CONTRACT_NAME: &str = "crates.io:cw721-headstash";
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
