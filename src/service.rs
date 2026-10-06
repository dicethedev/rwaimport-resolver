use crate::{
    cache::{CacheKey, ResolutionCache},
    config::Config,
    errors::ResolveError,
    input::ResolveInput,
    registry::{
        distribution::{read_distribution, Distribution},
        RegistryReader,
    },
    rpc::live::LiveRpc,
    types::{Resolution, ResolveResult},
    verification::verify,
};
use sha2::{Digest, Sha256};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, Semaphore};

pub struct ResolverService {
    pub config: Config,
    snapshot: RwLock<Arc<Distribution>>,
    rpc: LiveRpc,
    rpc_backups: std::collections::HashMap<u64, Vec<reqwest::Url>>,
    ledger_providers: std::collections::HashMap<String, Vec<reqwest::Url>>,
    cache: Mutex<ResolutionCache>,
    slots: Arc<Semaphore>,
    locks: Vec<Mutex<()>>,
    refresh_failure: RwLock<bool>,
    policies: crate::policies::PolicyCatalog,
    ledger_cache: Mutex<ResolutionCache<String, crate::types::ResolutionEnvelope>>,
    metrics: crate::operations::ResolverMetrics,
    pub batch_max: usize,
    pub batch_timeout: std::time::Duration,
    pub max_response_bytes: usize,
    max_registry_age: u64,
    require_rpc_ready: bool,
}
impl ResolverService {
    pub fn new(config: Config, snapshot: Distribution) -> Result<Self, String> {
        Self::with_policies(
            config,
            snapshot,
            crate::policies::PolicyCatalog::from_env()?,
        )
    }
    pub fn with_policies(
        config: Config,
        snapshot: Distribution,
        policies: crate::policies::PolicyCatalog,
    ) -> Result<Self, String> {
        Self::with_sources(
            config,
            snapshot,
            policies,
            crate::ledgers::providers()?,
            crate::ledgers::evm_backups()?,
        )
    }
    pub fn with_sources(
        config: Config,
        snapshot: Distribution,
        policies: crate::policies::PolicyCatalog,
        ledger_providers: std::collections::HashMap<String, Vec<reqwest::Url>>,
        rpc_backups: std::collections::HashMap<u64, Vec<reqwest::Url>>,
    ) -> Result<Self, String> {
        let mut rpc = LiveRpc::new(&config)?;
        rpc.health = crate::operations::ProviderHealth::new(
            setting("PROVIDER_FAILURE_THRESHOLD", 3, 1, 100)?,
            std::time::Duration::from_secs(setting("PROVIDER_COOLDOWN_SECONDS", 30, 1, 3600)?),
        );

        for url in config
            .rpc_urls
            .values()
            .chain(rpc_backups.values().flatten())
            .chain(ledger_providers.values().flatten())
        {
            rpc.health.register(url);
        }
        let ledger_cache =
            ResolutionCache::new(config.cache_ttl, config.cache_entries, config.cache_bytes);
        let cache =
            ResolutionCache::new(config.cache_ttl, config.cache_entries, config.cache_bytes);
        let slots = Arc::new(Semaphore::new(config.max_concurrency));
        Ok(Self {
            config,
            snapshot: RwLock::new(Arc::new(snapshot)),
            rpc,
            ledger_providers,
            rpc_backups,
            policies,
            ledger_cache: Mutex::new(ledger_cache),
            metrics: Default::default(),
            batch_timeout: std::time::Duration::from_millis(setting(
                "RESOLVER_BATCH_TIMEOUT_MS",
                60000,
                100,
                120000,
            )?),
            max_response_bytes: setting("RESOLVER_MAX_RESPONSE_BYTES", 4194304, 1024, 16777216)?
                as usize,
            batch_max: setting("RESOLVER_BATCH_MAX", 64, 1, 256)? as usize,
            max_registry_age: setting("REGISTRY_MAX_AGE_SECONDS", 0, 0, 31536000)?,
            require_rpc_ready: match std::env::var("READINESS_REQUIRE_RPC").as_deref() {
                Ok("true") => true,
                Ok("false") | Err(_) => false,
                _ => return Err("READINESS_REQUIRE_RPC must be true or false".into()),
            },
            cache: Mutex::new(cache),
            slots,
            locks: (0..64).map(|_| Mutex::new(())).collect(),
            refresh_failure: RwLock::new(false),
        })
    }
    pub async fn resolve(&self, chain_id: u64, address: &str) -> Result<Resolution, ResolveError> {
        let deadline = tokio::time::Instant::now() + self.config.resolve_timeout;
        let input = ResolveInput::new(chain_id, address)?;
        let snapshot = self.snapshot.read().await.clone();
        if !snapshot.supports_chain(chain_id) {
            return Err(ResolveError::UnsupportedChain);
        }
        let degraded = *self.refresh_failure.read().await;
        let key = CacheKey {
            input: input.clone(),
            revision: snapshot.revision.clone(),
        };
        if !degraded {
            if let Some(value) = self.cache.lock().await.get(&key) {
                self.metrics
                    .cache_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(value);
            }
        }
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ResolveError::Busy)?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hash);
        // Fixed lock stripes bound memory while coalescing simultaneous identical work.
        let _lock = tokio::time::timeout_at(
            deadline,
            self.locks[hash.finish() as usize % self.locks.len()].lock(),
        )
        .await
        .map_err(|_| ResolveError::Busy)?;
        if !degraded {
            if let Some(value) = self.cache.lock().await.get(&key) {
                self.metrics
                    .cache_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(value);
            }
        }
        let policy_input = crate::input::ResolutionInput::Evm(input.clone());
        let catalog = if self.policies.get(&policy_input).is_some() {
            &self.policies
        } else {
            &snapshot.policies
        };
        let policy = catalog.get(&policy_input);
        let context = snapshot.context(&input);
        let matched = context.map(|c| c.matched.clone());
        let standards: Vec<String> = context
            .into_iter()
            .flat_map(|c| c.deployment["standardIds"].as_array().into_iter().flatten())
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let mut warnings = vec![];
        if degraded {
            warnings.push("Registry refresh failed; using last valid revision".into());
        }
        let mut observed = None;
        let urls: Vec<_> = self
            .config
            .rpc_urls
            .get(&chain_id)
            .into_iter()
            .chain(self.rpc_backups.get(&chain_id).into_iter().flatten())
            .collect();
        if urls.is_empty() {
            warnings
                .push("RPC provider not configured for this chain; live checks unavailable".into());
        }
        for (index, url) in urls.iter().enumerate() {
            if !self.rpc.health.available(url) {
                warnings.push("Provider circuit is open; trying next provider".into());
                continue;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let attempt_deadline =
                tokio::time::Instant::now() + remaining / (urls.len() - index) as u32;
            match tokio::time::timeout_at(
                attempt_deadline,
                self.rpc
                    .observe_with_policy(url, &input, &standards, policy),
            )
            .await
            {
                Ok(Ok(value)) => {
                    self.rpc.health.observation_finished(url, true);
                    observed = Some(value);
                    break;
                }
                Ok(Err(ResolveError::RpcChainMismatch)) => {
                    self.rpc.health.reject_network(url);
                    return Err(ResolveError::RpcChainMismatch);
                }
                _ => {
                    self.rpc.health.observation_finished(url, false);
                    warnings.push("Live RPC observation unavailable; trying next provider within request deadline".into());
                }
            }
        }
        let (_, mut checks) = verify(matched.as_ref(), observed.as_ref());
        if let Some(policy) = policy {
            for check in &policy.evm_checks {
                checks.push(crate::policies::compare(
                    check.field.clone(),
                    &check.expected,
                    observed
                        .as_ref()
                        .and_then(|o| o.policy_observations.get(&check.field))
                        .and_then(Option::as_ref),
                ));
            }
        }
        let status = crate::policies::outcome(matched.is_some(), &checks);
        let verification =
            crate::types::VerificationMetadata::new(&catalog.version, policy.is_some(), &checks);
        // Optional reads do not suppress caching when all selected claims are available.
        let cacheable = !degraded
            && observed.is_some()
            && checks
                .iter()
                .all(|c| c.status != crate::types::CheckStatus::Unavailable);
        let resolution = Resolution {
   verification,
   result: ResolveResult { status, input, registry_revision: snapshot.revision.clone(), matched, contract: observed, checks, warnings },
   resolved_at: chrono::Utc::now().to_rfc3339(), registry_generated_at: snapshot.generated_at.clone(),
   product: context.map(|c| c.product.clone()), issuer: context.map(|c| c.issuer.clone()), underlying_asset: context.map(|c| c.underlying.clone()),
   deployment: context.map(|c| c.deployment.clone()), compliance: context.map(|c| c.compliance.clone()),
   valuation: context.map(|c| c.valuation.clone()), network: context.map(|c| c.network.clone()),
   standards: context.map(|c| c.standards.clone()).unwrap_or_default(), organizations: context.map(|c| c.organizations.clone()).unwrap_or_default(),
   evidence: context.map(|c| c.evidence.clone()),
   evidence_freshness: crate::freshness::summary(context.map(|c| &c.evidence), context.map(|c| &c.deployment)),
   verification_scope: "Selected deployment metadata, runtime bytecode hash and implementation claims only; standard conformance, legal rights, reserves and safety are not established".into(),
  };
        if cacheable {
            self.cache.lock().await.insert(key, resolution.clone());
        }
        Ok(resolution)
    }
    pub async fn resolve_ledger(
        &self,
        network: &str,
        address: &str,
        code: Option<&str>,
    ) -> Result<crate::types::ResolutionEnvelope, ResolveError> {
        if code.is_some() && network != "stellar" && network != "aptos" {
            return Err(ResolveError::InvalidAddress);
        }
        let raw_input = crate::input::ResolutionInput::Ledger(crate::input::LedgerInput {
            network: network.into(),
            address: address.into(),
            asset_code: if network == "stellar" {
                code.map(str::to_owned)
            } else {
                None
            },
            coin_type: if network == "aptos" {
                code.map(str::to_owned)
            } else {
                None
            },
        });
        let input = raw_input.canonical()?;
        let crate::input::ResolutionInput::Ledger(ref canonical) = input else {
            unreachable!()
        };
        let deadline = tokio::time::Instant::now() + self.config.resolve_timeout;
        let snapshot = self.snapshot.read().await.clone();
        let degraded = *self.refresh_failure.read().await;
        let key = format!(
            "{}:{}:{}",
            input.key(),
            snapshot.revision,
            self.policies.version
        );
        if !degraded {
            if let Some(result) = self.ledger_cache.lock().await.get(&key) {
                self.metrics
                    .cache_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(result);
            }
        }
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ResolveError::Busy)?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hash);
        let _lock = tokio::time::timeout_at(
            deadline,
            self.locks[hash.finish() as usize % self.locks.len()].lock(),
        )
        .await
        .map_err(|_| ResolveError::Busy)?;
        if !degraded {
            if let Some(result) = self.ledger_cache.lock().await.get(&key) {
                self.metrics
                    .cache_hits
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(result);
            }
        }
        let budget = deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .saturating_sub(std::time::Duration::from_millis(50));
        let raw = tokio::time::timeout_at(
            deadline,
            crate::ledgers::resolve(
                &self.rpc,
                &self.ledger_providers,
                &snapshot,
                (
                    &canonical.network,
                    &canonical.address,
                    canonical
                        .asset_code
                        .as_deref()
                        .or(canonical.coin_type.as_deref()),
                ),
                degraded,
                budget,
            ),
        )
        .await
        .map_err(|_| ResolveError::RpcUnavailable)??;
        let mut result: crate::types::ResolutionEnvelope =
            serde_json::from_value(raw).map_err(|_| ResolveError::RegistryUnavailable)?;
        let catalog = if self.policies.get(&input).is_some() {
            &self.policies
        } else {
            &snapshot.policies
        };
        let policy = catalog.get(&input);
        if let Some(policy) = policy {
            for check in &policy.ledger_checks {
                result.checks.push(crate::policies::compare(
                    check.field.clone(),
                    &check.expected,
                    result
                        .observation
                        .as_ref()
                        .and_then(|o| crate::policies::ledger_actual(o, &check.pointer)),
                ));
            }
        }
        result.status = crate::policies::outcome(result.product.is_some(), &result.checks);
        result.verification = crate::types::VerificationMetadata::new(
            &catalog.version,
            policy.is_some(),
            &result.checks,
        );
        if !degraded
            && result.observation.is_some()
            && result
                .checks
                .iter()
                .all(|c| c.status != crate::types::CheckStatus::Unavailable)
        {
            self.ledger_cache.lock().await.insert(key, result.clone());
        }
        Ok(result)
    }
    pub async fn resolve_any(
        &self,
        input: crate::input::ResolutionInput,
    ) -> Result<crate::types::ResolutionEnvelope, ResolveError> {
        self.metrics
            .requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let started = std::time::Instant::now();
        let result = match input.canonical() {
            Err(error) => Err(error),
            Ok(crate::input::ResolutionInput::Evm(input)) => self
                .resolve(input.chain_id, &input.address)
                .await
                .and_then(|r| r.try_into().map_err(|_| ResolveError::RegistryUnavailable)),
            Ok(crate::input::ResolutionInput::Ledger(input)) => {
                self.resolve_ledger(
                    &input.network,
                    &input.address,
                    input.asset_code.as_deref().or(input.coin_type.as_deref()),
                )
                .await
            }
        };
        if result.is_err() {
            self.metrics
                .failures
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        crate::operations::event(
            "resolution",
            serde_json::json!({"elapsedMilliseconds":started.elapsed().as_millis(),"status":result.as_ref().map(|r|format!("{:?}",r.status)).unwrap_or_else(|e|format!("{e:?}"))}),
        );
        result
    }
    pub fn metrics(&self) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        format!("rwaimport_resolutions_total {}\nrwaimport_resolution_errors_total {}\nrwaimport_cache_hits_total {}\n{}",self.metrics.requests.load(Relaxed),self.metrics.failures.load(Relaxed),self.metrics.cache_hits.load(Relaxed),self.rpc.health.metrics())
    }
    pub async fn refresh(&self) -> Result<bool, String> {
        let directory = self.config.registry_dir.clone();
        let old_revision = self.snapshot.read().await.revision.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            let directory = std::fs::canonicalize(directory)
                .map_err(|_| "Cannot resolve registry directory")?;
            let raw = read_distribution(&directory)?;
            if hex::encode(Sha256::digest(&raw)) == old_revision {
                return Ok(None);
            }
            Distribution::from_bytes(&raw, &directory.join("../schemas")).map(Some)
        })
        .await
        .map_err(|_| "Registry refresh worker failed".to_owned())?;
        match loaded {
            Ok(Some(snapshot)) => {
                *self.snapshot.write().await = Arc::new(snapshot);
                self.cache.lock().await.clear();
                self.ledger_cache.lock().await.clear();
                *self.refresh_failure.write().await = false;
                crate::operations::event(
                    "registry_activated",
                    serde_json::json!({"registryRevision":self.snapshot.read().await.revision}),
                );
                Ok(true)
            }
            Ok(None) => {
                *self.refresh_failure.write().await = false;
                Ok(false)
            }
            Err(error) => {
                *self.refresh_failure.write().await = true;
                Err(error)
            }
        }
    }
    pub async fn probe_providers(&self) {
        let mut urls: Vec<(u64, reqwest::Url)> = self
            .config
            .rpc_urls
            .iter()
            .map(|(id, url)| (*id, url.clone()))
            .collect();
        for (id, backups) in &self.rpc_backups {
            for url in backups {
                urls.push((*id, url.clone()));
            }
        }
        for (id, url) in urls {
            if let Ok(value) = self
                .rpc
                .request(&url, "eth_chainId", serde_json::json!([]))
                .await
            {
                let actual = value
                    .as_str()
                    .and_then(|s| s.strip_prefix("0x"))
                    .and_then(|s| u64::from_str_radix(s, 16).ok());
                if actual != Some(id) {
                    self.rpc.health.reject_network(&url);
                } else {
                    self.rpc.health.mark_network_verified(&url);
                }
            }
        }
        for (network, urls) in &self.ledger_providers {
            for url in urls {
                let value = if network == "solana" {
                    self.rpc
                        .request(url, "getGenesisHash", serde_json::json!([]))
                        .await
                } else {
                    self.rpc.get_json(url).await
                };
                if let Ok(value) = value {
                    let correct = match network.as_str() {
                        "solana" => value == "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
                        "stellar" => {
                            value["network_passphrase"]
                                == "Public Global Stellar Network ; September 2015"
                        }
                        "aptos" => value["chain_id"] == 1,
                        _ => false,
                    };
                    if !correct {
                        self.rpc.health.reject_network(url);
                    } else {
                        self.rpc.health.mark_network_verified(url);
                    }
                }
            }
        }
        crate::operations::event(
            "provider_probe",
            serde_json::json!({"providers":self.rpc.health.snapshot()}),
        );
    }
    pub async fn health(&self) -> serde_json::Value {
        let snapshot = self.snapshot.read().await;
        let mut rpc_chains: Vec<_> = self.config.rpc_urls.keys().copied().collect();
        rpc_chains.sort();
        let age = chrono::DateTime::parse_from_rfc3339(&snapshot.generated_at)
            .ok()
            .map(|t| {
                chrono::Utc::now()
                    .signed_duration_since(t)
                    .num_seconds()
                    .max(0) as u64
            });
        let stale = self.max_registry_age > 0 && age.is_none_or(|age| age > self.max_registry_age);
        let providers = self.rpc.health.snapshot();
        let no_rpc = snapshot.supported_chains().iter().any(|id| {
            !self
                .config
                .rpc_urls
                .get(id)
                .into_iter()
                .chain(self.rpc_backups.get(id).into_iter().flatten())
                .any(|url| self.rpc.health.ready(url))
        }) || snapshot.ledger_registry["chains"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["type"] != "evm" && c["status"] == "active")
            .any(|c| {
                c["id"].as_str().is_none_or(|id| {
                    !self
                        .ledger_providers
                        .get(id)
                        .into_iter()
                        .flatten()
                        .any(|url| self.rpc.health.ready(url))
                })
            });
        let degraded =
            *self.refresh_failure.read().await || stale || self.require_rpc_ready && no_rpc;
        serde_json::json!({"status":if degraded {"degraded"} else {"ok"},"registryRevision":snapshot.revision,"registryGeneratedAt":snapshot.generated_at,"registryAgeSeconds":age,"registryStale":stale,"supportedChains":snapshot.supported_chains(),"rpcConfiguredChains":rpc_chains,"supportedLedgerNetworks":["solana","stellar","aptos"],"ledgerConfiguredNetworks":self.ledger_providers.keys().collect::<Vec<_>>(),"providers":providers,"providerPoolsAvailable":!no_rpc,"policyVersion":self.policies.version,"registryPolicyVersion":snapshot.policies.version})
    }
}

