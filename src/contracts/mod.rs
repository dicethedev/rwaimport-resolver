pub mod abi;
use crate::proxy::ProxyObservation;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// Reads are pinned to a block hash with requireCanonical, where supported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractObservation {
    pub block_number: u64,
    pub block_hash: String,
    pub exists: bool,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub total_supply: Option<String>,
    pub runtime_code_sha256: String,
    pub proxy: ProxyObservation,
    pub owner: Option<String>,
    pub contract_admin: Option<String>,
    pub capabilities: BTreeMap<String, Option<bool>>,
    pub relationships: BTreeMap<String, Option<String>>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub policy_observations: BTreeMap<String, Option<serde_json::Value>>,
}
