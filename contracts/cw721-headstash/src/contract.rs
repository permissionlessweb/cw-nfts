use cosmwasm_std::{
    coins, to_json_binary, AnyMsg, BankMsg, Binary, CosmosMsg, Deps, DepsMut, Empty, Env,
    HexBinary, MessageInfo, Response, StdResult, Uint128,
};
use cw721::error::Cw721ContractError;
use cw721::execute::update_nft_info;
use cw721::msg::{Cw721InstantiateMsg, NftExtensionMsg};
use cw721::state::{Cw721Config, NftExtension, Trait, MINTER};
use cw721::traits::{Cw721Execute, Cw721Query};
use cw721::{
    DefaultOptionalCollectionExtension, DefaultOptionalCollectionExtensionMsg,
    DefaultOptionalNftExtension, DefaultOptionalNftExtensionMsg,
};
use headstash_wire::{
    canonical_batch, recover_address, registration_message, verify_batch, BatchPayout,
};

use crate::error::{drop_err, ContractError};
use crate::msg::{self, DropMsg, DropQuery, InstantiateMsg, NoteResponse, Payout, RootResponse};
use crate::state::{BATCH_VK, NOTE_COM, ROOT, SPENT, SUBDENOM};
use crate::{CONTRACT_NAME, CONTRACT_VERSION, TRAIT_ADDRESS, TRAIT_REGISTERED, TRAIT_Y};

const MINT_TYPE_URL: &str = "/osmosis.tokenfactory.v1beta1.MsgMint";

pub struct DropNft;

impl
    Cw721Execute<
        DefaultOptionalNftExtension,
        DefaultOptionalNftExtensionMsg,
        DefaultOptionalCollectionExtension,
        DefaultOptionalCollectionExtensionMsg,
        DropMsg,
        Empty,
    > for DropNft
{
    fn execute_extension(
        &self,
        deps: DepsMut,
        env: &Env,
        info: &MessageInfo,
        msg: DropMsg,
    ) -> Result<Response<Empty>, Cw721ContractError> {
        match msg {
            DropMsg::Register {
                token_id,
                y,
                signature,
            } => register(deps, token_id, y, signature),
            DropMsg::RotateUnregistered { new_root, notes } => rotate(deps, info, new_root, notes),
            DropMsg::ApplyBatch {
                root,
                rejected,
                payouts,
                signature,
            } => apply_batch(deps, env, root, rejected, payouts, signature),
        }
    }

    fn update_nft_info(
        &self,
        deps: DepsMut,
        env: &Env,
        info: &MessageInfo,
        token_id: String,
        token_uri: Option<String>,
        extension: DefaultOptionalNftExtensionMsg,
    ) -> Result<Response<Empty>, Cw721ContractError> {
        if let Some(ext) = &extension {
            if let Some(attributes) = &ext.attributes {
                for attr in attributes {
                    if msg::protected_trait(&attr.trait_type) {
                        return Err(drop_err(format!(
                            "trait {} is set by mint or register",
                            attr.trait_type
                        )));
                    }
                }
            }
        }
        update_nft_info::<DefaultOptionalNftExtension, DefaultOptionalNftExtensionMsg, Empty>(
            deps,
            env,
            Some(info),
            token_id,
            token_uri,
            extension,
        )
    }
}

impl Cw721Query<DefaultOptionalNftExtension, DefaultOptionalCollectionExtension, DropQuery>
    for DropNft
{
    fn query_extension(
        &self,
        deps: Deps,
        _env: &Env,
        msg: DropQuery,
    ) -> Result<Binary, Cw721ContractError> {
        let bin = match msg {
            DropQuery::Root {} => to_json_binary(&RootResponse {
                root: ROOT.load(deps.storage)?,
            })?,
            DropQuery::Note { token_id, fdi } => to_json_binary(&NoteResponse {
                rseed_com: NOTE_COM.may_load(deps.storage, (&token_id, fdi))?,
            })?,
            DropQuery::Spent { nullifier } => {
                let spent = SPENT.has(deps.storage, nullifier.as_slice());
                to_json_binary(&spent)?
            }
            DropQuery::Registered { token_id } => to_json_binary(&is_registered(deps, &token_id)?)?,
        };
        Ok(bin)
    }
}

pub mod entry {
    use super::*;

    #[cfg(not(feature = "library"))]
    use cosmwasm_std::entry_point;

