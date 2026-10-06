use crate::{
    errors::ResolveError,
    input::ResolveInput,
    registry::RegistryReader,
    types::{Deployment, Identity, RegistryMatch},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Clone)]
pub struct Context {
    pub matched: RegistryMatch,
    pub product: Value,
    pub issuer: Value,
    pub underlying: Value,
    pub deployment: Value,
    pub compliance: Value,
    pub valuation: Value,
    pub network: Value,
    pub standards: Vec<Value>,
    pub organizations: Vec<Value>,
    pub evidence: Value,
}
pub struct Distribution {
    pub revision: String,
    pub generated_at: String,
    chains: HashMap<u64, String>,
    contexts: HashMap<ResolveInput, Context>,
    pub ledger_registry: Value,
    pub policies: crate::policies::PolicyCatalog,
}
impl Distribution {
    pub fn load(directory: &Path) -> Result<Self, String> {
        let directory =
            std::fs::canonicalize(directory).map_err(|_| "Cannot resolve registry directory")?;
        Self::from_bytes(
            &read_distribution(&directory)?,
            &directory.join("../schemas"),
        )
    }
    pub fn from_bytes(raw: &[u8], schema_dir: &Path) -> Result<Self, String> {
        if raw.len() as u64 > MAX_BYTES {
            return Err("Registry exceeds 64 MiB".into());
        }
        let data: Value = serde_json::from_slice(raw).map_err(|_| "Invalid registry JSON")?;
        if data["schemaVersion"] != 2 {
            return Err("Expected registry schemaVersion 2".into());
        }
        let generated_at = string(&data, "generatedAt")?.to_owned();
        chrono::DateTime::parse_from_rfc3339(&generated_at).map_err(|_| "Invalid generatedAt")?;
        let mut validators = HashMap::new();
        for name in [
            "asset",
            "deployments",
            "compliance",
            "valuation",
            "sources",
            "claims",
            "history",
            "issuer",
            "chain",
            "standard",
            "underlying",
            "organization",
        ] {
            let schema: Value = serde_json::from_slice(&read_bounded(
                &schema_dir.join(format!("{name}.schema.json")),
            )?)
            .map_err(|_| "Invalid schema JSON")?;
            let validator = jsonschema::options()
                .should_validate_formats(true)
                .build(&schema)
                .map_err(|_| format!("Invalid {name} schema"))?;
            validators.insert(name, validator);
        }
        let mut collections = HashMap::new();
        for (key, schema) in [
            ("issuers", "issuer"),
            ("chains", "chain"),
            ("standards", "standard"),
            ("underlyings", "underlying"),
            ("organizations", "organization"),
        ] {
            let mut records = HashMap::new();
            for record in array(&data, key)? {
                validators[schema]
                    .validate(record)
                    .map_err(|_| format!("Invalid {schema} record"))?;
                let id = string(record, "id")?.to_owned();
                if records.insert(id, record.clone()).is_some() {
                    return Err(format!("Duplicate {key} ID"));
                }
            }
            collections.insert(key, records);
        }
        let mut chains = HashMap::new();
        let mut chain_refs = HashSet::new();
        for (id, chain) in &collections["chains"] {
            if !chain_refs.insert((
                chain["namespace"].clone().to_string(),
                chain["reference"].clone().to_string(),
            )) {
                return Err("Duplicate network reference".into());
            }
            if chain["type"] == "evm" {
                let chain_id = chain["chainId"].as_u64().ok_or("Missing EVM chainId")?;
                if chain_id == 0 || chains.insert(chain_id, id.clone()).is_some() {
                    return Err("Duplicate or invalid EVM chainId".into());
                }
            }
        }
        let mut contexts = HashMap::new();
        let mut products = HashSet::new();
        for bundle in array(&data, "assets")? {
            for name in [
                "asset",
                "deployments",
                "compliance",
                "valuation",
                "sources",
                "claims",
                "history",
            ] {
                validators[name]
                    .validate(&bundle[name])
                    .map_err(|_| format!("Invalid product {name}"))?;
            }
            let product = &bundle["asset"];
            let id = string(product, "id")?;
            if !products.insert(id.to_owned()) {
                return Err("Duplicate product ID".into());
            }
            let issuer_id = string(product, "issuerId")?;
            let underlying_id = string(product, "underlyingId")?;
            let issuer = collections["issuers"]
                .get(issuer_id)
                .ok_or("Unknown issuer")?;
            let underlying = collections["underlyings"]
                .get(underlying_id)
                .ok_or("Unknown underlying")?;
            let source_ids: Vec<String> = array(bundle, "sources")?
                .iter()
                .map(|v| string(v, "id").map(str::to_owned))
                .collect::<Result<_, _>>()?;
            let sources: HashSet<&str> = source_ids.iter().map(String::as_str).collect();
            if sources.len() != source_ids.len() {
                return Err("Duplicate product source ID".into());
            }
            for claim in array(bundle, "claims")? {
                check_sources(&claim["sourceIds"], &sources)?;
            }
            let mut history_ids = HashSet::new();
            for event in array(bundle, "history")? {
                if !history_ids.insert(string(event, "id")?) {
                    return Err("Duplicate history ID".into());
                }
                check_sources(&event["sourceIds"], &sources)?;
            }
            for record in [product, &bundle["compliance"], &bundle["valuation"]] {
                check_sources(&record["verifiedBy"], &sources)?;
            }
            for role in array(product, "organizationRoles")? {
                if !collections["organizations"].contains_key(string(role, "organizationId")?) {
                    return Err("Unknown organization".into());
                }
            }
            for provider in product["tokenizationProviderIds"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !collections["issuers"]
                    .contains_key(provider.as_str().ok_or("Invalid provider")?)
                {
                    return Err("Unknown provider".into());
                }
            }
            for dep in array(bundle, "deployments")? {
                let network = string(dep, "chain")?;
                let chain = collections["chains"]
                    .get(network)
                    .ok_or("Unknown deployment network")?;
                let standards = array(dep, "standardIds")?;
                for standard in standards {
                    if !collections["standards"]
                        .contains_key(standard.as_str().ok_or("Invalid standard")?)
                    {
                        return Err("Unknown standard".into());
                    }
                }
                check_sources(&dep["verifiedBy"], &sources)?;
                for evidence in dep["standardEvidence"].as_array().into_iter().flatten() {
                    if !standards.contains(&evidence["standardId"])
                        || !sources.contains(string(evidence, "sourceId")?)
                    {
                        return Err("Invalid standard evidence".into());
                    }
                }
                if chain["type"] != "evm" {
                    if dep.get("chainId").is_some() {
                        return Err("Non-EVM deployment has chainId".into());
                    }
                    continue;
                }
                let chain_id = dep["chainId"]
                    .as_u64()
                    .ok_or("Missing deployment chainId")?;
                if chain["chainId"].as_u64() != Some(chain_id) {
                    return Err("Deployment chainId conflict".into());
                }
                let address = string(dep, "address")?;
                let key = ResolveInput::new(chain_id, address)
                    .map_err(|_| "Invalid deployment address")?;
                let verification = &dep["verification"];
                let implementation = dep["implementationAddress"]
                    .as_str()
                    .or_else(|| verification["implementationAddress"].as_str())
                    .map(str::to_ascii_lowercase);
                let expected_decimals = dep["decimals"]
                    .as_u64()
                    .or_else(|| verification["observedDecimals"].as_u64())
                    .map(|v| v as u8);
                let matched = RegistryMatch {
                    identity: Identity {
                        product_id: id.into(),
                        issuer_id: issuer_id.into(),
                        underlying_asset_id: underlying_id.into(),
                    },
                    deployment: Deployment {
                        id: format!("{id}:{network}:{address}"),
                        network: network.into(),
                        standard: standards
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(","),
                        status: string(dep, "status")?.into(),
                    },
                    expected_name: verification["observedName"].as_str().map(str::to_owned),
                    expected_symbol: verification["observedSymbol"].as_str().map(str::to_owned),
                    expected_decimals,
                    expected_admin: verification["contractAdmin"]
                        .as_str()
                        .map(str::to_ascii_lowercase),
                    expected_implementation: implementation,
                    expected_runtime_code_sha256: verification["runtimeCodeSha256"]
                        .as_str()
                        .map(str::to_owned),
                    evidence_ids: source_ids.clone(),
                };
                let context = Context {
                    matched,
                    product: product.clone(),
                    issuer: issuer.clone(),
                    underlying: underlying.clone(),
                    deployment: dep.clone(),
                    compliance: bundle["compliance"].clone(),
                    valuation: bundle["valuation"].clone(),
                    network: chain.clone(),
                    standards: standards.iter().filter_map(|v| collections["standards"].get(v.as_str()?)).cloned().collect(),
                    organizations: array(product, "organizationRoles")?.iter().map(|role| json!({"organization": collections["organizations"][role["organizationId"].as_str().unwrap()], "roles": role["roles"]})).collect(),
                    evidence: json!({"sources": bundle["sources"], "claims": bundle["claims"], "underlyingSources": underlying["sources"], "underlyingClaims": underlying["claims"]}),
                };
                if contexts.insert(key, context).is_some() {
                    return Err("Duplicate deployment contract".into());
                }
            }
        }
        for bundle in array(&data, "assets")? {
            for relation in bundle["asset"]["relationships"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let source_ids: HashSet<&str> = array(bundle, "sources")?
                    .iter()
                    .filter_map(|v| v["id"].as_str())
                    .collect();
                check_sources(&relation["verifiedBy"], &source_ids)?;
                let related = string(relation, "assetId")?;
                if !products.contains(related) || bundle["asset"]["id"] == related {
                    return Err("Invalid product relationship".into());
                }
            }
        }
        for underlying in collections["underlyings"].values() {
            for product in array(underlying, "sourceAssetIds")? {
                if !products.contains(product.as_str().ok_or("Invalid underlying product ID")?) {
                    return Err("Unknown underlying product reference".into());
                }
            }
            let ids: Vec<&str> = underlying["sources"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["id"].as_str())
                .collect();
            let source_ids: HashSet<&str> = ids.iter().copied().collect();
            if ids.len() != source_ids.len() {
                return Err("Duplicate underlying source ID".into());
            }
            for claim in underlying["claims"].as_array().into_iter().flatten() {
                check_sources(&claim["sourceIds"], &source_ids)?;
            }
        }
        for standard in collections["standards"].values() {
            for related in standard["relatedStandardIds"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !collections["standards"]
                    .contains_key(related.as_str().ok_or("Invalid related standard")?)
                {
                    return Err("Unknown related standard".into());
                }
            }
        }
        let policies = if let Some(catalog) = data.get("deploymentPolicies") {
            let schema: Value = serde_json::from_slice(&read_bounded(
                &schema_dir.join("deployment-policies.schema.json"),
            )?)
            .map_err(|_| "Invalid policy schema JSON")?;
            let validator = jsonschema::options()
                .should_validate_formats(true)
                .build(&schema)
                .map_err(|_| "Invalid policy schema")?;
            if !validator.is_valid(catalog) {
                return Err("Invalid registry deployment policies".into());
            }
            let mut keys = HashSet::new();
            for policy in array(catalog, "deployments")? {
                let product = array(&data, "assets")?
                    .iter()
                    .find(|p| p["asset"]["id"] == policy["assetId"])
                    .ok_or("Unknown policy product")?;
                let target = array(product, "deployments")?
                    .iter()
                    .find(|d| {
                        d["chain"] == policy["deployment"]["chain"]
                            && if policy["input"].get("chainId").is_some()
                                || policy["input"]["network"] == "aptos"
                            {
                                d["address"].as_str().map(str::to_ascii_lowercase)
                                    == policy["deployment"]["address"]
                                        .as_str()
                                        .map(str::to_ascii_lowercase)
                            } else {
                                d["address"] == policy["deployment"]["address"]
                            }
                    })
                    .ok_or("Unknown policy deployment")?;
                if policy["deployment"].get("assetReference").is_some()
                    && policy["deployment"]["assetReference"] != target["assetReference"]
                {
                    return Err("Policy asset reference mismatch".into());
                }
                let input: crate::input::ResolutionInput =
                    serde_json::from_value(policy["input"].clone())
                        .map_err(|_| "Invalid policy input")?;
                let input = input.canonical().map_err(|_| "Invalid policy locator")?;
                if !keys.insert(input.key()) {
                    return Err("Duplicate policy locator".into());
                }
                match &input {
                    crate::input::ResolutionInput::Evm(value) => {
                        if target["chainId"].as_u64() != Some(value.chain_id)
                            || target["address"]
                                .as_str()
                                .map(str::to_ascii_lowercase)
                                .as_deref()
                                != Some(&value.address)
                        {
                            return Err("Policy EVM locator mismatch".into());
                        }
                    }
                    crate::input::ResolutionInput::Ledger(value) => {
                        let address_matches = if value.network == "aptos" {
                            target["address"]
                                .as_str()
                                .map(str::to_ascii_lowercase)
                                .as_deref()
                                == Some(&value.address)
                        } else {
                            target["address"].as_str() == Some(value.address.as_str())
                        };
                        let expected_coin = if target["assetNamespace"] == "coin" {
                            target["assetReference"]
                                .as_str()
                                .and_then(crate::ledgers::canonical_coin_type)
                        } else {
                            None
                        };
                        if value.network == "aptos" && value.coin_type != expected_coin {
                            return Err("Policy Aptos coin locator mismatch".into());
                        }
                        let symbol = target["verification"]["observedSymbol"]
                            .as_str()
                            .or_else(|| product["asset"]["symbol"].as_str());
                        if !address_matches
                            || (value.network == "stellar" && value.asset_code.as_deref() != symbol)
                            || target["chain"].as_str() != Some(value.network.as_str())
                        {
                            return Err("Policy ledger network mismatch".into());
                        }
                    }
                }
                let sources: HashSet<&str> = array(product, "sources")?
                    .iter()
                    .filter_map(|source| source["id"].as_str())
                    .collect();
                check_sources(&policy["provenance"]["sourceIds"], &sources)?;
            }
            crate::policies::PolicyCatalog::from_registry(catalog)?
        } else {
            crate::policies::PolicyCatalog::empty()
        };
        Ok(Self {
            policies,
            revision: hex::encode(Sha256::digest(raw)),
            generated_at,
            chains,
            contexts,
            ledger_registry: data,
        })
    }
    pub fn context(&self, input: &ResolveInput) -> Option<&Context> {
        self.contexts.get(input)
    }
    pub fn supported_chains(&self) -> Vec<u64> {
        let mut ids: Vec<_> = self.chains.keys().copied().collect();
        ids.sort();
        ids
    }
}
impl RegistryReader for Distribution {
    fn revision(&self) -> &str {
        &self.revision
    }
    fn supports_chain(&self, id: u64) -> bool {
        self.chains.contains_key(&id)
    }
    fn find_deployment(&self, input: &ResolveInput) -> Result<Option<RegistryMatch>, ResolveError> {
        Ok(self.context(input).map(|v| v.matched.clone()))
    }
}
pub fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|_| "Cannot read registry or schema file")?;
    let mut raw = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|_| "Cannot read registry or schema file")?;
    if raw.len() as u64 > MAX_BYTES {
        return Err("Registry or schema file exceeds 64 MiB".into());
    }
    Ok(raw)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("Missing {key}"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("Missing {key} array"))
}
fn check_sources(value: &Value, sources: &HashSet<&str>) -> Result<(), String> {
    for source in value.as_array().ok_or("Invalid source references")? {
        if !sources.contains(source.as_str().ok_or("Invalid source reference")?) {
            return Err("Unknown source reference".into());
        }
    }
    Ok(())
}

/// Manifest checks detect mixed/corrupted publication files, not publisher authenticity.
pub fn read_distribution(directory: &Path) -> Result<Vec<u8>, String> {
    let raw = read_bounded(&directory.join("registry.json"))?;
    let manifest_path = directory.join("manifest.json");
    match std::fs::metadata(&manifest_path) {
        Ok(_) => {
            let manifest: Value = serde_json::from_slice(&read_bounded(&manifest_path)?)
                .map_err(|_| "Invalid registry manifest JSON")?;
            validate_manifest(&raw, &manifest)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Cannot inspect registry manifest".into()),
    }
    Ok(raw)
}
pub fn validate_manifest(raw: &[u8], manifest: &Value) -> Result<(), String> {
    if manifest["schemaVersion"] != 1 {
        return Err("Unsupported registry manifest version".into());
    }
    let file = &manifest["files"]["registry.json"];
    if file["bytes"].as_u64() != Some(raw.len() as u64)
        || file["sha256"].as_str() != Some(hex::encode(Sha256::digest(raw)).as_str())
    {
        return Err("Registry bytes/checksum do not match manifest".into());
    }
    Ok(())
}
