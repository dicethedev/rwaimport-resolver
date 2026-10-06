//! Read-only non-EVM adapters. Ledger observations never imply legal or standard conformance.
use crate::{errors::ResolveError, registry::distribution::Distribution, rpc::live::LiveRpc};
use reqwest::Url;
use serde_json::{json, Value};
use std::collections::HashMap;

const TOKEN: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub fn validate_input(
    network: &str,
    address: &str,
    code: Option<&str>,
) -> Result<(), ResolveError> {
    let valid = match network {
        "solana" => {
            (32..=44).contains(&address.len())
                && address.bytes().all(|b| {
                    b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(&b)
                })
                && code.is_none()
        }
        "stellar" => {
            address.len() == 56
                && address.starts_with('G')
                && address
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b))
                && code.is_some_and(|c| {
                    (1..=12).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_alphanumeric())
                })
        }
        "aptos" => {
            address.len() == 66
                && address.starts_with("0x")
                && address[2..].bytes().all(|b| b.is_ascii_hexdigit())
                && code.is_none()
        }
        _ => return Err(ResolveError::UnsupportedChain),
    };
    if valid {
        Ok(())
    } else {
        Err(ResolveError::InvalidAddress)
    }
}
pub fn providers() -> Result<HashMap<String, Vec<Url>>, String> {
    let raw = std::env::var("LEDGER_URLS")
        .unwrap_or_else(|_| include_str!("../config/public-ledger.json").into());
    let values: HashMap<String, Vec<String>> =
        serde_json::from_str(&raw).map_err(|_| "Invalid LEDGER_URLS")?;
    values
        .into_iter()
        .map(|(network, urls)| {
            if !["solana", "stellar", "aptos"].contains(&network.as_str()) || urls.is_empty() {
                return Err("Invalid ledger provider network or empty list".into());
            }
            let urls = urls
                .into_iter()
                .map(|s| {
                    let url = Url::parse(&s).map_err(|_| "Invalid ledger URL")?;
                    if !["http", "https"].contains(&url.scheme())
                        || url.host_str().is_none()
                        || url.query().is_some()
                        || url.fragment().is_some()
                    {
                        return Err(
                            "Ledger URLs must be HTTP(S) base URLs without queries or fragments"
                                .into(),
                        );
                    }
                    Ok(url)
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok((network, urls))
        })
        .collect()
}
pub fn evm_backups() -> Result<HashMap<u64, Vec<Url>>, String> {
    let raw = std::env::var("RPC_BACKUP_URLS").unwrap_or_else(|_| "{}".into());
    let values: HashMap<String, Vec<String>> =
        serde_json::from_str(&raw).map_err(|_| "Invalid RPC_BACKUP_URLS")?;
    values
        .into_iter()
        .map(|(id, urls)| {
            let id: u64 = id.parse().map_err(|_| "Invalid backup chain ID")?;
            if id == 0 {
                return Err("Backup chain ID must be positive".into());
            }
            let urls = urls
                .into_iter()
                .map(|s| {
                    let url = Url::parse(&s).map_err(|_| "Invalid backup RPC URL")?;
                    if !["http", "https"].contains(&url.scheme())
                        || url.host_str().is_none()
                        || url.fragment().is_some()
                    {
                        return Err("Invalid backup HTTP(S) URL".into());
                    }
                    Ok(url)
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok((id, urls))
        })
        .collect()
}
fn check(field: &str, expected: Value, actual: Value) -> Value {
    let status = if actual.is_null() {
        "unavailable"
    } else if expected == actual {
        "verified"
    } else {
        "mismatched"
    };
    json!({"field":field,"expected":expected,"actual":actual,"status":status})
}
fn child(base: &Url, path: &str) -> Result<Url, ResolveError> {
    let mut base = base.clone();
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(path).map_err(|_| ResolveError::RpcUnavailable)
}
async fn observe(
    rpc: &LiveRpc,
    url: &Url,
    network: &str,
    address: &str,
    code: Option<&str>,
) -> Result<Value, ResolveError> {
    let unavailable = |_| ResolveError::RpcUnavailable;
    match network {
        "solana" => {
            let genesis = rpc
                .request(url, "getGenesisHash", json!([]))
                .await
                .map_err(unavailable)?;
            if genesis != "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d" {
                return Err(ResolveError::RpcChainMismatch);
            }
            let response = rpc
                .request(
                    url,
                    "getAccountInfo",
                    json!([address,{"encoding":"jsonParsed","commitment":"finalized"}]),
                )
                .await
                .map_err(unavailable)?;
            let slot = response["context"]["slot"]
                .as_u64()
                .ok_or(ResolveError::RpcUnavailable)?;
            let account = response.get("value").ok_or(ResolveError::RpcUnavailable)?;
            if account.is_null() {
                return Ok(json!({"exists":false,"slot":slot,"commitment":"finalized"}));
            }
            let owner = account["owner"]
                .as_str()
                .ok_or(ResolveError::RpcUnavailable)?;
            let mint = account["data"]["parsed"]["type"] == "mint"
                && [TOKEN, TOKEN_2022].contains(&owner)
                && account["executable"] == false
                && account["data"]["parsed"]["info"]["isInitialized"] == true;
            let info = &account["data"]["parsed"]["info"];
            Ok(
                json!({"exists":true,"slot":slot,"commitment":"finalized","owner":owner,"mint":mint,"decimals":if mint {info["decimals"].clone()} else {Value::Null},"supply":if mint {info["supply"].clone()} else {Value::Null},"mintAuthority":info["mintAuthority"],"freezeAuthority":info["freezeAuthority"]}),
            )
        }
        "stellar" => {
            let root = rpc.get_json(url).await.map_err(unavailable)?;
            if root["network_passphrase"] != "Public Global Stellar Network ; September 2015" {
                return Err(ResolveError::RpcChainMismatch);
            }
            let ledger = root["history_latest_ledger"]
                .as_u64()
                .ok_or(ResolveError::RpcUnavailable)?;
            let mut assets = child(url, "assets")?;
            assets
                .query_pairs_mut()
                .append_pair("asset_code", code.unwrap())
                .append_pair("asset_issuer", address)
                .append_pair("limit", "1");
            let response = rpc.get_json(&assets).await.map_err(unavailable)?;
            let records = response["_embedded"]["records"]
                .as_array()
                .ok_or(ResolveError::RpcUnavailable)?;
            let record = records.first();
            if record
                .is_some_and(|r| r["asset_issuer"] != address || r["asset_code"] != code.unwrap())
            {
                return Err(ResolveError::RpcUnavailable);
            }
            Ok(
                json!({"exists":record.is_some(),"ledger":ledger,"consistency":"Horizon indexed state; reads are not pinned to a ledger","symbol":record.map(|r|r["asset_code"].clone()),"issuer":record.map(|r|r["asset_issuer"].clone()),"flags":record.map(|r|r["flags"].clone())}),
            )
        }
        "aptos" => {
            let root = rpc.get_json(url).await.map_err(unavailable)?;
            if root["chain_id"] != 1 {
                return Err(ResolveError::RpcChainMismatch);
            }
            let version = root["ledger_version"]
                .as_str()
                .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                .ok_or(ResolveError::RpcUnavailable)?;
            let mut resource_url = child(url, &format!("accounts/{address}/resources"))?;
            resource_url
                .query_pairs_mut()
                .append_pair("ledger_version", version);
            let resources = rpc.get_json(&resource_url).await.map_err(unavailable)?;
            let resources = resources.as_array().ok_or(ResolveError::RpcUnavailable)?;
            let metadata = resources
                .iter()
                .find(|r| r["type"] == "0x1::fungible_asset::Metadata");
            let data = metadata.map(|r| &r["data"]);
            let owner = resources
                .iter()
                .find(|r| r["type"] == "0x1::object::ObjectCore")
                .map(|r| r["data"]["owner"].clone());
            Ok(
                json!({"exists":!resources.is_empty(),"ledgerVersion":version,"fungibleAsset":metadata.is_some(),"name":data.map(|d|d["name"].clone()),"symbol":data.map(|d|d["symbol"].clone()),"decimals":data.map(|d|d["decimals"].clone()),"objectOwner":owner}),
            )
        }
        _ => Err(ResolveError::UnsupportedChain),
    }
}
pub async fn resolve(
    rpc: &LiveRpc,
    providers: &HashMap<String, Vec<Url>>,
    snapshot: &Distribution,
    locator: (&str, &str, Option<&str>),
    degraded: bool,
    budget: std::time::Duration,
) -> Result<Value, ResolveError> {
    let (network, address, code) = locator;
    validate_input(network, address, code)?;
    let deadline = tokio::time::Instant::now() + budget;
    let registry = &snapshot.ledger_registry;
    let network_record = registry["chains"]
        .as_array()
        .and_then(|chains| chains.iter().find(|c| c["id"] == network))
        .ok_or(ResolveError::UnsupportedChain)?;
    let expected_reference = match network {
        "solana" => "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
        "stellar" => "pubnet",
        "aptos" => "1",
        _ => return Err(ResolveError::UnsupportedChain),
    };
    if network_record["namespace"] != network || network_record["reference"] != expected_reference {
        return Err(ResolveError::UnsupportedChain);
    }
    let mut matches = vec![];
    for bundle in registry["assets"].as_array().unwrap() {
        for dep in bundle["deployments"].as_array().unwrap() {
            let same_address = dep["address"].as_str().is_some_and(|a| {
                if network == "aptos" {
                    a.eq_ignore_ascii_case(address)
                } else {
                    a == address
                }
            });
            let same_code =
                network != "stellar" || dep["verification"]["observedSymbol"].as_str() == code;
            if dep["chain"] == network && same_address && same_code {
                matches.push((bundle, dep));
            }
        }
    }
    if matches.len() > 1 {
        return Err(ResolveError::RpcUnavailable);
    }
    let context = matches.first().copied();
    let mut observation = Value::Null;
    let mut warnings = vec![];
    if degraded {
        warnings.push("Registry refresh failed; using last valid revision");
    }
    let urls = providers
        .get(network)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (index, url) in urls.iter().enumerate() {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let attempt = remaining / (urls.len() - index) as u32;
        match tokio::time::timeout(attempt, observe(rpc, url, network, address, code))
            .await
            .unwrap_or(Err(ResolveError::RpcUnavailable))
        {
            Ok(value) => {
                observation = value;
                break;
            }
            Err(ResolveError::RpcChainMismatch) => return Err(ResolveError::RpcChainMismatch),
            Err(_) => {
                warnings.push("Provider observation unavailable; trying next configured provider")
            }
        }
    }
    if observation.is_null() {
        warnings.push("Live ledger observation unavailable");
    }
    let mut checks = vec![];
    if let Some((_, dep)) = context {
        checks.push(check("exists", json!(true), observation["exists"].clone()));
        let v = &dep["verification"];
        for (field, expected) in [
            ("name", v["observedName"].clone()),
            ("symbol", v["observedSymbol"].clone()),
            (
                "decimals",
                dep.get("decimals")
                    .unwrap_or(&v["observedDecimals"])
                    .clone(),
            ),
        ] {
            if !expected.is_null() {
                checks.push(check(field, expected, observation[field].clone()));
            }
        }
        if network == "solana" {
            checks.push(check("mint", json!(true), observation["mint"].clone()));
            if !v["owner"].is_null() {
                checks.push(check(
                    "owner",
                    v["owner"].clone(),
                    observation["owner"].clone(),
                ));
            }
            let standards = dep["standardIds"].as_array().unwrap();
            if standards.contains(&json!("solana-token-2022")) {
                checks.push(check(
                    "tokenProgram",
                    json!(TOKEN_2022),
                    observation["owner"].clone(),
                ));
            }
        }
        if network == "stellar" {
            checks.push(check(
                "issuer",
                json!(address),
                observation["issuer"].clone(),
            ));
        }
        if network == "aptos"
            && dep["standardIds"]
                .as_array()
                .unwrap()
                .contains(&json!("aptos-fungible-asset"))
        {
            checks.push(check(
                "fungibleAsset",
                json!(true),
                observation["fungibleAsset"].clone(),
            ));
        }
    }
    let status = if context.is_none() {
        "UNKNOWN"
    } else if checks.iter().any(|c| c["status"] == "mismatched") {
        "MISMATCH"
    } else if checks.len() > 1 && checks.iter().all(|c| c["status"] == "verified") {
        "VERIFIED"
    } else {
        "PARTIAL"
    };
    let lookup = |collection: &str, id: &Value| {
        registry[collection]
            .as_array()
            .and_then(|records| records.iter().find(|r| r["id"] == *id))
            .cloned()
    };
    let issuer = context.and_then(|(b, _)| lookup("issuers", &b["asset"]["issuerId"]));
    let underlying = context.and_then(|(b, _)| lookup("underlyings", &b["asset"]["underlyingId"]));
    let evidence = context.map(|(b,_)|json!({"sources":b["sources"],"claims":b["claims"],"underlyingSources":underlying.as_ref().map(|u|&u["sources"]),"underlyingClaims":underlying.as_ref().map(|u|&u["claims"])}));
    let standards: Vec<_> = context
        .into_iter()
        .flat_map(|(_, d)| d["standardIds"].as_array().unwrap())
        .filter_map(|id| lookup("standards", id))
        .collect();
    let organizations: Vec<_> = context.into_iter().flat_map(|(b,_)|b["asset"]["organizationRoles"].as_array().unwrap()).map(|role|json!({"organization":lookup("organizations",&role["organizationId"]),"roles":role["roles"]})).collect();
    Ok(json!({
        "status":status,
        "input":{"network":network,"address":address,"assetCode":code},
        "registryRevision":snapshot.revision,
        "registryGeneratedAt":snapshot.generated_at,
        "resolvedAt":chrono::Utc::now().to_rfc3339(),
        "product":context.map(|(b,_)|&b["asset"]),
        "issuer":issuer,"underlyingAsset":underlying,"standards":standards,"organizations":organizations,
        "compliance":context.map(|(b,_)|&b["compliance"]),"valuation":context.map(|(b,_)|&b["valuation"]),
        "deployment":context.map(|(_,d)|d),"network":network_record,"evidence":evidence,
        "observation":observation,"checks":checks,"warnings":warnings,
        "evidenceFreshness":crate::freshness::summary(evidence.as_ref(),context.map(|(_,d)|d)),
        "verificationScope":"Selected ledger identity and recorded metadata claims; mutable account hashes, full standard conformance, legal rights and reserves are not verified"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        routing::{get, post},
        Json, Router,
    };
    use std::sync::Arc;
    async fn mock(State(mode): State<Arc<String>>, Json(body): Json<Value>) -> Json<Value> {
        let result = match body["method"].as_str().unwrap() {
            "getGenesisHash" => json!(if mode.as_str() == "wrong" {
                "devnet"
            } else {
                "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
            }),
            _ => {
                json!({"context":{"slot":123},"value":{"owner":TOKEN_2022,"executable":false,"data":{"parsed":{"type":if mode.as_str()=="notmint" {"account"} else {"mint"},"info":{"decimals":9,"isInitialized":true,"supply":"10000000000000000000","mintAuthority":null,"freezeAuthority":null}}}}})
            }
        };
        Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
    }
    async fn server(mode: &str) -> (Url, tokio::task::JoinHandle<()>) {
        let mode = Arc::new(mode.to_owned());
        let app = Router::new().route("/",post(mock).get(||async {Json(json!({"network_passphrase":"Public Global Stellar Network ; September 2015","history_latest_ledger":123,"chain_id":1,"ledger_version":"456"}))})).route("/assets",get(|| async {Json(json!({"_embedded":{"records":[{"asset_issuer":"GBHNGLLIE3KWGKCHIKMHJ5HVZHYIK7WTBE4QF5PLAKL4CJGSEU7HZIW5","asset_code":"BENJI"}]}}))})).route("/accounts/{address}/resources",get(||async {Json(json!([{"type":"0x1::fungible_asset::Metadata","data":{"name":"Benji","symbol":"BENJI","decimals":6}}]))})).with_state(mode);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, task)
    }
    fn rpc() -> LiveRpc {
        LiveRpc::new(&crate::config::Config::from_env().unwrap()).unwrap()
    }
    #[test]
    fn validates_ledger_locators() {
        assert!(validate_input(
            "stellar",
            "GBHNGLLIE3KWGKCHIKMHJ5HVZHYIK7WTBE4QF5PLAKL4CJGSEU7HZIW5",
            None
        )
        .is_err());
        assert!(validate_input("solana", "0x0000", None).is_err());
        assert!(validate_input("aptos", &format!("0x{}", "a".repeat(64)), None).is_ok());
        assert!(validate_input(
            "stellar",
            "GBHNGLLIE3KWGKCHIKMHJ5HVZHYIK7WTBE4QF5PLAKL4CJGSEU7HZIW5",
            Some("BENJI")
        )
        .is_ok());
    }
    #[tokio::test]
    async fn observes_each_ledger_and_rejects_wrong_network() {
        let (url, task) = server("ok").await;
        let rpc = rpc();
        let s = observe(&rpc, &url, "solana", "mint", None).await.unwrap();
        assert_eq!(s["mint"], true);
        assert_eq!(s["decimals"], 9);
        assert_eq!(s["slot"], 123);
        let a = observe(&rpc, &url, "aptos", "0x123", None).await.unwrap();
        assert_eq!(a["fungibleAsset"], true);
        assert_eq!(a["ledgerVersion"], "456");
        let t = observe(
            &rpc,
            &url,
            "stellar",
            "GBHNGLLIE3KWGKCHIKMHJ5HVZHYIK7WTBE4QF5PLAKL4CJGSEU7HZIW5",
            Some("BENJI"),
        )
        .await
        .unwrap();
        assert_eq!(t["symbol"], "BENJI");
        assert!(observe(&rpc, &url, "stellar", "OTHER", Some("BENJI"))
            .await
            .is_err());
        task.abort();
        let (url, task) = server("wrong").await;
        assert_eq!(
            observe(&rpc, &url, "solana", "mint", None)
                .await
                .unwrap_err(),
            ResolveError::RpcChainMismatch
        );
        task.abort();
        let (url, task) = server("notmint").await;
        assert_eq!(
            observe(&rpc, &url, "solana", "mint", None).await.unwrap()["mint"],
            false
        );
        task.abort();
    }
    #[tokio::test]
    async fn failover_preserves_identity_and_never_hides_mismatches() {
        let (url, task) = server("ok").await;
        let rpc = rpc();
        let raw = include_bytes!("../fixtures/registry.json");
        let snapshot = Distribution::from_bytes(
            raw,
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/schemas"),
        )
        .unwrap();
        let address = "GyWgeqpy5GueU2YbkE8xqUeVEokCMMCEeUrfbtMw6phr";
        let providers = HashMap::from([(
            "solana".to_string(),
            vec!["http://127.0.0.1:1/".parse().unwrap(), url],
        )]);
        let result = resolve(
            &rpc,
            &providers,
            &snapshot,
            ("solana", address, None),
            false,
            std::time::Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "MISMATCH"); // fixture records six decimals; provider returns nine.
        assert_eq!(result["product"]["id"], "buidl");
        assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
        assert_eq!(result["observation"]["slot"], 123);
        let unavailable = resolve(
            &rpc,
            &HashMap::new(),
            &snapshot,
            ("solana", address, None),
            true,
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(unavailable["status"], "PARTIAL");
        assert_eq!(unavailable["product"]["id"], "buidl");
        let unknown = resolve(
            &rpc,
            &HashMap::new(),
            &snapshot,
            ("solana", "11111111111111111111111111111111", None),
            false,
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(unknown["status"], "UNKNOWN");
        task.abort();
    }
}
