#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    InvalidChainId,
    InvalidAddress,
    UnsupportedChain,
    RegistryUnavailable,
    AmbiguousDeployment,
    RpcUnavailable,
    RpcNotConfigured,
    RpcChainMismatch,
    Busy,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ResolveError {}
