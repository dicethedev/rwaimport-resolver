use axum::{extract::State, routing::post, Json, Router};
use http_body_util::BodyExt;
use rwaimport_resolver::{
    config::Config, contracts::abi, http::router, registry::distribution::Distribution,
    service::ResolverService, types::Status,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;

const ADDRESS: &str = "0x6a9da2d710bb9b700acde7cb81f10f1ff8c89041";
const IMPLEMENTATION: &str = "0x9e2693f54831f6f52b0bb952c2935d26919a3626";
const CODE: &str = "0x60006000";
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}
fn fixture() -> Value {
    serde_json::from_slice(include_bytes!("../fixtures/registry.json")).unwrap()
}
fn distribution(mut data: Value) -> Distribution {
    use sha2::{Digest, Sha256};
    data["assets"][0]["deployments"][0]["verification"]["runtimeCodeSha256"] = json!(hex::encode(
        Sha256::digest(hex::decode(&CODE[2..]).unwrap())
    ));
    Distribution::from_bytes(
        &serde_json::to_vec(&data).unwrap(),
        &fixture_dir().join("schemas"),
    )
    .unwrap()
}
fn config(url: Option<String>) -> Config {
    Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        registry_dir: fixture_dir(),
        rpc_urls: url
            .map(|v| HashMap::from([(1, v.parse().unwrap())]))
            .unwrap_or_default(),
        rpc_timeout: Duration::from_millis(200),
        resolve_timeout: Duration::from_secs(2),
        rpc_retries: 1,
        cache_ttl: Duration::from_secs(10),
        cache_entries: 4,
        cache_bytes: 1024 * 1024,
        max_concurrency: 4,
        refresh_interval: Duration::from_secs(60),
    }
}
fn word(n: u64) -> String {
    format!("0x{n:064x}")
}
fn text(s: &str) -> String {
    format!(
        "0x{:064x}{:064x}{}{}",
        32,
        s.len(),
        hex::encode(s),
        "0".repeat((32 - s.len() % 32) % 32 * 2)
    )
}
#[derive(Clone)]
struct Mock {
    mode: &'static str,
    calls: Arc<Mutex<Vec<Value>>>,
}
async fn rpc(State(mock): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
    mock.calls.lock().unwrap().push(body.clone());
    let method = body["method"].as_str().unwrap();
    if mock.mode == "retry" && method == "eth_chainId" && mock.calls.lock().unwrap().len() == 1 {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let result = match method {
        "eth_chainId" => json!(if mock.mode == "wrong-chain" {
            "0x89"
        } else {
            "0x1"
        }),
        "eth_getBlockByNumber" => {
            json!({"number": "0x7b", "hash": format!("0x{}", if mock.mode == "reorg" && body["params"][0] != "latest" { "bb".repeat(32) } else { "aa".repeat(32) })})
        }
        "eth_getCode" => json!(if mock.mode == "no-code" { "0x" } else { CODE }),
        "eth_getStorageAt" => json!(if body["params"][1]
            == rwaimport_resolver::proxy::IMPLEMENTATION_SLOT
        {
            format!("0x{}{}", "0".repeat(24), &IMPLEMENTATION[2..])
        } else {
            word(0)
        }),
        "eth_call" => {
            let selector = body["params"][0]["data"].as_str().unwrap();
            if selector == abi::selector("symbol()") {
                json!(text(if mock.mode == "mismatch" {
                    "WRONG"
                } else {
                    "BUIDL-I"
                }))
            } else if selector == abi::selector("name()") {
                json!(text(
                    "BlackRock USD Institutional Digital Liquidity Fund - I Class"
                ))
            } else if selector == abi::selector("decimals()") && mock.mode != "revert" {
                json!(word(6))
            } else if selector == abi::selector("totalSupply()") {
                json!(format!("0x{}", "ff".repeat(32)))
            } else if selector == abi::selector("owner()") {
                json!(format!("0x{}{}", "0".repeat(24), &IMPLEMENTATION[2..]))
            } else if mock.mode == "policy" && selector == abi::selector("paused()") {
                json!(word(0))
            } else if mock.mode == "policy"
                && selector.starts_with(&abi::selector("hasRole(bytes32,address)"))
            {
                json!(word(1))
            } else if mock.mode == "policy"
                && selector.starts_with(&abi::selector(
                    "detectTransferRestriction(address,address,uint256)",
                ))
            {
                json!(word(2))
            } else if mock.mode == "policy"
                && [
                    "asset()",
                    "identityRegistry()",
                    "compliance()",
                    "ruleEngine()",
                ]
                .iter()
                .any(|sig| selector == abi::selector(sig))
            {
                json!(format!("0x{}{}", "0".repeat(24), &IMPLEMENTATION[2..]))
            } else if mock.mode == "policy" && selector == abi::selector("VERSION()") {
                json!(text("3.0.0"))
            } else if mock.mode == "policy"
                && (selector == abi::selector("totalAssets()")
                    || [
                        "convertToAssets(uint256)",
                        "convertToShares(uint256)",
                        "previewDeposit(uint256)",
                        "previewRedeem(uint256)",
                    ]
                    .iter()
                    .any(|sig| selector.starts_with(&abi::selector(sig))))
            {
                json!(word(0))
            } else if selector.starts_with(&abi::selector("supportsInterface(bytes4)")) {
                json!(word(u64::from(selector[10..].starts_with("01ffc9a7"))))
            } else {
                return Json(
                    json!({"jsonrpc": "2.0", "id": body["id"], "error": {"code": -32000, "message": "execution reverted"}}),
                );
            }
        }
        _ => panic!("unexpected method"),
    };
    Json(
        json!({"jsonrpc": "2.0", "id": if mock.mode == "wrong-id" { json!(0) } else { body["id"].clone() }, "result": result}),
    )
}
async fn mock_rpc(mode: &'static str) -> (String, Mock, tokio::task::JoinHandle<()>) {
    let state = Mock {
        mode,
        calls: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/", post(rpc))
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, state, task)
}
#[tokio::test]
async fn live_rpc_matches_deployment_metadata_and_cache_coalesces_requests() {
    let (url, mock, task) = mock_rpc("ok").await;
    let service = ResolverService::new(config(Some(url)), distribution(fixture())).unwrap();
    let mixed_address = ADDRESS.to_uppercase().replacen("0X", "0x", 1);
    let (a, b) = tokio::join!(
        service.resolve(1, ADDRESS),
        service.resolve(1, &mixed_address)
    );
    assert_eq!(a.unwrap().result.status, Status::Verified);
    assert_eq!(b.unwrap().result.status, Status::Verified);
    let count = mock.calls.lock().unwrap().len();
    let result = service.resolve(1, ADDRESS).await.unwrap();
    assert_eq!(count, mock.calls.lock().unwrap().len());
    assert_eq!(result.product.unwrap()["symbol"], "BUIDL");
    assert_eq!(result.valuation.unwrap()["valuationType"], "fixed");
    assert_eq!(result.network.unwrap()["chainId"], 1);
    assert!(result
        .organizations
        .iter()
        .any(|v| v["organization"]["id"] == "securitize"));
    assert_eq!(result.standards[0]["id"], "erc20");
    let contract = result.result.contract.unwrap();
    assert_eq!(contract.symbol.as_deref(), Some("BUIDL-I"));
    assert_eq!(
        contract.total_supply.as_deref(),
        Some("115792089237316195423570985008687907853269984665640564039457584007913129639935")
    );
    assert_eq!(contract.capabilities["erc165"], Some(true));
    for call in mock.calls.lock().unwrap().iter() {
        if ["eth_getCode", "eth_call", "eth_getStorageAt"]
            .contains(&call["method"].as_str().unwrap())
        {
            let block = call["params"].as_array().unwrap().last().unwrap();
            assert_eq!(block["blockHash"], format!("0x{}", "aa".repeat(32)));
            assert_eq!(block["requireCanonical"], true);
        }
    }
    task.abort();
}
#[tokio::test]
async fn mismatches_reverts_missing_code_and_provider_errors_remain_distinct() {
    for (mode, expected) in [
        ("mismatch", Status::Mismatch),
        ("revert", Status::Partial),
        ("no-code", Status::Mismatch),
        ("wrong-id", Status::Partial),
        ("reorg", Status::Partial),
        ("retry", Status::Verified),
    ] {
        let (url, _, task) = mock_rpc(mode).await;
        let service = ResolverService::new(config(Some(url)), distribution(fixture())).unwrap();
        let result = service.resolve(1, ADDRESS).await.unwrap();
        assert_eq!(result.result.status, expected, "{mode}");
        task.abort();
    }
    let (url, _, task) = mock_rpc("wrong-chain").await;
    let service = ResolverService::new(config(Some(url)), distribution(fixture())).unwrap();
    assert!(matches!(
        service.resolve(1, ADDRESS).await,
        Err(rwaimport_resolver::errors::ResolveError::RpcChainMismatch)
    ));
    task.abort();
}
#[tokio::test]
async fn http_returns_identity_without_rpc_and_rejects_invalid_inputs() {
    let service = Arc::new(ResolverService::new(config(None), distribution(fixture())).unwrap());
    let app = router(service);
    for (path, status) in [
        (format!("/v1/resolve/1/{ADDRESS}"), 200),
        (format!("/v1/resolve/0/{ADDRESS}"), 400),
        ("/v1/resolve/1/0x123".into(), 400),
        (format!("/v1/resolve/2/{ADDRESS}"), 400),
        (format!("/v1/resolve/1/{ADDRESS}?extra=1"), 400),
    ] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(&path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        if status == 200 {
            assert_eq!(body["data"]["status"], "PARTIAL");
            assert_eq!(body["data"]["issuer"]["id"], "blackrock");
            assert!(body["data"]["contract"].is_null());
        }
    }
}
#[test]
fn registry_validation_rejects_corrupt_references_duplicate_contracts_and_wrong_chain_ids() {
    for mode in ["schema", "duplicate", "chain", "issuer", "source"] {
        let mut data = fixture();
        match mode {
            "schema" => data["assets"][0]["asset"]["instrumentType"] = json!("not-a-type"),
            "duplicate" => {
                let copy = data["assets"][0]["deployments"][0].clone();
                data["assets"][0]["deployments"]
                    .as_array_mut()
                    .unwrap()
                    .push(copy);
            }
            "chain" => data["assets"][0]["deployments"][0]["chainId"] = json!(2),
            "issuer" => data["assets"][0]["asset"]["issuerId"] = json!("missing"),
            "source" => data["assets"][0]["claims"][0]["sourceIds"] = json!(["missing"]),
            _ => unreachable!(),
        }
        assert!(
            Distribution::from_bytes(
                &serde_json::to_vec(&data).unwrap(),
                &fixture_dir().join("schemas")
            )
            .is_err(),
            "{mode}"
        );
    }
}
#[test]
fn abi_rejects_malformed_values_and_supports_bytes32_strings() {
    assert_eq!(abi::selector("symbol()"), "0x95d89b41");
    assert_eq!(
        abi::text(&format!("0x{}{}", hex::encode("ABC"), "0".repeat(58))).as_deref(),
        Some("ABC")
    );
    assert_eq!(abi::text("0x"), None);
    assert_eq!(abi::boolean(&word(2)), None);
    assert_eq!(abi::small_uint(&format!("0x{}", "ff".repeat(32))), None);
    assert_eq!(abi::text(&format!("0x{:064x}{:064x}", 32, u64::MAX)), None);
}
#[test]
fn detects_minimal_proxy_without_mistaking_arbitrary_bytecode_for_proxy() {
    let code = hex::decode(format!(
        "363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
        &IMPLEMENTATION[2..]
    ))
    .unwrap();
    assert_eq!(
        rwaimport_resolver::proxy::minimal_proxy(&code).as_deref(),
        Some(IMPLEMENTATION)
    );
    assert!(rwaimport_resolver::proxy::minimal_proxy(&[0; 45]).is_none());
}

#[tokio::test]
async fn unknown_contracts_can_have_live_observations_without_becoming_identified() {
    let (url, _, task) = mock_rpc("ok").await;
    let service = ResolverService::new(config(Some(url)), distribution(fixture())).unwrap();
    let result = service
        .resolve(1, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        .await
        .unwrap();
    assert_eq!(result.result.status, Status::Unknown);
    assert!(result.result.contract.is_some());
    assert!(result.product.is_none());
    assert!(result.result.checks.is_empty());
    task.abort();
}
#[tokio::test]
async fn unavailable_critical_reads_are_not_cached_and_cache_expiry_rechecks_rpc() {
    let (url, mock, task) = mock_rpc("revert").await;
    let service = ResolverService::new(config(Some(url)), distribution(fixture())).unwrap();
    service.resolve(1, ADDRESS).await.unwrap();
    let count = mock.calls.lock().unwrap().len();
    service.resolve(1, ADDRESS).await.unwrap();
    assert!(mock.calls.lock().unwrap().len() > count);
    task.abort();
    let (url, mock, task) = mock_rpc("ok").await;
    let mut settings = config(Some(url));
    settings.cache_ttl = Duration::from_millis(10);
    let service = ResolverService::new(settings, distribution(fixture())).unwrap();
    service.resolve(1, ADDRESS).await.unwrap();
    let count = mock.calls.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(20)).await;
    service.resolve(1, ADDRESS).await.unwrap();
    assert!(mock.calls.lock().unwrap().len() > count);
    task.abort();
}
#[tokio::test]
async fn registry_refresh_retains_valid_identity_and_recovers_atomically() {
    let directory =
        std::env::temp_dir().join(format!("rwaimport-resolver-refresh-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("dist")).unwrap();
    std::fs::create_dir_all(directory.join("schemas")).unwrap();
    for entry in std::fs::read_dir(fixture_dir().join("schemas")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(
            entry.path(),
            directory.join("schemas").join(entry.file_name()),
        )
        .unwrap();
    }
    let mut data = fixture();
    let path = directory.join("dist/registry.json");
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    let mut settings = config(None);
    settings.registry_dir = directory.join("dist");
    let service = ResolverService::new(
        settings.clone(),
        Distribution::load(&settings.registry_dir).unwrap(),
    )
    .unwrap();
    assert!(!service.refresh().await.unwrap());
    let revision = service
        .resolve(1, ADDRESS)
        .await
        .unwrap()
        .result
        .registry_revision;
    std::fs::write(&path, "invalid JSON").unwrap();
    assert!(service.refresh().await.is_err());
    let retained = service.resolve(1, ADDRESS).await.unwrap();
    assert_eq!(retained.result.registry_revision, revision);
    assert!(retained
        .result
        .warnings
        .iter()
        .any(|w| w.contains("last valid")));
    assert_eq!(service.health().await["status"], "degraded");
    data["assets"][0]["asset"]["name"] = json!("Updated product display name");
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(service.refresh().await.unwrap());
    let fresh = service.resolve(1, ADDRESS).await.unwrap();
    assert_ne!(fresh.result.registry_revision, revision);
    assert_eq!(
        fresh.product.unwrap()["name"],
        "Updated product display name"
    );
    assert_eq!(service.health().await["status"], "ok");
    std::fs::remove_dir_all(&directory).unwrap();
}
#[tokio::test]
async fn cache_entry_and_byte_limits_evict_or_bypass_results() {
    use rwaimport_resolver::cache::{CacheKey, ResolutionCache};
    let service = ResolverService::new(config(None), distribution(fixture())).unwrap();
    let result = service.resolve(1, ADDRESS).await.unwrap();
    let first = CacheKey {
        input: result.result.input.clone(),
        revision: result.result.registry_revision.clone(),
    };
    let mut second = first.clone();
    second.revision = "different".into();
    let mut cache = ResolutionCache::new(Duration::from_secs(1), 1, 1024 * 1024);
    cache.insert(first.clone(), result.clone());
    cache.insert(second.clone(), result.clone());
    assert!(cache.get(&first).is_none());
    assert!(cache.get(&second).is_some());
    cache.clear();
    assert!(cache.get(&second).is_none());
    let mut cache = ResolutionCache::new(Duration::from_secs(1), 4, 1);
    cache.insert(first.clone(), result);
    assert!(cache.get(&first).is_none());
}

#[test]
fn validates_distribution_manifest_checksum_and_size() {
    use rwaimport_resolver::registry::distribution::validate_manifest;
    use sha2::{Digest, Sha256};
    let raw = b"registry bytes";
    let mut manifest = json!({"schemaVersion": 1, "files": {"registry.json": {"sha256": hex::encode(Sha256::digest(raw)), "bytes": raw.len()}}});
    assert!(validate_manifest(raw, &manifest).is_ok());
    manifest["files"]["registry.json"]["bytes"] = json!(1);
    assert!(validate_manifest(raw, &manifest).is_err());
    manifest["files"]["registry.json"]["bytes"] = json!(raw.len());
    manifest["files"]["registry.json"]["sha256"] = json!("00");
    assert!(validate_manifest(raw, &manifest).is_err());
}
#[test]
fn rejects_broken_underlying_evidence_and_standard_relationships() {
    for mode in [
        "underlying-product",
        "underlying-source",
        "standard-related",
    ] {
        let mut data = fixture();
        match mode {
            "underlying-product" => data["underlyings"][0]["sourceAssetIds"] = json!(["missing"]),
            "underlying-source" => {
                data["underlyings"][0]["claims"] = json!([{
                    "field": "jurisdiction",
                    "sourceIds": ["missing"],
                    "reviewStatus": "verified"
                }])
            }
            "standard-related" => data["standards"][0]["relatedStandardIds"] = json!(["missing"]),
            _ => unreachable!(),
        }
        assert!(
            Distribution::from_bytes(
                &serde_json::to_vec(&data).unwrap(),
                &fixture_dir().join("schemas")
            )
            .is_err(),
            "{mode}"
        );
    }
}
#[test]
fn verifies_recorded_proxy_admin_as_a_separate_claim() {
    use rwaimport_resolver::{
        contracts::ContractObservation, proxy::ProxyObservation, registry::RegistryReader,
        verification::verify,
    };
    let registry = distribution(fixture());
    let input = rwaimport_resolver::input::ResolveInput::new(1, ADDRESS).unwrap();
    let mut known = registry.find_deployment(&input).unwrap().unwrap();
    known.expected_name = None;
    known.expected_symbol = None;
    known.expected_decimals = None;
    known.expected_implementation = None;
    known.expected_runtime_code_sha256 = None;
    known.expected_admin = Some(IMPLEMENTATION.into());
    let mut observed = ContractObservation {
        block_number: 1,
        block_hash: "0x".to_string() + &"aa".repeat(32),
        exists: true,
        name: None,
        symbol: None,
        decimals: None,
        total_supply: None,
        runtime_code_sha256: String::new(),
        proxy: ProxyObservation::Undetermined,
        owner: None,
        contract_admin: Some(IMPLEMENTATION.into()),
        policy_observations: Default::default(),
        capabilities: Default::default(),
        relationships: Default::default(),
        warnings: vec![],
    };
    assert_eq!(verify(Some(&known), Some(&observed)).0, Status::Verified);
    observed.contract_admin = Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    assert_eq!(verify(Some(&known), Some(&observed)).0, Status::Mismatch);
    observed.contract_admin = None;
    assert_eq!(verify(Some(&known), Some(&observed)).0, Status::Partial);
}

#[cfg(unix)]
#[tokio::test]
async fn picks_up_atomically_activated_release_with_matching_schemas() {
    use std::os::unix::fs::symlink;
    let directory =
        std::env::temp_dir().join(format!("rwaimport-pointer-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for label in ["a", "b"] {
        let release = directory.join(label);
        std::fs::create_dir_all(release.join("dist")).unwrap();
        symlink(fixture_dir().join("schemas"), release.join("schemas")).unwrap();
        let mut data = fixture();
        data["assets"][0]["asset"]["name"] = json!(label);
        std::fs::write(
            release.join("dist/registry.json"),
            serde_json::to_vec(&data).unwrap(),
        )
        .unwrap();
    }
    symlink(directory.join("a"), directory.join("current")).unwrap();
    let mut settings = config(None);
    settings.registry_dir = directory.join("current/dist");
    let service = ResolverService::new(
        settings.clone(),
        Distribution::load(&settings.registry_dir).unwrap(),
    )
    .unwrap();
    assert_eq!(
        service.resolve(1, ADDRESS).await.unwrap().product.unwrap()["name"],
        "a"
    );
    symlink(directory.join("b"), directory.join("next")).unwrap();
    std::fs::rename(directory.join("next"), directory.join("current")).unwrap();
    assert!(service.refresh().await.unwrap());
    assert_eq!(
        service.resolve(1, ADDRESS).await.unwrap().product.unwrap()["name"],
        "b"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn explicit_permission_policy_is_block_pinned_and_reports_missing_checks() {
    let (url, mock, task) = mock_rpc("ok").await;
    let policy = json!({"schemaVersion":1,"deployments":[{"input":{"chainId":1,"address":ADDRESS},"evmChecks":[{"field":"owner","signature":"owner()","expected":IMPLEMENTATION},{"field":"paused","signature":"paused()","expected":false}]}]});
    let policies = rwaimport_resolver::policies::PolicyCatalog::from_bytes(
        &serde_json::to_vec(&policy).unwrap(),
    )
    .unwrap();
    let service =
        ResolverService::with_policies(config(Some(url)), distribution(fixture()), policies)
            .unwrap();
    let result = service.resolve(1, ADDRESS).await.unwrap();
    assert_eq!(result.result.status, Status::Partial);
    assert!(result.verification.policy_applied);
    assert!(result
        .verification
        .checks_performed
        .contains(&"policy.owner".into()));
    assert!(result
        .verification
        .checks_unavailable
        .contains(&"policy.paused".into()));
    for call in mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c["method"] == "eth_call")
    {
        assert_eq!(
            call["params"][1]["blockHash"],
            format!("0x{}", "aa".repeat(32))
        );
    }
    task.abort();
}
#[tokio::test]
async fn batch_preserves_order_isolates_errors_and_exposes_metrics() {
    let service = Arc::new(ResolverService::new(config(None), distribution(fixture())).unwrap());
    let app = router(service);
    let body = json!({"requests":[{"chainId":1,"address":ADDRESS},{"chainId":0,"address":ADDRESS},{"network":"solana","address":"bad"}]});
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/resolve/batch")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let data: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["data"][0]["index"], 0);
    assert_eq!(data["data"][0]["data"]["status"], "PARTIAL");
    assert_eq!(data["data"][1]["error"]["code"], "INVALID_CHAIN_ID");
    assert_eq!(data["data"][2]["error"]["code"], "INVALID_ADDRESS");
    let metrics = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/metrics")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let text = String::from_utf8(
        metrics
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("rwaimport_resolutions_total 3"));
    let too_big = json!({"requests":vec![json!({"chainId":1,"address":ADDRESS});65]});
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/resolve/batch")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(too_big.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn standard_and_role_policies_compare_real_reads_without_claiming_full_conformance() {
    let (url, _, task) = mock_rpc("policy").await;
    let mut data = fixture();
    data["assets"][0]["deployments"][0]["standardIds"] =
        json!(["erc20", "erc4626", "erc3643", "erc1404", "cmtat"]);
    let policy = json!({"schemaVersion":1,"deployments":[{"input":{"chainId":1,"address":ADDRESS},"evmChecks":[
        {"field":"minter","signature":"hasRole(bytes32,address)","args":[format!("0x{}","0".repeat(64)),IMPLEMENTATION],"expected":true},
        {"field":"restriction","signature":"detectTransferRestriction(address,address,uint256)","args":[ADDRESS,IMPLEMENTATION,"1"],"expected":"2"},
        {"field":"paused","signature":"paused()","expected":false},
        {"field":"version","signature":"VERSION()","expected":"3.0.0"}
    ]}]});
    let catalog = rwaimport_resolver::policies::PolicyCatalog::from_bytes(
        &serde_json::to_vec(&policy).unwrap(),
    )
    .unwrap();
    let service = ResolverService::with_policies(
        config(Some(url.clone())),
        distribution(data.clone()),
        catalog,
    )
    .unwrap();
    let result = service.resolve(1, ADDRESS).await.unwrap();
    assert_eq!(result.result.status, Status::Verified);
    assert!(result
        .verification
        .checks_performed
        .contains(&"policy.minter".into()));
    assert_eq!(
        result.result.contract.unwrap().relationships["convertToAssetsZero"],
        Some("0".into())
    );
    let mut changed = policy;
    changed["deployments"][0]["evmChecks"][2]["expected"] = json!(true);
    let catalog = rwaimport_resolver::policies::PolicyCatalog::from_bytes(
        &serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let service =
        ResolverService::with_policies(config(Some(url)), distribution(data), catalog).unwrap();
    assert_eq!(
        service.resolve(1, ADDRESS).await.unwrap().result.status,
        Status::Mismatch
    );
    task.abort();
}

fn registry_policy_fixture(review_status: &str) -> Value {
    let mut data = fixture();
    let deployment = data["assets"][0]["deployments"][0].clone();
    let unknown = json!({"availability":"unknown"});
    data["deploymentPolicies"] = json!({
        "schemaVersion":1,"generatedAt":"2026-10-06T00:00:00Z","deployments":[{
            "assetId":data["assets"][0]["asset"]["id"],
            "deployment":{"chain":deployment["chain"],"address":deployment["address"]},
            "input":{"chainId":1,"address":deployment["address"]},
            "permissions":{"owner":unknown,"admin":unknown,"mint":unknown,"burn":unknown,"pause":unknown,"upgrade":unknown},
            "standardPolicies":{},"supportedChecks":["evm-read"],
            "evmChecks":[{"field":"paused","signature":"paused()","expected":false}],"ledgerChecks":[],
            "provenance":{"sourceIds":[data["assets"][0]["sources"][0]["id"]],"reviewedAt":"2026-10-06","reviewer":"test-fixture","reviewStatus":review_status}
        }]
    });
    data
}
#[tokio::test]
async fn consumes_only_verified_registry_policies_from_the_snapshot() {
    for (review, applied) in [
        ("verified", true),
        ("needs-review", false),
        ("disputed", false),
    ] {
        let service = ResolverService::with_policies(
            config(None),
            distribution(registry_policy_fixture(review)),
            rwaimport_resolver::policies::PolicyCatalog::empty(),
        )
        .unwrap();
        let result = service.resolve(1, ADDRESS).await.unwrap();
        assert_eq!(result.verification.policy_applied, applied);
        assert!(result
            .verification
            .policy_version
            .starts_with("resolver-policy-v1:"));
        if applied {
            assert!(result
                .verification
                .checks_unavailable
                .contains(&"policy.paused".to_string()));
        }
    }
}
#[test]
fn rejects_registry_policies_with_missing_evidence_or_wrong_locator() {
    for field in ["evidence", "locator", "schema"] {
        let mut data = registry_policy_fixture("verified");
        match field {
            "evidence" => {
                data["deploymentPolicies"]["deployments"][0]["provenance"]["sourceIds"] =
                    json!(["missing-source"])
            }
            "locator" => {
                data["deploymentPolicies"]["deployments"][0]["input"]["chainId"] = json!(99999)
            }
            _ => data["deploymentPolicies"]["schemaVersion"] = json!(2),
        }
        assert!(Distribution::from_bytes(
            &serde_json::to_vec(&data).unwrap(),
            &fixture_dir().join("schemas")
        )
        .is_err());
    }
}
#[tokio::test]
async fn batch_obeys_configured_response_byte_budget() {
    let mut service = ResolverService::with_policies(
        config(None),
        distribution(fixture()),
        rwaimport_resolver::policies::PolicyCatalog::empty(),
    )
    .unwrap();
    service.max_response_bytes = 1024;
    let service = Arc::new(service);
    let input = rwaimport_resolver::input::ResolutionInput::Evm(
        rwaimport_resolver::input::ResolveInput::new(1, ADDRESS).unwrap(),
    );
    assert!(service
        .batch(rwaimport_resolver::batch::BatchRequest {
            requests: vec![input]
        })
        .await
        .is_err());
}
