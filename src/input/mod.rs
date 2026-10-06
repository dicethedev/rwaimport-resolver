use crate::errors::ResolveError;
use serde::{Deserialize, Serialize};

/// Canonical lookup key. Lowercase normalization is not EIP-55 validation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveInput {
    pub chain_id: u64,
    pub address: String,
}

impl ResolveInput {
    pub fn new(chain_id: u64, address: &str) -> Result<Self, ResolveError> {
        if chain_id == 0 {
            return Err(ResolveError::InvalidChainId);
        }
        if address.len() != 42
            || !address.starts_with("0x")
            || !address[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ResolveError::InvalidAddress);
        }
        Ok(Self {
            chain_id,
            address: address.to_ascii_lowercase(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LedgerInput {
    pub network: String,
    pub address: String,
    #[serde(default)]
    pub asset_code: Option<String>,
    #[serde(default)]
    pub coin_type: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResolutionInput {
    Evm(ResolveInput),
    Ledger(LedgerInput),
}
impl ResolutionInput {
    pub fn canonical(&self) -> Result<Self, ResolveError> {
        match self {
            Self::Evm(input) => ResolveInput::new(input.chain_id, &input.address).map(Self::Evm),
            Self::Ledger(input) => {
                if input.network != "stellar" && input.asset_code.is_some()
                    || input.network != "aptos" && input.coin_type.is_some()
                {
                    return Err(ResolveError::InvalidAddress);
                }
                let mut input = input.clone();
                crate::ledgers::validate_input(
                    &input.network,
                    &input.address,
                    input.asset_code.as_deref().or(input.coin_type.as_deref()),
                )?;
                if input.network == "aptos" {
                    input.address = input.address.to_ascii_lowercase();
                    input.coin_type = input
                        .coin_type
                        .as_deref()
                        .and_then(crate::ledgers::canonical_coin_type);
                    if let Some(coin) = &input.coin_type {
                        let creator = coin.split("::").next().unwrap()[2..].to_owned();
                        if format!("0x{creator:0>64}") != input.address {
                            return Err(ResolveError::InvalidAddress);
                        }
                    }
                }
                Ok(Self::Ledger(input))
            }
        }
    }
    pub fn key(&self) -> String {
        serde_json::to_string(self).expect("input is serializable")
    }
}
