use cosmwasm_schema::cw_serde;

#[cw_serde]
pub struct ContractInfoResponse {
    pub name: String,
    pub symbol: String,
}
