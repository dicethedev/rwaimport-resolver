use crate::{contracts::ContractObservation, input::ResolveInput};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Verified,
    Partial,
    Mismatch,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Verified,
    Unavailable,
    Mismatched,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub field: String,
    pub expected: String,
    pub actual: Option<String>,
    pub status: CheckStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub product_id: String,
    pub issuer_id: String,
    pub underlying_asset_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub network: String,
    pub standard: String,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryMatch {
    pub identity: Identity,
    pub deployment: Deployment,
    /// Deployment observations, never inferred from product display labels.
    pub expected_name: Option<String>,
    pub expected_symbol: Option<String>,
    pub expected_decimals: Option<u8>,
    pub expected_admin: Option<String>,
    pub expected_implementation: Option<String>,
    pub expected_runtime_code_sha256: Option<String>,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveResult {
    pub status: Status,
    pub input: ResolveInput,
    pub registry_revision: String,
    pub matched: Option<RegistryMatch>,
    pub contract: Option<ContractObservation>,
    pub checks: Vec<Check>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    #[serde(flatten)]
    pub result: ResolveResult,
    pub resolved_at: String,
    pub registry_generated_at: String,
    pub product: Option<Value>,
    pub issuer: Option<Value>,
    pub underlying_asset: Option<Value>,
    pub deployment: Option<Value>,
    pub compliance: Option<Value>,
    pub valuation: Option<Value>,
    pub network: Option<Value>,
    pub standards: Vec<Value>,
    pub organizations: Vec<Value>,
    pub evidence: Option<Value>,
    pub verification_scope: String,
    #[serde(default)]
    pub evidence_freshness: Value,
    #[serde(flatten, default)]
    pub verification: VerificationMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VerificationMetadata {
    pub policy_version: String,
    pub policy_applied: bool,
    pub checks_performed: Vec<String>,
    pub checks_unavailable: Vec<String>,
}
impl VerificationMetadata {
    pub fn new(version: &str, applied: bool, checks: &[Check]) -> Self {
        Self {
            policy_version: version.into(),
            policy_applied: applied,
            checks_performed: checks
                .iter()
                .filter(|c| c.status != CheckStatus::Unavailable)
                .map(|c| c.field.clone())
                .collect(),
            checks_unavailable: checks
                .iter()
                .filter(|c| c.status == CheckStatus::Unavailable)
                .map(|c| c.field.clone())
                .collect(),
        }
    }
}
/// Shared envelope; input and observation retain ledger-specific information.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionEnvelope {
    pub status: Status,
    pub input: crate::input::ResolutionInput,
    pub registry_revision: String,
    #[serde(default)]
    pub matched: Option<Value>,
    #[serde(default)]
    pub contract: Option<ContractObservation>,
    #[serde(default)]
    pub observation: Option<Value>,
    pub checks: Vec<Check>,
    pub warnings: Vec<String>,
    pub resolved_at: String,
    pub registry_generated_at: String,
    pub product: Option<Value>,
    pub issuer: Option<Value>,
    pub underlying_asset: Option<Value>,
    pub deployment: Option<Value>,
    pub compliance: Option<Value>,
    pub valuation: Option<Value>,
    pub network: Option<Value>,
    pub standards: Vec<Value>,
    pub organizations: Vec<Value>,
    pub evidence: Option<Value>,
    pub evidence_freshness: Value,
    pub verification_scope: String,
    #[serde(flatten, default)]
    pub verification: VerificationMetadata,
}
impl TryFrom<Resolution> for ResolutionEnvelope {
    type Error = serde_json::Error;
    fn try_from(value: Resolution) -> Result<Self, Self::Error> {
        let mut raw = serde_json::to_value(value)?;
        raw["observation"] = raw["contract"].clone();
        serde_json::from_value(raw)
    }
}
