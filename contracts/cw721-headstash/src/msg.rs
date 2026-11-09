use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{HexBinary, Uint128};
use cw721::msg::{
    CollectionExtensionMsg, Cw721ExecuteMsg, Cw721QueryMsg, NftExtensionMsg, RoyaltyInfoResponse,
};
use cw721::{DefaultOptionalCollectionExtension, DefaultOptionalNftExtension};

#[cw_serde]
pub struct InstantiateMsg {
    pub name: String,
    pub symbol: String,
    pub minter: Option<String>,
    pub creator: Option<String>,
    /// Current Blake3 allocation root. 32 bytes.
    pub root: HexBinary,
    /// Compressed secp256k1 key of the sidecar that signs batches. 33 bytes.
    pub batch_vk: HexBinary,
    /// Tokenfactory subdenom. The batch mints `factory/{contract}/{subdenom}`.
    pub subdenom: String,
}

#[cw_serde]
pub struct NoteRotation {
    pub token_id: String,
    pub fdi: u32,
    /// Replacement `rseed_com`. 32 bytes. Not the secret string.
    pub rseed_com: HexBinary,
}

#[cw_serde]
pub struct Payout {
    pub destination: String,
    pub amount: Uint128,
    pub nullifier: HexBinary,
}

/// Airdrop messages carried in cw721 `UpdateExtension`.
#[cw_serde]
pub enum DropMsg {
    /// Bind `Y` to the eligible address on this token. The signature is
    /// MetaMask `personal_sign` of [`headstash_wire::registration_message`].
    Register {
        token_id: String,
        /// 32-byte Bandersnatch point, hex without a `0x` prefix.
        y: String,
        signature: HexBinary,
    },
    /// Replace the root and the listed `rseed_com` values.
    /// Every listed token must still be unregistered. Registered accounts
    /// keep the commitment they already had.
    RotateUnregistered {
        new_root: HexBinary,
        notes: Vec<NoteRotation>,
    },
    /// One signed sidecar batch. This is the mint. Nullifiers are recorded
    /// once. Proof bytes are not an input.
    ApplyBatch {
        root: HexBinary,
        rejected: u32,
        payouts: Vec<Payout>,
        signature: HexBinary,
    },
}

impl cw721::traits::Cw721CustomMsg for DropMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum DropQuery {
    #[returns(RootResponse)]
    Root {},
    #[returns(NoteResponse)]
    Note { token_id: String, fdi: u32 },
    #[returns(bool)]
    Spent { nullifier: HexBinary },
    #[returns(bool)]
    Registered { token_id: String },
}

impl cw721::traits::Cw721CustomMsg for DropQuery {}

#[cw_serde]
pub struct RootResponse {
    pub root: HexBinary,
}

#[cw_serde]
pub struct NoteResponse {
    pub rseed_com: Option<HexBinary>,
}

pub type ExecuteMsg = Cw721ExecuteMsg<
    Option<NftExtensionMsg>,
    Option<CollectionExtensionMsg<RoyaltyInfoResponse>>,
    DropMsg,
>;

pub type QueryMsg =
    Cw721QueryMsg<DefaultOptionalNftExtension, DefaultOptionalCollectionExtension, DropQuery>;

/// Trait names the collection understands. Other cw721 metadata stays untouched.
pub fn protected_trait(name: &str) -> bool {
    matches!(
        name,
        crate::TRAIT_ADDRESS | crate::TRAIT_REGISTERED | crate::TRAIT_Y
    )
}
