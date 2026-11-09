use cosmwasm_std::HexBinary;
use cw_storage_plus::{Item, Map};

pub const ROOT: Item<HexBinary> = Item::new("hs_root");
pub const BATCH_VK: Item<HexBinary> = Item::new("hs_batch_vk");
pub const SUBDENOM: Item<String> = Item::new("hs_subdenom");
/// `rseed_com` for one note of an eligible token. The raw string is not stored.
pub const NOTE_COM: Map<(&str, u32), HexBinary> = Map::new("hs_rseed_com");
pub const SPENT: Map<&[u8], ()> = Map::new("hs_spent");
