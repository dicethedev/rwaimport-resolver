use crate::{
    errors::ResolveError, input::ResolveInput, registry::RegistryReader, rpc::ChainReader,
    types::ResolveResult, verification::verify,
};

pub fn resolve(
    registry: &impl RegistryReader,
    chain: &impl ChainReader,
    chain_id: u64,
    address: &str,
) -> Result<ResolveResult, ResolveError> {
    let input = ResolveInput::new(chain_id, address)?;
    if !registry.supports_chain(input.chain_id) {
        return Err(ResolveError::UnsupportedChain);
    }
    let matched = registry.find_deployment(&input)?;
    let mut warnings = vec![];
    let contract = match chain.observe(&input) {
        Ok(value) => Some(value),
        Err(ResolveError::RpcUnavailable) => {
            warnings.push("Live chain observation unavailable".into());
            None
        }
        Err(error) => return Err(error),
    };
    let (status, checks) = verify(matched.as_ref(), contract.as_ref());
    Ok(ResolveResult {
        status,
        input,
        registry_revision: registry.revision().into(),
        matched,
        contract,
        checks,
        warnings,
    })
}
