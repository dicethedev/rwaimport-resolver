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
                eprintln!("Registry refresh failed: {error}; retaining last valid revision");
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    eprintln!("RWAimport resolver listening on {}", listener.local_addr()?);
    let result = axum::serve(listener, router(service))
        .with_graceful_shutdown(shutdown())
        .await;
    refresh.abort();
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
