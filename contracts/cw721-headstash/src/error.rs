use cosmwasm_std::StdError;
use cw721::error::Cw721ContractError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error(transparent)]
    Cw721(#[from] Cw721ContractError),

    #[error("{0}")]
    Drop(String),
}

impl From<StdError> for ContractError {
    fn from(err: StdError) -> Self {
        ContractError::Cw721(err.into())
    }
}

pub fn drop_err(message: impl Into<String>) -> Cw721ContractError {
    Cw721ContractError::Std(StdError::msg(message.into()))
}
