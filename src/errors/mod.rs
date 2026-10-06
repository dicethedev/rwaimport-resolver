#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    InvalidChainId,
    InvalidBatch,
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

impl ResolveError {
    pub fn public_error(&self) -> (&'static str, &'static str, u16) {
        match self {
            Self::InvalidBatch => (
                "INVALID_BATCH",
                "Batch must contain between one and the configured maximum number of requests",
                400,
            ),
            Self::InvalidChainId => (
                "INVALID_CHAIN_ID",
                "Chain ID must be a positive decimal integer",
                400,
            ),
            Self::InvalidAddress => (
                "INVALID_ADDRESS",
                "Invalid ledger locator or missing asset code",
                400,
            ),
            Self::UnsupportedChain => (
                "UNSUPPORTED_CHAIN",
                "Network is not supported by this endpoint",
                400,
            ),
            Self::Busy => ("RESOLVER_BUSY", "Resolver request capacity exceeded", 503),
            Self::RpcChainMismatch => (
                "RPC_CHAIN_MISMATCH",
                "Configured provider reports a different network",
                503,
            ),
            _ => ("RESOLVER_UNAVAILABLE", "Resolution source unavailable", 503),
        }
    }
}
