pub mod live;
use crate::{contracts::ContractObservation, errors::ResolveError, input::ResolveInput};

/// A live adapter must verify eth_chainId, pin a block and bound timeouts/retries.
/// Missing optional reads become None; provider failures are RpcUnavailable.
pub trait ChainReader {
    fn observe(&self, input: &ResolveInput) -> Result<ContractObservation, ResolveError>;
}