    #[cfg_attr(not(feature = "library"), entry_point)]
    pub fn instantiate(
        mut deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: InstantiateMsg,
    ) -> Result<Response, ContractError> {
        if msg.root.len() != 32 {
            return Err(ContractError::Drop("root must be 32 bytes".into()));
        }
        if msg.batch_vk.len() != 33 {
            return Err(ContractError::Drop("batch key must be 33 bytes".into()));
        }
        if msg.subdenom.is_empty() {
            return Err(ContractError::Drop("subdenom must be set".into()));
        }
        let cw = Cw721InstantiateMsg {
            name: msg.name,
            symbol: msg.symbol,
            collection_info_extension: None,
            minter: msg.minter,
            creator: msg.creator,
            withdraw_address: None,
        };
        DropNft.instantiate_with_version(
            deps.branch(),
            &env,
            &info,
            cw,
            CONTRACT_NAME,
            CONTRACT_VERSION,
        )?;
        ROOT.save(deps.storage, &msg.root)?;
        BATCH_VK.save(deps.storage, &msg.batch_vk)?;
        SUBDENOM.save(deps.storage, &msg.subdenom)?;
        Ok(Response::new().add_attribute("action", "instantiate_headstash"))
    }

    #[cfg_attr(not(feature = "library"), entry_point)]
    pub fn execute(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: msg::ExecuteMsg,
    ) -> Result<Response, ContractError> {
        DropNft
            .execute(deps, &env, &info, msg)
            .map_err(ContractError::Cw721)
    }

    #[cfg_attr(not(feature = "library"), entry_point)]
    pub fn query(deps: Deps, env: Env, msg: msg::QueryMsg) -> Result<Binary, ContractError> {
        DropNft.query(deps, &env, msg).map_err(ContractError::Cw721)
    }
}

fn register(
    deps: DepsMut,
    token_id: String,
    y: String,
    signature: HexBinary,
) -> Result<Response<Empty>, Cw721ContractError> {
    let y_bytes = decode_hex(&y).map_err(|err| drop_err(err))?;
    if y_bytes.len() != 32 || y_bytes.iter().all(|byte| *byte == 0) {
        return Err(drop_err("Y must be 32 non-zero bytes"));
    }
    let cfg = Cw721Config::<Option<NftExtension>>::default();
    let mut nft = cfg.nft_info.load(deps.storage, &token_id)?;
    let eligible = trait_value(nft.extension.as_ref(), TRAIT_ADDRESS)
        .ok_or_else(|| drop_err("eligible_address trait is missing"))?;
    if is_registered_ext(nft.extension.as_ref()) {
        return Err(drop_err("account is already registered"));
    }
    let message = registration_message(&eligible, &y);
    let recovered =
        recover_address(&message, signature.as_slice()).map_err(|err| drop_err(err.to_string()))?;
    if normalize_addr(&eligible) != hex_encode(&recovered) {
        return Err(drop_err(
            "registration signature does not match the eligible address",
        ));
    }
    let ext = nft.extension.get_or_insert_with(NftExtension::default);
    set_trait(ext, TRAIT_REGISTERED, "true".into());
    set_trait(ext, TRAIT_Y, y);
    cfg.nft_info.save(deps.storage, &token_id, &nft)?;
    Ok(Response::new()
        .add_attribute("action", "register")
        .add_attribute("token_id", token_id))
}

fn rotate(
    deps: DepsMut,
    info: &MessageInfo,
    new_root: HexBinary,
    notes: Vec<msg::NoteRotation>,
) -> Result<Response<Empty>, Cw721ContractError> {
    MINTER
        .assert_owner(deps.storage, &info.sender)
        .map_err(|err| Cw721ContractError::Ownership(err))?;
    if new_root.len() != 32 {
        return Err(drop_err("root must be 32 bytes"));
    }
    if notes.is_empty() {
        return Err(drop_err("rotation needs at least one unregistered note"));
    }
    for note in &notes {
        if note.rseed_com.len() != 32 {
            return Err(drop_err("rseed_com must be 32 bytes"));
        }
        if is_registered(deps.as_ref(), &note.token_id)? {
            return Err(drop_err(format!(
                "account {} is registered and its rseed commitment stays",
                note.token_id
            )));
        }
    }
    for note in &notes {
        NOTE_COM.save(deps.storage, (&note.token_id, note.fdi), &note.rseed_com)?;
    }
    ROOT.save(deps.storage, &new_root)?;
    Ok(Response::new()
        .add_attribute("action", "rotate_unregistered")
        .add_attribute("notes", notes.len().to_string()))
}

