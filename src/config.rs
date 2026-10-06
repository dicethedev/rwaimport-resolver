use reqwest::Url;
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, time::Duration};

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub registry_dir: PathBuf,
    pub rpc_urls: HashMap<u64, Url>,
    pub rpc_timeout: Duration,
    pub resolve_timeout: Duration,
    pub rpc_retries: usize,
    pub cache_ttl: Duration,
    pub cache_entries: usize,
    pub cache_bytes: usize,
    pub max_concurrency: usize,
    pub refresh_interval: Duration,
}
impl Config {
    pub fn from_env() -> Result<Self, String> {
        let rpc_json = match std::env::var("RPC_URLS") {
            Ok(value) => value,
            Err(_) => match std::env::var_os("RPC_URLS_FILE") {
                Some(path) => {
                    std::fs::read_to_string(path).map_err(|_| "Cannot read RPC_URLS_FILE")?
                }
                None => "{}".into(),
            },
        };
        let urls: HashMap<String, String> = serde_json::from_str(&rpc_json)
            .map_err(|_| "RPC_URLS must be a JSON object of chain IDs to URLs")?;
        let mut rpc_urls = HashMap::new();
        for (id, value) in urls {
            let id: u64 = id.parse().map_err(|_| "Invalid RPC chain ID")?;
            if id == 0 {
                return Err("RPC chain IDs must be positive".into());
            }
            let url = Url::parse(&value).map_err(|_| "Invalid RPC URL")?;
            if !["http", "https"].contains(&url.scheme())
                || url.host_str().is_none()
                || url.fragment().is_some()
            {
                return Err("RPC URLs must use HTTP(S), with no fragment".into());
            }
            rpc_urls.insert(id, url);
        }
        Ok(Self {
            bind: std::env::var("RESOLVER_BIND")
                .unwrap_or_else(|_| "127.0.0.1:3001".into())
                .parse()
                .map_err(|_| "Invalid RESOLVER_BIND")?,
            registry_dir: std::env::var_os("REGISTRY_DIST_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rwaimport-registry/dist")
                }),
            rpc_urls,
            rpc_timeout: Duration::from_millis(number("RPC_TIMEOUT_MS", 2000, 100, 30000)? as u64),
            resolve_timeout: Duration::from_millis(
                number("RESOLVE_TIMEOUT_MS", 8000, 1000, 120000)? as u64,
            ),
            rpc_retries: number("RPC_RETRIES", 1, 0, 2)?,
            cache_ttl: Duration::from_millis(
                number("RESOLVER_CACHE_TTL_MS", 15000, 0, 60000)? as u64
            ),
            cache_entries: number("RESOLVER_CACHE_MAX_ENTRIES", 256, 0, 10000)?,
            cache_bytes: number("RESOLVER_CACHE_MAX_BYTES", 8388608, 0, 67108864)?,
            max_concurrency: number("RESOLVER_MAX_CONCURRENCY", 16, 1, 256)?,
            refresh_interval: Duration::from_millis(number(
                "REGISTRY_REFRESH_INTERVAL_MS",
                60000,
                1000,
                3600000,
            )? as u64),
        })
    }
}
fn number(key: &str, default: usize, min: usize, max: usize) -> Result<usize, String> {
    let value = std::env::var(key)
        .map(|v| v.parse::<usize>().map_err(|_| format!("Invalid {key}")))
        .unwrap_or(Ok(default))?;
    if !(min..=max).contains(&value) {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(value)
}
