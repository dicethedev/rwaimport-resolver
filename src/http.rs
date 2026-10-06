use crate::{errors::ResolveError, service::ResolverService};
use axum::{
    extract::{DefaultBodyLimit, Path, RawQuery, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
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
        .route("/metrics", get(metrics))
        .route("/v1/resolve/batch", post(batch))
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
        .layer(DefaultBodyLimit::max(256 * 1024))
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
async fn ready(State(service): State<Arc<ResolverService>>) -> Response {
    let health = service.health().await;
    let status = if health["status"] == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(health)).into_response()
}
async fn metrics(State(service): State<Arc<ResolverService>>) -> Response {
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        service.metrics(),
    )
        .into_response()
}
async fn batch(
    State(service): State<Arc<ResolverService>>,
    RawQuery(query): RawQuery,
    body: Result<Json<crate::batch::BatchRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if query.is_some_and(|q| !q.is_empty()) {
        return failure(
            "UNSUPPORTED_QUERY",
            "This endpoint accepts no query parameters",
            StatusCode::BAD_REQUEST,
        );
    }
    let request = match body {
        Ok(Json(request)) => request,
        Err(error) => {
            return failure(
                "INVALID_BATCH",
                "Invalid batch JSON",
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    StatusCode::PAYLOAD_TOO_LARGE
                } else {
                    StatusCode::BAD_REQUEST
                },
            )
        }
    };
    match service.batch(request).await {
        Ok(results) => Json(json!({"data":results})).into_response(),
        Err(error) => error.into_response(),
    }
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
    match service
        .resolve_any(crate::input::ResolutionInput::Evm(
            crate::input::ResolveInput { chain_id, address },
        ))
        .await
    {
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
        let (code, message, status) = self.public_error();
        failure(code, message, StatusCode::from_u16(status).unwrap())
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
    if code.is_some() && network != "stellar" && network != "aptos" {
        return ResolveError::InvalidAddress.into_response();
    }
    if query.is_some_and(|q| !q.is_empty()) {
        return failure(
            "UNSUPPORTED_QUERY",
            "This endpoint accepts no query parameters",
            StatusCode::BAD_REQUEST,
        );
    }
    match service
        .resolve_any(crate::input::ResolutionInput::Ledger(
            crate::input::LedgerInput {
                network: network.clone(),
                address,
                asset_code: if network == "stellar" {
                    code.clone()
                } else {
                    None
                },
                coin_type: if network == "aptos" { code } else { None },
            },
        ))
        .await
    {
        Ok(value) => Json(json!({"data": value})).into_response(),
        Err(error) => error.into_response(),
    }
}