fn apply_batch(
    deps: DepsMut,
    env: &Env,
    root: HexBinary,
    rejected: u32,
    payouts: Vec<Payout>,
    signature: HexBinary,
) -> Result<Response<Empty>, Cw721ContractError> {
    if root.len() != 32 {
        return Err(drop_err("batch root must be 32 bytes"));
    }
    if signature.len() != 64 {
        return Err(drop_err("batch signature must be 64 bytes"));
    }
    let mut wire = Vec::with_capacity(payouts.len());
    let mut seen = Vec::with_capacity(payouts.len());
    for payout in &payouts {
        if payout.nullifier.len() != 32 {
            return Err(drop_err("nullifier must be 32 bytes"));
        }
        if seen
            .iter()
            .any(|prev: &Vec<u8>| prev.as_slice() == payout.nullifier.as_slice())
        {
            return Err(drop_err("duplicate nullifier in batch"));
        }
        if SPENT.has(deps.storage, payout.nullifier.as_slice()) {
            return Err(drop_err("nullifier already spent"));
        }
        deps.api.addr_validate(&payout.destination)?;
        seen.push(payout.nullifier.to_vec());
        wire.push(BatchPayout {
            destination: payout.destination.clone(),
            amount: payout.amount.u128(),
            nullifier: payout.nullifier.as_slice().try_into().expect("32 bytes"),
        });
    }
    let mut root_bytes = [0u8; 32];
    root_bytes.copy_from_slice(root.as_slice());
    let message = canonical_batch(&root_bytes, rejected, &wire);
    let vk = BATCH_VK.load(deps.storage)?;
    if vk.len() != 33 {
        return Err(drop_err("stored batch key is not 33 bytes"));
    }
    let mut public = [0u8; 33];
    public.copy_from_slice(vk.as_slice());
    let mut sig = [0u8; 64];
    sig.copy_from_slice(signature.as_slice());
    verify_batch(&public, &message, &sig).map_err(|err| drop_err(err.to_string()))?;

    let denom = format!(
        "factory/{}/{}",
        env.contract.address,
        SUBDENOM.load(deps.storage)?
    );
    let mut messages = Vec::with_capacity(payouts.len() * 2);
    for payout in &payouts {
        SPENT.save(deps.storage, payout.nullifier.as_slice(), &())?;
        messages.extend(mint_msgs(
            &env.contract.address,
            &denom,
            payout.amount,
            &payout.destination,
        )?);
    }
    Ok(Response::new()
        .add_messages(messages)
        .add_attribute("action", "apply_batch")
        .add_attribute("payouts", payouts.len().to_string()))
}

fn mint_msgs(
    contract: &cosmwasm_std::Addr,
    denom: &str,
    amount: Uint128,
    to: &str,
) -> StdResult<Vec<CosmosMsg>> {
    let mint = encode_msg_mint(
        contract.as_str(),
        denom,
        &amount.to_string(),
        contract.as_str(),
    );
    let stargate = CosmosMsg::Any(AnyMsg {
        type_url: MINT_TYPE_URL.to_string(),
        value: Binary::new(mint),
    });
    let send = BankMsg::Send {
        to_address: to.to_string(),
        amount: coins(amount.u128(), denom),
    };
    Ok(vec![stargate, send.into()])
}

/// Protobuf `MsgMint`: sender = 1, amount = 2, mint_to_address = 3.
/// `Coin` is denom = 1, amount = 2. The bytes are what `CosmosMsg::Any` carries.
fn encode_msg_mint(sender: &str, denom: &str, amount: &str, mint_to: &str) -> Vec<u8> {
    let coin = [proto_string(1, denom), proto_string(2, amount)].concat();
    [
        proto_string(1, sender),
        proto_bytes(2, &coin),
        proto_string(3, mint_to),
    ]
    .concat()
}

fn proto_string(field: u32, value: &str) -> Vec<u8> {
    proto_bytes(field, value.as_bytes())
}

fn proto_bytes(field: u32, value: &[u8]) -> Vec<u8> {
    let mut out = proto_varint(((field as u64) << 3) | 2);
    out.extend(proto_varint(value.len() as u64));
    out.extend_from_slice(value);
    out
}

fn proto_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
    out
}

fn is_registered(deps: Deps, token_id: &str) -> Result<bool, Cw721ContractError> {
    let cfg = Cw721Config::<Option<NftExtension>>::default();
    let nft = cfg.nft_info.load(deps.storage, token_id)?;
    Ok(is_registered_ext(nft.extension.as_ref()))
}

