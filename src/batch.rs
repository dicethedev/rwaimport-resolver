use crate::{
    errors::ResolveError, input::ResolutionInput, service::ResolverService,
    types::ResolutionEnvelope,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRequest {
    pub requests: Vec<ResolutionInput>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchError {
    pub code: String,
    pub message: String,
    pub status_code: u16,
}
#[derive(Debug, Serialize)]
pub struct BatchItem {
    pub index: usize,
    pub input: ResolutionInput,
    pub data: Option<ResolutionEnvelope>,
    pub error: Option<BatchError>,
}
impl ResolverService {
    pub async fn batch(
        self: &Arc<Self>,
        request: BatchRequest,
    ) -> Result<Vec<BatchItem>, ResolveError> {
        if request.requests.is_empty() || request.requests.len() > self.batch_max {
            return Err(ResolveError::InvalidBatch);
        }
        let deadline = tokio::time::Instant::now() + self.batch_timeout;
        let mut response_bytes = 12usize;
        let mut pending = request.requests.into_iter().enumerate();
        let mut tasks = tokio::task::JoinSet::new();
        let mut results = vec![];
        let width = self.config.max_concurrency.min(8);
        loop {
            while tasks.len() < width {
                let Some((index, input)) = pending.next() else {
                    break;
                };
                let service = self.clone();
                tasks.spawn(async move {
                    let result = service.resolve_any(input.clone()).await;
                    match result {
                        Ok(data) => BatchItem {
                            index,
                            input,
                            data: Some(data),
                            error: None,
                        },
                        Err(error) => {
                            let (code, message, status) = error.public_error();
                            BatchItem {
                                index,
                                input,
                                data: None,
                                error: Some(BatchError {
                                    code: code.into(),
                                    message: message.into(),
                                    status_code: status,
                                }),
                            }
                        }
                    }
                });
            }
            if tasks.is_empty() {
                break;
            }
            let item = tokio::time::timeout_at(deadline, tasks.join_next())
                .await
                .map_err(|_| ResolveError::RpcUnavailable)?
                .ok_or(ResolveError::RegistryUnavailable)?
                .map_err(|_| ResolveError::RegistryUnavailable)?;
            response_bytes += serde_json::to_vec(&item)
                .map_err(|_| ResolveError::RegistryUnavailable)?
                .len()
                + 1;
            if response_bytes > self.max_response_bytes {
                return Err(ResolveError::RpcUnavailable);
            }
            results.push(item);
        }
        results.sort_by_key(|r| r.index);
        Ok(results)
    }
}
