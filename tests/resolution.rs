use rwaimport_resolver::{
    contracts::ContractObservation, errors::ResolveError, input::ResolveInput,
    proxy::ProxyObservation, registry::RegistryReader, resolver::resolve, rpc::ChainReader,
    types::*,
};

const ADDRESS: &str = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
struct Registry(Option<RegistryMatch>);
impl RegistryReader for Registry {
    fn revision(&self) -> &str {
        "fixture-revision"
    }
    fn supports_chain(&self, id: u64) -> bool {
        id == 1
    }
    fn find_deployment(&self, _: &ResolveInput) -> Result<Option<RegistryMatch>, ResolveError> {
        Ok(self.0.clone())
    }
}
struct Chain(Result<ContractObservation, ResolveError>);
impl ChainReader for Chain {
    fn observe(&self, _: &ResolveInput) -> Result<ContractObservation, ResolveError> {
        self.0.clone()
    }
}
fn known() -> Registry {
    Registry(Some(RegistryMatch {
        identity: Identity {
            product_id: "fund".into(),
            issuer_id: "issuer".into(),
            underlying_asset_id: "treasury".into(),
        },
        deployment: Deployment {
            id: "fund:ethereum:address".into(),
            network: "ethereum".into(),
            standard: "erc20".into(),
            status: "active".into(),
        },
        expected_admin: None,
        expected_implementation: None,
        expected_runtime_code_sha256: None,
        expected_name: None,
        expected_symbol: Some("FUND".into()),
        expected_decimals: Some(6),
        evidence_ids: vec![],
    }))
}
fn observation() -> ContractObservation {
    ContractObservation {
        block_number: 123,
        block_hash: "0x".to_owned() + &"ab".repeat(32),
        runtime_code_sha256: "hash".into(),
        owner: None,
        contract_admin: None,
        policy_observations: Default::default(),
        capabilities: Default::default(),
        relationships: Default::default(),
        warnings: vec![],
        exists: true,
        name: None,
        symbol: Some("FUND".into()),
        decimals: Some(6),
        total_supply: None,
        proxy: ProxyObservation::Undetermined,
    }
}
#[test]
fn validates_and_normalizes_input() {
    assert_eq!(
        ResolveInput::new(1, ADDRESS).unwrap().address,
        ADDRESS.to_lowercase()
    );
    for address in [
        "",
        "0xabc",
        " 0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "0xGGAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        assert_eq!(
            ResolveInput::new(1, address),
            Err(ResolveError::InvalidAddress)
        );
    }
    assert_eq!(
        ResolveInput::new(0, ADDRESS),
        Err(ResolveError::InvalidChainId)
    );
    assert_eq!(
        resolve(&known(), &Chain(Ok(observation())), 2, ADDRESS),
        Err(ResolveError::UnsupportedChain)
    );
}
#[test]
fn selected_claims_match() {
    let result = resolve(&known(), &Chain(Ok(observation())), 1, ADDRESS).unwrap();
    assert_eq!(result.status, Status::Verified);
    assert_eq!(result.registry_revision, "fixture-revision");
    assert!(result
        .checks
        .iter()
        .all(|c| c.status == CheckStatus::Verified));
}
#[test]
fn mismatch_takes_precedence_over_missing_reads() {
    let mut observed = observation();
    observed.symbol = Some("OTHER".into());
    observed.decimals = None;
    let result = resolve(&known(), &Chain(Ok(observed)), 1, ADDRESS).unwrap();
    assert_eq!(result.status, Status::Mismatch);
    assert_eq!(result.checks[2].status, CheckStatus::Unavailable);
}
#[test]
fn provider_failure_preserves_identity_without_verifying() {
    let result = resolve(
        &known(),
        &Chain(Err(ResolveError::RpcUnavailable)),
        1,
        ADDRESS,
    )
    .unwrap();
    assert_eq!(result.status, Status::Partial);
    assert!(result.matched.is_some());
    assert!(result.contract.is_none());
    assert_eq!(result.warnings.len(), 1);
}
#[test]
fn absent_code_conflicts_with_known_deployment() {
    let mut observed = observation();
    observed.exists = false;
    observed.symbol = None;
    observed.decimals = None;
    assert_eq!(
        resolve(&known(), &Chain(Ok(observed)), 1, ADDRESS)
            .unwrap()
            .status,
        Status::Mismatch
    );
}
#[test]
fn unknown_only_means_unidentified() {
    assert_eq!(
        resolve(&Registry(None), &Chain(Ok(observation())), 1, ADDRESS)
            .unwrap()
            .status,
        Status::Unknown
    );
    assert_eq!(
        resolve(
            &Registry(None),
            &Chain(Err(ResolveError::RpcUnavailable)),
            1,
            ADDRESS
        )
        .unwrap()
        .status,
        Status::Unknown
    );
}
#[test]
fn existence_alone_is_insufficient_for_verified() {
    let mut registry = known();
    let entry = registry.0.as_mut().unwrap();
    entry.expected_symbol = None;
    entry.expected_decimals = None;
    assert_eq!(
        resolve(&registry, &Chain(Ok(observation())), 1, ADDRESS)
            .unwrap()
            .status,
        Status::Partial
    );
}
