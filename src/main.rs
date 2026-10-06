use rwaimport_resolver::{
    config::Config, http::router, registry::distribution::Distribution, service::ResolverService,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, directory] = args.as_slice() {
        if flag == "--check-registry" {
            let snapshot = Distribution::load(std::path::Path::new(directory))?;
            println!(
                "{}",
                serde_json::json!({"registryRevision": snapshot.revision, "generatedAt": snapshot.generated_at, "supportedChains": snapshot.supported_chains()})
            );
            return Ok(());
        }
    }
    if !args.is_empty() {
        return Err("Usage: rwaimport-resolver [--check-registry DIST_DIRECTORY]".into());
    }
    let config = Config::from_env()?;
    let directory = config.registry_dir.clone();
    let snapshot = tokio::task::spawn_blocking(move || Distribution::load(&directory)).await??;
    let service = Arc::new(ResolverService::new(config.clone(), snapshot)?);
    let poller = service.clone();
    let refresh = tokio::spawn(async move {
        let mut interval = tokio::time::interval(config.refresh_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = poller.refresh().await {
                rwaimport_resolver::operations::event(
                    "registryRefreshFailed",
                    serde_json::json!({"error": error.to_string(), "retainedLastValidRevision": true}),
                );
            }
        }
    });
    let probe_service = service.clone();
    let probe_seconds = std::env::var("PROVIDER_PROBE_INTERVAL_SECONDS")
        .unwrap_or_else(|_| "60".into())
        .parse::<u64>()
        .map_err(|_| "Invalid PROVIDER_PROBE_INTERVAL_SECONDS")?;
    if probe_seconds > 3600 || probe_seconds > 0 && probe_seconds < 10 {
        return Err("Probe interval must be zero or 10..3600 seconds".into());
    }
    let probes = tokio::spawn(async move {
        if probe_seconds == 0 {
            return;
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(probe_seconds));
        loop {
            interval.tick().await;
            probe_service.probe_providers().await;
        }
    });
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    rwaimport_resolver::operations::event(
        "listening",
        serde_json::json!({"bind":listener.local_addr()?.to_string()}),
    );
    let result = axum::serve(listener, router(service))
        .with_graceful_shutdown(shutdown())
        .await;
    refresh.abort();
    probes.abort();
    result?;
    Ok(())
}
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}
