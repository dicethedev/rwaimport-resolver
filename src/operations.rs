//! Provider labels are hashes, never URLs or credentials.
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct ProviderHealth {
    inner: Arc<Mutex<BTreeMap<String, Provider>>>,
    threshold: u64,
    cooldown: Duration,
}
#[derive(Default)]
struct Provider {
    requests: u64,
    completed: u64,
    network_verified: bool,
    failures: u64,
    consecutive: u64,
    observation_failures: u64,
    latency_ms: u128,
    open_until: Option<Instant>,
    probe: bool,
    succeeded: bool,
}
pub struct Permit {
    health: ProviderHealth,
    key: String,
    started: Instant,
    completed: bool,
}
impl ProviderHealth {
    pub fn new(threshold: u64, cooldown: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BTreeMap::new())),
            threshold,
            cooldown,
        }
    }
    fn key(url: &Url) -> String {
        hex::encode(Sha256::digest(url.as_str().as_bytes()))[..16].to_owned()
    }
    pub fn register(&self, url: &Url) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(Self::key(url))
            .or_default();
    }
    pub fn acquire(&self, url: &Url) -> Option<Permit> {
        let key = Self::key(url);
        let mut states = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let p = states.entry(key.clone()).or_default();
        if p.open_until.is_some_and(|until| until > Instant::now()) || p.probe {
            return None;
        }
        if p.open_until.is_some() {
            p.probe = true;
        }
        p.requests += 1;
        Some(Permit {
            health: self.clone(),
            key,
            started: Instant::now(),
            completed: false,
        })
    }
    pub fn reject_network(&self, url: &Url) {
        let mut states = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let p = states.entry(Self::key(url)).or_default();
        p.open_until = Some(Instant::now() + self.cooldown);
        p.consecutive = self.threshold;
        p.network_verified = false;
    }
    pub fn mark_network_verified(&self, url: &Url) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(Self::key(url))
            .or_default()
            .network_verified = true;
    }
    pub fn ready(&self, url: &Url) -> bool {
        self.available(url)
            && self
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&Self::key(url))
                .is_some_and(|p| p.network_verified)
    }
    pub fn observation_finished(&self, url: &Url, success: bool) {
        let mut states = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = Self::key(url);
        let p = states.entry(id.clone()).or_default();
        if success {
            p.observation_failures = 0;
            p.network_verified = true;
        } else {
            p.observation_failures += 1;
            if p.observation_failures >= self.threshold {
                p.open_until = Some(Instant::now() + self.cooldown);
                event("provider_circuit_open", json!({"providerId":id}));
            }
        }
    }
    pub fn available(&self, url: &Url) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&Self::key(url))
            .is_none_or(|p| p.open_until.is_none_or(|until| until <= Instant::now()) && !p.probe)
    }
    pub fn snapshot(&self) -> Value {
        let states = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Value::Array(states.iter().map(|(id,p)|json!({"providerId":id,"state":if p.open_until.is_some_and(|until|until>Instant::now()) {"open"} else if p.open_until.is_some() {"halfOpen"} else if p.succeeded {"closed"} else {"unknown"},"requests":p.requests,"completedRequests":p.completed,"networkVerified":p.network_verified,"failures":p.failures,"consecutiveFailures":p.consecutive,"consecutiveObservationFailures":p.observation_failures,"latencyMillisecondsTotal":p.latency_ms,"cooldownRemainingMilliseconds":p.open_until.map(|until|until.saturating_duration_since(Instant::now()).as_millis()).unwrap_or(0)})).collect())
    }
    pub fn metrics(&self) -> String {
        let states = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = String::new();
        for (id, p) in states.iter() {
            out+=&format!("rwaimport_provider_requests_total{{provider=\"{id}\"}} {}\nrwaimport_provider_failures_total{{provider=\"{id}\"}} {}\nrwaimport_provider_latency_seconds_sum{{provider=\"{id}\"}} {}\nrwaimport_provider_latency_seconds_count{{provider=\"{id}\"}} {}\nrwaimport_provider_circuit_open{{provider=\"{id}\"}} {}\n",p.requests,p.failures,p.latency_ms as f64/1000.0,p.completed,u8::from(p.open_until.is_some()));
        }
        out
    }
}
impl Permit {
    pub fn finish(mut self, success: bool) {
        self.record(success);
        self.completed = true;
    }
    fn record(&self, success: bool) {
        let mut states = self.health.inner.lock().unwrap_or_else(|e| e.into_inner());
        let p = states.get_mut(&self.key).unwrap();
        p.probe = false;
        p.latency_ms += self.started.elapsed().as_millis();
        p.completed += 1;
        if success {
            p.consecutive = 0;
            p.open_until = None;
            p.succeeded = true;
        } else {
            p.failures += 1;
            p.consecutive += 1;
            if p.consecutive >= self.health.threshold {
                p.open_until = Some(Instant::now() + self.health.cooldown);
            }
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if !self.completed {
            self.record(false);
        }
    }
}
#[derive(Default)]
pub struct ResolverMetrics {
    pub requests: std::sync::atomic::AtomicU64,
    pub failures: std::sync::atomic::AtomicU64,
    pub cache_hits: std::sync::atomic::AtomicU64,
}
pub fn event(name: &str, fields: Value) {
    eprintln!(
        "{}",
        json!({"timestamp":chrono::Utc::now().to_rfc3339(),"event":name,"fields":fields})
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circuit_opens_and_allows_one_recovery_probe() {
        let h = ProviderHealth::new(1, Duration::from_millis(5));
        let u: Url = "https://example.com/private-key".parse().unwrap();
        h.acquire(&u).unwrap().finish(false);
        assert!(h.acquire(&u).is_none());
        std::thread::sleep(Duration::from_millis(8));
        let probe = h.acquire(&u).unwrap();
        assert!(h.acquire(&u).is_none());
        probe.finish(true);
        assert!(h.acquire(&u).is_some());
        assert!(!h.metrics().contains("private-key"));
    }
}
