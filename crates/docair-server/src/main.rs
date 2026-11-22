//! `docair-server`: the HTTP backend of the docair web app.

use anyhow::Result;
use docair_server::{Config, router};

/***
 * Start the server: read the settings, listen on DOCAIR_BIND (default 0.0.0.0:8080), stop cleanly on SIGTERM.
 *
 * @id      docair-server.main
 * @calls   docair-server.config () -> Config
 * @calls   docair-server.router (config) -> routes
 ***/
#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env();
    let addr = std::env::var("DOCAIR_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!(
        "docair-server listening on {addr} (max upload {} MB, max {} files)",
        config.max_upload_bytes / (1024 * 1024),
        config.max_files
    );
    match &config.ai {
        Some(ai) => println!(
            "docair-server: asking {} at {} (key {})",
            ai.model,
            ai.public_endpoint(),
            if ai.api_key.is_some() {
                "set"
            } else {
                "not set"
            }
        ),
        None => println!("docair-server: AI is off (no DOCAIR_AI_ENDPOINT and DOCAIR_AI_MODEL)"),
    }
    axum::serve(listener, router(config))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// Resolves on Ctrl-C or SIGTERM (what Kubernetes sends before it stops a pod).
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    println!("docair-server: shutting down");
}
