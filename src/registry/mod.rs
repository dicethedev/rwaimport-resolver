pub mod distribution;
use crate::{errors::ResolveError, input::ResolveInput, types::RegistryMatch};

/// Adapter must hold one validated, immutable registry revision for the request.
/// Duplicate exact deployment matches must return AmbiguousDeployment.
pub trait RegistryReader {
    fn revision(&self) -> &str;
    fn supports_chain(&self, chain_id: u64) -> bool;
    fn find_deployment(&self, input: &ResolveInput) -> Result<Option<RegistryMatch>, ResolveError>;
}
