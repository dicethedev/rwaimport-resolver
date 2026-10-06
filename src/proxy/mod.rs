use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ProxyObservation {
    /// Absence of a known proxy signature does not prove a direct contract.
    Undetermined,
    Detected {
        pattern: String,
        implementation: String,
    },
}

pub const IMPLEMENTATION_SLOT: &str =
    "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
pub const BEACON_SLOT: &str = "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50";
pub fn minimal_proxy(code: &[u8]) -> Option<String> {
    let prefix = hex::decode("363d3d373d3d3d363d73").ok()?;
    let suffix = hex::decode("5af43d82803e903d91602b57fd5bf3").ok()?;
    if code.len() == 45 && code.starts_with(&prefix) && code.ends_with(&suffix) {
        Some(format!("0x{}", hex::encode(&code[10..30])))
    } else {
        None
    }
}

pub const ADMIN_SLOT: &str = "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";
