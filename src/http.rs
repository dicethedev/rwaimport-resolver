use crate::{errors::ResolveError, service::ResolverService};
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;

pub fn router(service: Arc<ResolverService>) -> Router {
    Router::new()
        .route(
            "/health/live",
            get(|| async { Json(json!({"status": "ok"})) }),
        )
        .route("/health/ready", get(ready))
        .route("/v1/resolve/{chain_id}/{address}", get(resolve))
        .route(
            "/v1/resolve/network/{network}/{address}",
            get(resolve_network),
        )
        .route(
            "/v1/resolve/network/{network}/{address}/{asset_code}",
            get(resolve_asset),
        )
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": {"code": "NOT_FOUND", "message": "Route not found"}})),
            )
        })
        .layer(middleware::from_fn(no_store))
        .with_state(service)
}
async fn no_store(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
async fn ready(State(service): State<Arc<ResolverService>>) -> Json<serde_json::Value> {
    Json(service.health().await)
}
async fn resolve(
    State(service): State<Arc<ResolverService>>,
    Path((chain_id, address)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    if query.is_some_and(|q| !q.is_empty()) {
        return failure(
            "UNSUPPORTED_QUERY",
            "This endpoint accepts no query parameters",
            StatusCode::BAD_REQUEST,
        );
    }
    if chain_id.is_empty()
        || !chain_id.bytes().all(|v| v.is_ascii_digit())
        || chain_id.starts_with('0')
    {
        return ResolveError::InvalidChainId.into_response();
    }
    let Ok(chain_id) = chain_id.parse::<u64>() else {
        return ResolveError::InvalidChainId.into_response();
    };
    match service.resolve(chain_id, &address).await {
        Ok(result) => Json(json!({"data": result})).into_response(),
        Err(error) => error.into_response(),
    }
}
fn failure(code: &str, message: &str, status: StatusCode) -> Response {
    (
        status,
        Json(json!({"statusCode": status.as_u16(), "error": {"code": code, "message": message}})),
    )
        .into_response()
}
impl IntoResponse for ResolveError {
    fn into_response(self) -> Response {
        let (code, message, status) = match self {
            Self::InvalidChainId => (
                "INVALID_CHAIN_ID",
                "Chain ID must be a positive decimal integer",
                StatusCode::BAD_REQUEST,
            ),
            Self::InvalidAddress => (
                "INVALID_ADDRESS",
                "Invalid ledger address or missing Stellar asset code",
                StatusCode::BAD_REQUEST,
            ),
            Self::UnsupportedChain => (
                "UNSUPPORTED_CHAIN",
                "Network is not supported by this endpoint",
                StatusCode::BAD_REQUEST,
            ),
            Self::Busy => (
                "RESOLVER_BUSY",
                "Resolver request capacity exceeded",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            Self::RpcChainMismatch => (
                "RPC_CHAIN_MISMATCH",
                "Configured RPC provider reports a different chain ID",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            _ => (
                "RESOLVER_UNAVAILABLE",
                "Resolution source unavailable",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        };
        failure(code, message, status)
    }
}

async fn resolve_network(
    State(service): State<Arc<ResolverService>>,
    Path((network, address)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    ledger_response(service, network, address, None, query).await
}
async fn resolve_asset(
    State(service): State<Arc<ResolverService>>,
    Path((network, address, code)): Path<(String, String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    ledger_response(service, network, address, Some(code), query).await
}
async fn ledger_response(
    service: Arc<ResolverService>,
    network: String,
    address: String,
    code: Option<String>,
    query: Option<String>,
) -> Response {
    if query.is_some_and(|q| !q.is_empty()) {
        return failure(
            "UNSUPPORTED_QUERY",
            "This endpoint accepts no query parameters",
            StatusCode::BAD_REQUEST,
        );
    }
    match service
        .resolve_ledger(&network, &address, code.as_deref())
        .await
    {
        Ok(value) => Json(json!({"data": value})).into_response(),
        Err(error) => error.into_response(),
    }
}