fn setting(name: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    let value = std::env::var(name)
        .map(|s| s.parse::<u64>().map_err(|_| format!("Invalid {name}")))
        .unwrap_or(Ok(default))?;
    if value < min || value > max {
        return Err(format!("{name} out of range"));
    }
    Ok(value)
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    #[tokio::test]
    async fn stale_registry_degrades_readiness_without_discarding_identity() {
        let mut raw: serde_json::Value =
            serde_json::from_slice(include_bytes!("../fixtures/registry.json")).unwrap();
        raw["generatedAt"] = serde_json::json!("2000-01-01T00:00:00Z");
        let snapshot = Distribution::from_bytes(
            &serde_json::to_vec(&raw).unwrap(),
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/schemas"),
        )
        .unwrap();
        let mut config = Config::from_env().unwrap();
        config.rpc_urls.clear();
        let mut service = ResolverService::with_sources(
            config,
            snapshot,
            crate::policies::PolicyCatalog::empty(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        )
        .unwrap();
        service.max_registry_age = 60;
        let health = service.health().await;
        assert_eq!(health["status"], "degraded");
        assert_eq!(health["registryStale"], true);
        let result = service
            .resolve(1, "0x6a9da2d710bb9b700acde7cb81f10f1ff8c89041")
            .await
            .unwrap();
        assert!(result.product.is_some());
        assert_eq!(result.result.status, crate::types::Status::Partial);
    }
}
