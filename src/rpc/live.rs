use crate::{
    config::Config,
    contracts::{abi, ContractObservation},
    errors::ResolveError,
    input::ResolveInput,
    proxy::{self, ProxyObservation},
};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
#[derive(Clone)]
pub struct LiveRpc {
    client: Client,
    timeout: Duration,
    retries: usize,
    next_id: Arc<AtomicU64>,
}
#[derive(Debug)]
pub(crate) enum RpcError {
    Unavailable,
    Remote,
    Malformed,
}
impl LiveRpc {
    pub fn new(config: &Config) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(config.rpc_timeout)
            .connect_timeout(config.rpc_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Cannot initialize RPC client")?;
        Ok(Self {
            client,
            timeout: config.rpc_timeout,
            retries: config.rpc_retries,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }
    pub(crate) async fn request(
        &self,
        url: &Url,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        for attempt in 0..=self.retries {
            let result = tokio::time::timeout(self.timeout, self.send(url, &body, id))
                .await
                .unwrap_or(Err(RpcError::Unavailable));
            match result {
                Err(RpcError::Unavailable) if attempt < self.retries => {
                    tokio::time::sleep(Duration::from_millis(50 * (attempt as u64 + 1))).await
                }
                other => return other,
            }
        }
        Err(RpcError::Unavailable)
    }
    async fn send(&self, url: &Url, body: &Value, id: u64) -> Result<Value, RpcError> {
        let mut response = self
            .client
            .post(url.clone())
            .json(body)
            .send()
            .await
            .map_err(|_| RpcError::Unavailable)?;
        if !response.status().is_success() {
            return Err(RpcError::Unavailable);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(RpcError::Malformed);
        }
        let mut raw = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| RpcError::Unavailable)? {
            if raw.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(RpcError::Malformed);
            }
            raw.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&raw).map_err(|_| RpcError::Malformed)?;
        if value["jsonrpc"] != "2.0" || value["id"].as_u64() != Some(id) {
            return Err(RpcError::Malformed);
        }
        if value.get("error").is_some() {
            return Err(RpcError::Remote);
        }
        value.get("result").cloned().ok_or(RpcError::Malformed)
    }
    pub(crate) async fn get_json(&self, url: &Url) -> Result<Value, RpcError> {
        for attempt in 0..=self.retries {
            let result = tokio::time::timeout(self.timeout, self.get_json_once(url))
                .await
                .unwrap_or(Err(RpcError::Unavailable));
            match result {
                Err(RpcError::Unavailable) if attempt < self.retries => {
                    tokio::time::sleep(Duration::from_millis(50 * (attempt as u64 + 1))).await
                }
                other => return other,
            }
        }
        Err(RpcError::Unavailable)
    }
    async fn get_json_once(&self, url: &Url) -> Result<Value, RpcError> {
        let mut response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| RpcError::Unavailable)?;
        if !response.status().is_success() {
            return Err(RpcError::Unavailable);
        }
        let mut raw = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| RpcError::Unavailable)? {
            if raw.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(RpcError::Malformed);
            }
            raw.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&raw).map_err(|_| RpcError::Malformed)
    }
    async fn raw_call(
        &self,
        url: &Url,
        address: &str,
        data: &str,
        block: &Value,
    ) -> Option<String> {
        self.request(
            url,
            "eth_call",
            json!([{"to": address, "data": data, "gas": if data.starts_with(&abi::selector("supportsInterface(bytes4)")) { "0x7530" } else { "0x1e8480" }}, block]),
        )
        .await
        .ok()?
        .as_str()
        .map(str::to_owned)
    }
    async fn call(
        &self,
        url: &Url,
        address: &str,
        signature: &str,
        block: &Value,
    ) -> Option<String> {
        self.raw_call(url, address, &abi::selector(signature), block)
            .await
    }
    async fn storage(&self, url: &Url, address: &str, slot: &str, block: &Value) -> Option<String> {
        self.request(url, "eth_getStorageAt", json!([address, slot, block]))
            .await
            .ok()?
            .as_str()
            .map(str::to_owned)
    }
    pub async fn observe(
        &self,
        url: &Url,
        input: &ResolveInput,
        standards: &[String],
    ) -> Result<ContractObservation, ResolveError> {
        let remote_chain = self
            .request(url, "eth_chainId", json!([]))
            .await
            .map_err(|_| ResolveError::RpcUnavailable)?;
        let remote_chain = remote_chain
            .as_str()
            .and_then(quantity)
            .ok_or(ResolveError::RpcUnavailable)?;
        if remote_chain != input.chain_id {
            return Err(ResolveError::RpcChainMismatch);
        }
        let head = self
            .request(url, "eth_getBlockByNumber", json!(["latest", false]))
            .await
            .map_err(|_| ResolveError::RpcUnavailable)?;
        let block_number = head["number"]
            .as_str()
            .and_then(quantity)
            .ok_or(ResolveError::RpcUnavailable)?;
        let block_hash = head["hash"]
            .as_str()
            .filter(|v| abi::word(v).is_some())
            .ok_or(ResolveError::RpcUnavailable)?
            .to_ascii_lowercase();
        let block = json!({"blockHash": block_hash, "requireCanonical": true});
        let code = self
            .request(url, "eth_getCode", json!([input.address, block]))
            .await
            .map_err(|_| ResolveError::RpcUnavailable)?;
        let code = code
            .as_str()
            .and_then(abi::bytes)
            .ok_or(ResolveError::RpcUnavailable)?;
        let mut observation = ContractObservation {
            block_number,
            block_hash,
            exists: !code.is_empty(),
            name: None,
            symbol: None,
            decimals: None,
            total_supply: None,
            runtime_code_sha256: hex::encode(Sha256::digest(&code)),
            proxy: ProxyObservation::Undetermined,
            owner: None,
            contract_admin: None,
            capabilities: BTreeMap::new(),
            relationships: BTreeMap::new(),
            warnings: vec![],
        };
        if code.is_empty() {
            return Ok(observation);
        }
        let address = &input.address;
        let (name, symbol, decimals, supply, owner) = tokio::join!(
            self.call(url, address, "name()", &block),
            self.call(url, address, "symbol()", &block),
            self.call(url, address, "decimals()", &block),
            self.call(url, address, "totalSupply()", &block),
            self.call(url, address, "owner()", &block)
        );
        observation.name = name.as_deref().and_then(abi::text);
        observation.symbol = symbol.as_deref().and_then(abi::text);
        observation.decimals = decimals
            .as_deref()
            .and_then(abi::small_uint)
            .and_then(|v| u8::try_from(v).ok());
        observation.total_supply = supply.as_deref().and_then(abi::uint256);
        observation.owner = owner.as_deref().and_then(abi::address);
        observation.capabilities.insert(
            "erc20Metadata".into(),
            crate::standards::erc20_metadata(&observation),
        );
        for (field, unavailable) in [
            ("name", observation.name.is_none()),
            ("symbol", observation.symbol.is_none()),
            ("decimals", observation.decimals.is_none()),
            ("totalSupply", observation.total_supply.is_none()),
            ("owner", observation.owner.is_none()),
        ] {
            if unavailable {
                observation.warnings.push(format!(
                    "{field} read unavailable, reverted, or could not be decoded"
                ));
            }
        }
        observation.contract_admin = self
            .storage(url, address, proxy::ADMIN_SLOT, &block)
            .await
            .as_deref()
            .and_then(abi::address);
        observation.proxy = self.resolve_proxy(url, address, &block, &code).await;
        if observation.proxy == ProxyObservation::Undetermined {
            observation.warnings.push(
                "Proxy pattern undetermined; this does not establish a direct contract".into(),
            );
        }
        let valid = format!(
            "{}01ffc9a7{}",
            abi::selector("supportsInterface(bytes4)"),
            "0".repeat(56)
        );
        let invalid = format!(
            "{}ffffffff{}",
            abi::selector("supportsInterface(bytes4)"),
            "0".repeat(56)
        );
        let (valid, invalid) = tokio::join!(
            self.raw_call(url, address, &valid, &block),
            self.raw_call(url, address, &invalid, &block)
        );
        let erc165 = crate::standards::erc165(
            valid.as_deref().and_then(abi::boolean),
            invalid.as_deref().and_then(abi::boolean),
        );
        observation.capabilities.insert("erc165".into(), erc165);
        if standards.iter().any(|s| s == "erc4626") {
            let (asset, assets) = tokio::join!(
                self.call(url, address, "asset()", &block),
                self.call(url, address, "totalAssets()", &block)
            );
            let asset = asset.as_deref().and_then(abi::address);
            let assets = assets.as_deref().and_then(abi::uint256);
            observation.capabilities.insert(
                "erc4626Reads".into(),
                if asset.is_some() && assets.is_some() {
                    Some(true)
                } else {
                    None
                },
            );
            observation.relationships.insert("vaultAsset".into(), asset);
            observation
                .relationships
                .insert("totalAssets".into(), assets);
        }
        if standards.iter().any(|s| s == "erc3643") {
            let (identity, compliance) = tokio::join!(
                self.call(url, address, "identityRegistry()", &block),
                self.call(url, address, "compliance()", &block)
            );
            let identity = identity.as_deref().and_then(abi::address);
            let compliance = compliance.as_deref().and_then(abi::address);
            observation.capabilities.insert(
                "erc3643Relationships".into(),
                if identity.is_some() && compliance.is_some() {
                    Some(true)
                } else {
                    None
                },
            );
            observation
                .relationships
                .insert("identityRegistry".into(), identity);
            observation
                .relationships
                .insert("compliance".into(), compliance);
        }
        if standards.iter().any(|s| s == "erc1400") {
            let granularity = self
                .call(url, address, "granularity()", &block)
                .await
                .as_deref()
                .and_then(abi::uint256);
            observation.capabilities.insert(
                "erc1400Granularity".into(),
                granularity.as_ref().map(|_| true),
            );
            observation
                .relationships
                .insert("granularity".into(), granularity);
        }
        // Detect reorgs during the observation. Do not return a mixture of forks.
        let confirm = self
            .request(
                url,
                "eth_getBlockByNumber",
                json!([format!("0x{block_number:x}"), false]),
            )
            .await
            .map_err(|_| ResolveError::RpcUnavailable)?;
        if confirm["hash"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(&observation.block_hash)
        {
            return Err(ResolveError::RpcUnavailable);
        }
        Ok(observation)
    }
    async fn resolve_proxy(
        &self,
        url: &Url,
        address: &str,
        block: &Value,
        code: &[u8],
    ) -> ProxyObservation {
        if let Some(implementation) = proxy::minimal_proxy(code) {
            return ProxyObservation::Detected {
                pattern: "eip1167".into(),
                implementation,
            };
        }
        let slot = self
            .storage(url, address, proxy::IMPLEMENTATION_SLOT, block)
            .await;
        let Some(slot) = slot else {
            return ProxyObservation::Undetermined;
        };
        if let Some(implementation) = abi::address(&slot) {
            return ProxyObservation::Detected {
                pattern: "eip1967".into(),
                implementation,
            };
        }
        if abi::word(&slot) != Some([0; 32]) {
            return ProxyObservation::Undetermined;
        }
        if let Some(beacon) = self
            .storage(url, address, proxy::BEACON_SLOT, block)
            .await
            .as_deref()
            .and_then(abi::address)
        {
            if let Some(implementation) = self
                .call(url, &beacon, "implementation()", block)
                .await
                .as_deref()
                .and_then(abi::address)
            {
                return ProxyObservation::Detected {
                    pattern: "eip1967-beacon".into(),
                    implementation,
                };
            }
        }
        // Legacy OpenZeppelin and EIP-1822 use the unmodified hash of these labels.
        for (label, pattern) in [
            ("org.zeppelinos.proxy.implementation", "openzeppelin-legacy"),
            ("PROXIABLE", "eip1822"),
        ] {
            let slot = format!(
                "0x{}",
                hex::encode(sha3::Keccak256::digest(label.as_bytes()))
            );
            if let Some(implementation) = self
                .storage(url, address, &slot, block)
                .await
                .as_deref()
                .and_then(abi::address)
            {
                return ProxyObservation::Detected {
                    pattern: pattern.into(),
                    implementation,
                };
            }
        }
        ProxyObservation::Undetermined
    }
}
pub fn quantity(value: &str) -> Option<u64> {
    let raw = value.strip_prefix("0x")?;
    if raw.is_empty() || (raw.len() > 1 && raw.starts_with('0')) {
        return None;
    }
    u64::from_str_radix(raw, 16).ok()
}

#[cfg(test)]
mod rest_tests {
    use super::*;
    use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
    use std::sync::atomic::AtomicUsize;
    #[tokio::test]
    async fn retries_transient_rest_failure_and_bounds_response_size() {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/",
                get(|State(calls): State<Arc<AtomicUsize>>| async move {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})))
                    } else {
                        (StatusCode::OK, Json(json!({"chain_id":1})))
                    }
                }),
            )
            .route(
                "/large",
                get(|| async { "x".repeat(MAX_RESPONSE_BYTES + 1) }),
            )
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url: Url = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut config = Config::from_env().unwrap();
        config.rpc_retries = 1;
        let rpc = LiveRpc::new(&config).unwrap();
        assert_eq!(rpc.get_json(&url).await.unwrap()["chain_id"], 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(matches!(
            rpc.get_json(&url.join("large").unwrap()).await,
            Err(RpcError::Malformed)
        ));
        task.abort();
    }
}
