//! Operator-reviewed expectations, keyed to an exact deployment. No role holders are inferred.
use crate::{
    contracts::abi,
    input::ResolutionInput,
    types::{Check, CheckStatus, Status},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
};
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadTarget {
    #[default]
    Deployment,
    Implementation,
    ProxyAdmin,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvmCheck {
    pub field: String,
    pub signature: String,
    #[serde(default)]
    pub target: ReadTarget,
    #[serde(default)]
    pub args: Vec<String>,
    pub expected: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LedgerCheck {
    pub field: String,
    pub pointer: String,
    pub expected: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentPolicy {
    pub input: ResolutionInput,
    #[serde(default)]
    pub evm_checks: Vec<EvmCheck>,
    #[serde(default)]
    pub ledger_checks: Vec<LedgerCheck>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileFormat {
    schema_version: u64,
    deployments: Vec<DeploymentPolicy>,
}
pub struct PolicyCatalog {
    pub version: String,
    deployments: HashMap<String, DeploymentPolicy>,
}
#[derive(Clone, Copy)]
pub enum Decode {
    Address,
    Bool,
    Uint,
    Text,
    Bytes32,
}
pub fn getter(signature: &str) -> Option<Decode> {
    Some(match signature {
        "owner()" | "pendingOwner()" | "asset()" | "identityRegistry()" | "compliance()"
        | "ruleEngine()" => Decode::Address,
        "paused()"
        | "isAgent(address)"
        | "isFrozen(address)"
        | "hasRole(bytes32,address)"
        | "canTransfer(address,address,uint256)" => Decode::Bool,
        "totalAssets()"
        | "convertToAssets(uint256)"
        | "convertToShares(uint256)"
        | "previewDeposit(uint256)"
        | "previewRedeem(uint256)"
        | "maxDeposit(address)"
        | "maxRedeem(address)"
        | "detectTransferRestriction(address,address,uint256)" => Decode::Uint,
        "version()" | "VERSION()" | "messageForTransferRestriction(uint8)" => Decode::Text,
        "proxiableUUID()" | "getRoleAdmin(bytes32)" => Decode::Bytes32,
        _ => return None,
    })
}
pub fn calldata(check: &EvmCheck) -> Result<String, String> {
    getter(&check.signature).ok_or("Unsupported read-only policy getter")?;
    let args = check
        .signature
        .split_once('(')
        .ok_or("Invalid signature")?
        .1
        .strip_suffix(')')
        .ok_or("Invalid signature")?;
    let kinds: Vec<_> = if args.is_empty() {
        vec![]
    } else {
        args.split(',').collect()
    };
    if kinds.len() != check.args.len() {
        return Err("Policy argument count mismatch".into());
    }
    let mut data = abi::selector(&check.signature);
    for (kind, arg) in kinds.into_iter().zip(&check.args) {
        let word = match kind {
            "address" => {
                let normalized = crate::input::ResolveInput::new(1, arg)
                    .map_err(|_| "Invalid policy address")?
                    .address;
                format!("{}{}", "0".repeat(24), &normalized[2..])
            }
            "bytes32" => {
                abi::word(arg).ok_or("Invalid bytes32 role ID")?;
                arg[2..].to_ascii_lowercase()
            }
            "uint256" | "uint8" => {
                let value = decimal_word(arg).ok_or("Invalid unsigned policy argument")?;
                if kind == "uint8" && value[..31].iter().any(|v| *v != 0) {
                    return Err("uint8 argument overflow".into());
                }
                hex::encode(value)
            }
            _ => return Err("Unsupported argument type".into()),
        };
        data += &word;
    }
    Ok(data)
}
fn decimal_word(value: &str) -> Option<[u8; 32]> {
    if value.is_empty() || value.len() > 78 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut word = [0u8; 32];
    for digit in value.bytes() {
        let mut carry = u16::from(digit - b'0');
        for byte in word.iter_mut().rev() {
            let n = u16::from(*byte) * 10 + carry;
            *byte = n as u8;
            carry = n >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(word)
}
pub fn decode(signature: &str, raw: &str) -> Option<Value> {
    Some(match getter(signature)? {
        Decode::Bool => Value::Bool(abi::boolean(raw)?),
        Decode::Uint => Value::String(abi::uint256(raw)?),
        Decode::Text => Value::String(abi::text(raw)?),
        Decode::Bytes32 => {
            abi::word(raw)?;
            Value::String(raw.to_ascii_lowercase())
        }
        Decode::Address => {
            let w = abi::word(raw)?;
            if w[..12].iter().any(|v| *v != 0) {
                return None;
            }
            Value::String(format!("0x{}", hex::encode(&w[12..])))
        }
    })
}
impl PolicyCatalog {
    pub fn empty() -> Self {
        Self {
            version: "resolver-policy-v1:none".into(),
            deployments: HashMap::new(),
        }
    }
    pub fn from_env() -> Result<Self, String> {
        match std::env::var_os("DEPLOYMENT_POLICIES_FILE") {
            None => Ok(Self::empty()),
            Some(path) => {
                let mut raw = Vec::new();
                File::open(path)
                    .map_err(|_| "Cannot read deployment policies")?
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut raw)
                    .map_err(|_| "Cannot read deployment policies")?;
                Self::from_bytes(&raw)
            }
        }
    }
    pub fn from_registry(catalog: &Value) -> Result<Self, String> {
        let deployments: Vec<Value> = catalog["deployments"].as_array().ok_or("Missing policy deployments")?.iter()
            .filter(|policy| policy["provenance"]["reviewStatus"] == "verified")
            .filter(|policy| policy["evmChecks"].as_array().is_some_and(|v| !v.is_empty()) || policy["ledgerChecks"].as_array().is_some_and(|v| !v.is_empty()))
            .map(|policy| serde_json::json!({"input": policy["input"], "evmChecks": policy["evmChecks"], "ledgerChecks": policy["ledgerChecks"]})).collect();
        Self::from_bytes(
            &serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"deployments":deployments}))
                .map_err(|_| "Cannot serialize policies")?,
        )
    }
    pub fn from_bytes(raw: &[u8]) -> Result<Self, String> {
        if raw.len() > 1024 * 1024 {
            return Err("Policy file exceeds 1 MiB".into());
        }
        let file: FileFormat = serde_json::from_slice(raw).map_err(|_| "Invalid policy JSON")?;
        if file.schema_version != 1 || file.deployments.len() > 4096 {
            return Err("Unsupported policy version or too many policies".into());
        }
        let mut deployments = HashMap::new();
        for mut policy in file.deployments {
            policy.input = policy
                .input
                .canonical()
                .map_err(|_| "Invalid policy locator")?;
            if policy.evm_checks.len() + policy.ledger_checks.len() > 32 {
                return Err("At most 32 checks per deployment".into());
            }
            let mut fields = HashSet::new();
            for field in policy
                .evm_checks
                .iter()
                .map(|c| &c.field)
                .chain(policy.ledger_checks.iter().map(|c| &c.field))
            {
                if field.is_empty()
                    || field.len() > 120
                    || !field
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    || !fields.insert(field)
                {
                    return Err("Invalid or duplicate policy field".into());
                }
            }
            for check in &mut policy.evm_checks {
                calldata(check)?;
                match getter(&check.signature).unwrap() {
                    Decode::Address => {
                        let address = check
                            .expected
                            .as_str()
                            .ok_or("Address expectation must be a string")?;
                        check.expected = Value::String(
                            crate::input::ResolveInput::new(1, address)
                                .map_err(|_| "Invalid expected address")?
                                .address,
                        );
                    }
                    Decode::Bool => {
                        if !check.expected.is_boolean() {
                            return Err("Boolean expectation required".into());
                        }
                    }
                    Decode::Uint => {
                        let word = decimal_word(
                            check
                                .expected
                                .as_str()
                                .ok_or("Uint expectation must be a decimal string")?,
                        )
                        .ok_or("Invalid uint expectation")?;
                        check.expected = Value::String(
                            abi::uint256(&format!("0x{}", hex::encode(word))).unwrap(),
                        );
                    }
                    Decode::Bytes32 => {
                        abi::word(
                            check
                                .expected
                                .as_str()
                                .ok_or("Bytes32 expectation required")?,
                        )
                        .ok_or("Invalid bytes32 expectation")?;
                        check.expected =
                            Value::String(check.expected.as_str().unwrap().to_lowercase());
                    }
                    Decode::Text => {
                        if check.expected.as_str().is_none_or(|s| s.len() > 4096) {
                            return Err("Text expectation required".into());
                        }
                    }
                }
            }
            for check in &policy.ledger_checks {
                if !check.pointer.starts_with('/') || check.pointer.len() > 512 {
                    return Err("Ledger expectation requires a bounded JSON pointer".into());
                }
            }
            match &policy.input {
                ResolutionInput::Evm(_) if !policy.ledger_checks.is_empty() => {
                    return Err("EVM policy contains ledger checks".into())
                }
                ResolutionInput::Ledger(_) if !policy.evm_checks.is_empty() => {
                    return Err("Ledger policy contains EVM checks".into())
                }
                _ => {}
            }
            if deployments.insert(policy.input.key(), policy).is_some() {
                return Err("Duplicate deployment policy".into());
            }
        }
        Ok(Self {
            version: format!("resolver-policy-v1:{}", hex::encode(Sha256::digest(raw))),
            deployments,
        })
    }
    pub fn get(&self, input: &ResolutionInput) -> Option<&DeploymentPolicy> {
        self.deployments.get(&input.key())
    }
}
pub fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
pub fn ledger_actual<'a>(observation: &'a Value, pointer: &str) -> Option<&'a Value> {
    observation.pointer(pointer).filter(|value| {
        !value.is_null()
            || observation["availableNullFields"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some(pointer)))
    })
}
pub fn null_fields(value: &Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        Value::Null => out.push(prefix.into()),
        Value::Object(map) => {
            for (key, value) in map {
                let key = key.replace('~', "~0").replace('/', "~1");
                null_fields(value, &format!("{prefix}/{key}"), out);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                null_fields(value, &format!("{prefix}/{index}"), out);
            }
        }
        _ => {}
    }
}
pub fn compare(field: String, expected: &Value, actual: Option<&Value>) -> Check {
    let status = match actual {
        None => CheckStatus::Unavailable,
        Some(a) if a == expected => CheckStatus::Verified,
        Some(_) => CheckStatus::Mismatched,
    };
    Check {
        field: format!("policy.{field}"),
        expected: value_text(expected),
        actual: actual.map(value_text),
        status,
    }
}
pub fn outcome(known: bool, checks: &[Check]) -> Status {
    if !known {
        Status::Unknown
    } else if checks.iter().any(|c| c.status == CheckStatus::Mismatched) {
        Status::Mismatch
    } else if checks.len() > 1 && checks.iter().all(|c| c.status == CheckStatus::Verified) {
        Status::Verified
    } else {
        Status::Partial
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_writes_duplicate_policies_and_uint_overflow() {
        let check = EvmCheck {
            field: "role".into(),
            signature: "hasRole(bytes32,address)".into(),
            target: ReadTarget::Deployment,
            args: vec![
                format!("0x{}", "0".repeat(64)),
                format!("0x{}", "1".repeat(40)),
            ],
            expected: Value::Bool(true),
        };
        assert_eq!(calldata(&check).unwrap().len(), 138);
        let mut write = check.clone();
        write.signature = "mint(address,uint256)".into();
        assert!(calldata(&write).is_err());
        assert!(decimal_word(&"9".repeat(78)).is_none());
        assert_eq!(
            decode("owner()", &format!("0x{}", "0".repeat(64))),
            Some(Value::String(format!("0x{}", "0".repeat(40))))
        );
    }
}
