use crate::errors::ResolveError;
use serde::{Deserialize, Serialize};

/// Canonical lookup key. Lowercase normalization is not EIP-55 validation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
