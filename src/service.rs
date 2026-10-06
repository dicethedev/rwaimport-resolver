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
}
impl ResolverService {
    pub fn new(config: Config, snapshot: Distribution) -> Result<Self, String> {
        let rpc = LiveRpc::new(&config)?;
        let cache =
            ResolutionCache::new(config.cache_ttl, config.cache_entries, config.cache_bytes);
        let slots = Arc::new(Semaphore::new(config.max_concurrency));
        Ok(Self {
            config,
            snapshot: RwLock::new(Arc::new(snapshot)),
            rpc,
            ledger_providers: crate::ledgers::providers()?,
            rpc_backups: crate::ledgers::evm_backups()?,
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
                return Ok(value);
            }
        }
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
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let attempt_deadline =
                tokio::time::Instant::now() + remaining / (urls.len() - index) as u32;
            match tokio::time::timeout_at(attempt_deadline, self.rpc.observe(url,&input,&standards)).await {
                Ok(Ok(value)) => { observed=Some(value); break; },
                Ok(Err(ResolveError::RpcChainMismatch)) => return Err(ResolveError::RpcChainMismatch),
                _ => warnings.push("Live RPC observation unavailable; trying next provider within request deadline".into()),
            }
        }
        let (status, checks) = verify(matched.as_ref(), observed.as_ref());
        // Optional reads do not suppress caching when all selected claims are available.
        let cacheable = !degraded
            && observed.is_some()
            && checks
                .iter()
                .all(|c| c.status != crate::types::CheckStatus::Unavailable);
        let resolution = Resolution {
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
    ) -> Result<serde_json::Value, ResolveError> {
        crate::ledgers::validate_input(network, address, code)?;
        let _permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ResolveError::Busy)?;
        let snapshot = self.snapshot.read().await.clone();
        let degraded = *self.refresh_failure.read().await;
        tokio::time::timeout(
            self.config.resolve_timeout,
            crate::ledgers::resolve(
                &self.rpc,
                &self.ledger_providers,
                &snapshot,
                (network, address, code),
                degraded,
                self.config
                    .resolve_timeout
                    .saturating_sub(std::time::Duration::from_millis(50)),
            ),
        )
        .await
        .map_err(|_| ResolveError::Busy)?
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
                *self.refresh_failure.write().await = false;
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
    pub async fn health(&self) -> serde_json::Value {
        let snapshot = self.snapshot.read().await;
        let mut rpc_chains: Vec<_> = self.config.rpc_urls.keys().copied().collect();
        rpc_chains.sort();
        serde_json::json!({"status": if *self.refresh_failure.read().await { "degraded" } else { "ok" }, "registryRevision": snapshot.revision, "registryGeneratedAt": snapshot.generated_at, "supportedChains": snapshot.supported_chains(), "rpcConfiguredChains": rpc_chains, "supportedLedgerNetworks": ["solana","stellar","aptos"], "ledgerConfiguredNetworks": self.ledger_providers.keys().collect::<Vec<_>>()})
    }
}