fn is_registered_ext(ext: Option<&NftExtension>) -> bool {
    trait_value(ext, TRAIT_REGISTERED).as_deref() == Some("true")
}

fn trait_value(ext: Option<&NftExtension>, name: &str) -> Option<String> {
    ext.and_then(|ext| ext.attributes.as_ref())
        .and_then(|attrs| attrs.iter().find(|attr| attr.trait_type == name))
        .map(|attr| attr.value.clone())
}

fn set_trait(ext: &mut NftExtension, name: &str, value: String) {
    let attrs = ext.attributes.get_or_insert_with(Vec::new);
    if let Some(existing) = attrs.iter_mut().find(|attr| attr.trait_type == name) {
        existing.value = value;
    } else {
        attrs.push(Trait {
            display_type: None,
            trait_type: name.to_string(),
            value,
        });
    }
}

fn normalize_addr(value: &str) -> String {
    value.trim().trim_start_matches("0x").to_ascii_lowercase()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim().trim_start_matches("0x");
    if value.len() % 2 != 0 {
        return Err("hex length".into());
    }
    (0..value.len() / 2)
        .map(|i| u8::from_str_radix(&value[2 * i..2 * i + 2], 16).map_err(|err| err.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
    use cw721::msg::Cw721ExecuteMsg;
    use cw721::traits::Cw721Execute;
    use headstash_wire::{batch_public, sign_batch, sign_personal};

    fn setup(deps: DepsMut, sender: &cosmwasm_std::Addr) -> [u8; 32] {
        let secret = [9u8; 32];
        let info = message_info(sender, &[]);
        entry::instantiate(
            deps,
            mock_env(),
            info,
            InstantiateMsg {
                name: "Headstash".into(),
                symbol: "HS".into(),
                minter: Some(sender.to_string()),
                creator: Some(sender.to_string()),
                root: HexBinary::from([1u8; 32].to_vec()),
                batch_vk: HexBinary::from(batch_public(&secret).to_vec()),
                subdenom: "uterp".into(),
            },
        )
        .unwrap();
        secret
    }

    fn mint_eligible(deps: DepsMut, minter: &cosmwasm_std::Addr, token_id: &str, eth: &str) {
        let extension = Some(NftExtensionMsg {
            attributes: Some(vec![
                Trait {
                    display_type: None,
                    trait_type: TRAIT_ADDRESS.into(),
                    value: eth.into(),
                },
                Trait {
                    display_type: None,
                    trait_type: TRAIT_REGISTERED.into(),
                    value: "false".into(),
                },
            ]),
            ..NftExtensionMsg::default()
        });
        DropNft
            .execute(
                deps,
                &mock_env(),
                &message_info(minter, &[]),
                Cw721ExecuteMsg::Mint {
                    token_id: token_id.into(),
                    owner: minter.to_string(),
                    token_uri: None,
                    extension,
                },
            )
            .unwrap();
    }

    #[test]
    fn msg_mint_is_the_protobuf_any() {
        let bytes = encode_msg_mint("ab", "d", "1", "ab");
        assert_eq!(
            bytes,
            vec![
                0x0a, 0x02, b'a', b'b', 0x12, 0x06, 0x0a, 0x01, b'd', 0x12, 0x01, b'1', 0x1a, 0x02,
                b'a', b'b',
            ]
        );
    }

    #[test]
    fn rotation_stops_at_registration_and_the_batch_mints_once() {
        let mut deps = mock_dependencies();
        let minter = deps.api.addr_make("registrar");
        let dest = deps.api.addr_make("dest");
        let secret = setup(deps.as_mut(), &minter);
        mint_eligible(
            deps.as_mut(),
            &minter,
            "open",
            "aabbccddeeff00112233445566778899aabbccdd",
        );
        mint_eligible(
            deps.as_mut(),
            &minter,
            "held",
            "00112233445566778899aabbccddeeff00112233",
        );

        let com = HexBinary::from([7u8; 32].to_vec());
        DropNft
            .execute(
                deps.as_mut(),
                &mock_env(),
                &message_info(&minter, &[]),
                Cw721ExecuteMsg::UpdateExtension {
                    msg: DropMsg::RotateUnregistered {
                        new_root: HexBinary::from([2u8; 32].to_vec()),
                        notes: vec![
                            msg::NoteRotation {
                                token_id: "open".into(),
                                fdi: 0,
                                rseed_com: com.clone(),
                            },
                            msg::NoteRotation {
                                token_id: "held".into(),
                                fdi: 1,
                                rseed_com: com.clone(),
                            },
                        ],
                    },
                },
            )
            .unwrap();

        let eth_secret = [4u8; 32];
        let y = hex_encode(&[8u8; 32]);
        let probe = sign_personal(&eth_secret, b"probe").unwrap();
        let real_addr = hex_encode(&recover_address(b"probe", &probe).unwrap());
        mint_eligible(deps.as_mut(), &minter, "signed", &real_addr);
        let message = registration_message(&real_addr, &y);
        let signature = sign_personal(&eth_secret, &message).unwrap();
        DropNft
            .execute(
                deps.as_mut(),
                &mock_env(),
                &message_info(&minter, &[]),
                Cw721ExecuteMsg::UpdateExtension {
                    msg: DropMsg::Register {
                        token_id: "signed".into(),
                        y: y.clone(),
                        signature: HexBinary::from(signature.to_vec()),
                    },
                },
            )
            .unwrap();

        let frozen = DropNft.execute(
            deps.as_mut(),
            &mock_env(),
            &message_info(&minter, &[]),
            Cw721ExecuteMsg::UpdateExtension {
                msg: DropMsg::RotateUnregistered {
                    new_root: HexBinary::from([3u8; 32].to_vec()),
                    notes: vec![msg::NoteRotation {
                        token_id: "signed".into(),
                        fdi: 0,
                        rseed_com: HexBinary::from([9u8; 32].to_vec()),
                    }],
                },
            },
        );
        assert!(frozen.is_err());
        let root: RootResponse = cosmwasm_std::from_json(
            DropNft
                .query(
                    deps.as_ref(),
                    &mock_env(),
                    cw721::msg::Cw721QueryMsg::Extension {
                        msg: DropQuery::Root {},
                    },
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(root.root.as_slice(), &[2u8; 32]);

        let still_open = DropNft.execute(
            deps.as_mut(),
            &mock_env(),
            &message_info(&minter, &[]),
            Cw721ExecuteMsg::UpdateExtension {
                msg: DropMsg::RotateUnregistered {
                    new_root: HexBinary::from([4u8; 32].to_vec()),
                    notes: vec![msg::NoteRotation {
                        token_id: "open".into(),
                        fdi: 0,
                        rseed_com: HexBinary::from([5u8; 32].to_vec()),
                    }],
                },
            },
        );
        assert!(still_open.is_ok());

        let payout = Payout {
            destination: dest.to_string(),
            amount: Uint128::new(15),
            nullifier: HexBinary::from([6u8; 32].to_vec()),
        };
        let wire = [BatchPayout {
            destination: payout.destination.clone(),
            amount: 15,
            nullifier: [6u8; 32],
        }];
        let batch_root = [9u8; 32];
        let bytes = canonical_batch(&batch_root, 0, &wire);
        let signature = sign_batch(&secret, &bytes);
        let res = DropNft
            .execute(
                deps.as_mut(),
                &mock_env(),
                &message_info(&minter, &[]),
                Cw721ExecuteMsg::UpdateExtension {
                    msg: DropMsg::ApplyBatch {
                        root: HexBinary::from(batch_root.to_vec()),
                        rejected: 0,
                        payouts: vec![payout],
                        signature: HexBinary::from(signature.to_vec()),
                    },
                },
            )
            .unwrap();
        assert_eq!(res.messages.len(), 2);
        assert_eq!(
            res.messages[0].msg,
            mint_msgs(
                &mock_env().contract.address,
                &format!("factory/{}/uterp", mock_env().contract.address),
                Uint128::new(15),
                dest.as_str()
            )
            .unwrap()[0]
        );

        let replay = DropNft.execute(
            deps.as_mut(),
            &mock_env(),
            &message_info(&minter, &[]),
            Cw721ExecuteMsg::UpdateExtension {
                msg: DropMsg::ApplyBatch {
                    root: HexBinary::from(batch_root.to_vec()),
                    rejected: 0,
                    payouts: vec![Payout {
                        destination: dest.to_string(),
                        amount: Uint128::new(15),
                        nullifier: HexBinary::from([6u8; 32].to_vec()),
                    }],
                    signature: HexBinary::from(signature.to_vec()),
                },
            },
        );
        assert!(replay.is_err());
    }
}
