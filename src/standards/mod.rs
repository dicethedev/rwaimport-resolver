//! Capability observations are hints, not proofs of full token-standard conformance.
use crate::contracts::ContractObservation;

pub fn erc20_metadata(observation: &ContractObservation) -> Option<bool> {
    if observation.name.is_some()
        && observation.symbol.is_some()
        && observation.decimals.is_some()
        && observation.total_supply.is_some()
    {
        Some(true)
    } else {
        None
    }
}
/// ERC-165 requires supporting its own interface and rejecting 0xffffffff.
pub fn erc165(supported: Option<bool>, invalid: Option<bool>) -> Option<bool> {
    match (supported, invalid) {
        (Some(true), Some(false)) => Some(true),
        (Some(_), Some(_)) => Some(false),
        _ => None,
    }
}
