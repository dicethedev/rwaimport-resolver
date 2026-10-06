use crate::{input::ResolveInput, types::Resolution};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub input: ResolveInput,
    pub revision: String,
}
struct Entry<V> {
    value: V,
    expires: Instant,
    size: usize,
}
pub struct ResolutionCache<K = CacheKey, V = Resolution> {
    entries: HashMap<K, Entry<V>>,
    ttl: Duration,
    max_entries: usize,
    max_bytes: usize,
    bytes: usize,
}
impl<K: std::hash::Hash + Eq + Clone, V: serde::Serialize + Clone> ResolutionCache<K, V> {
    pub fn new(ttl: Duration, max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
            max_entries,
            max_bytes,
            bytes: 0,
        }
    }
    pub fn get(&mut self, key: &K) -> Option<V> {
        self.expire();
        self.entries.get(key).map(|v| v.value.clone())
    }
    pub fn insert(&mut self, key: K, value: V) {
        let size = serde_json::to_vec(&value)
            .map(|v| v.len())
            .unwrap_or(usize::MAX)
            + std::mem::size_of_val(&key);
        if self.ttl.is_zero() || self.max_entries == 0 || size > self.max_bytes {
            return;
        }
        self.expire();
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= old.size;
        }
        while self.entries.len() >= self.max_entries || self.bytes + size > self.max_bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.expires)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                if let Some(old) = self.entries.remove(&oldest) {
                    self.bytes -= old.size;
                }
            } else {
                break;
            }
        }
        self.bytes += size;
        self.entries.insert(
            key,
            Entry {
                value,
                size,
                expires: Instant::now() + self.ttl,
            },
        );
    }
    fn expire(&mut self) {
        let now = Instant::now();
        let mut removed = 0;
        self.entries.retain(|_, entry| {
            if entry.expires <= now {
                removed += entry.size;
                false
            } else {
                true
            }
        });
        self.bytes -= removed;
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}
