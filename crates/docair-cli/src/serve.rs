//! `docair serve`: the CLI as a web server for one project, and for uploads (SPEC §1.2, docs/SLICES.md §8).
//!
//! The server itself is docair-server's router with the project in its
//! settings: the project's documentation at `/`, the AI routes on the
//! server's own graph, uploads as the docair-server program takes them. This
//! file starts it, looks at the project's files every second, and stops it on
//! Ctrl-C.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use docair_server::project::ServedProject;
use tower_http::services::{ServeDir, ServeFile};

/// How often the project's files are looked at for changes.
const WATCH_EVERY: Duration = Duration::from_secs(1);

/***
 * `docair serve`: serve a project's documentation and the AI on it, and uploads, until Ctrl-C.
 *
 * The documentation is written again when the project's files change (looked
 * at every second), after an answer that brings a walkthrough, and after
 * applied edits. Other files in the output folder are served at their paths;
 * with `--app`, the web app is served at `/app/`.
 *
 * @calls docair-cli.ask.ai_config "reads the AI settings of the project"
 * @calls docair-server.project.served-project.open "scans the project and writes its documentation"
 * @calls docair-server.router "serves the project, the AI on it and uploads"
 * @calls docair-server.project.served-project.refresh "writes the documentation again when files change"
 ***/
pub fn cmd_serve(
    root: &Path,
    out: Option<PathBuf>,
    lenient: bool,
    bind: &str,
    app: Option<PathBuf>,
) -> Result<ExitCode> {
    let ai = crate::ask::ai_config(root)?;
    let project = Arc::new(ServedProject::open(root, out, lenient, ai.clone())?);
    let mut config = docair_server::Config::default();
    config.ai = ai;
    config.project = Some(project.clone());
    let mut router = docair_server::router(config);
    if let Some(dir) = &app {
        let index = dir.join("index.html");
        router = router.nest_service("/app", ServeDir::new(dir).fallback(ServeFile::new(index)));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        println!("docair serve: http://{addr}/ shows {}", root.display());
        if app.is_some() {
            println!("docair serve: the web app is at http://{addr}/app/");
        }
        println!(
            "docair serve: AI is {}",
            if project.ai_on {
                "on"
            } else {
                "off ([ai] in docair.toml)"
            }
        );
        let watcher = project.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(WATCH_EVERY).await;
                let w = watcher.clone();
                match tokio::task::spawn_blocking(move || w.refresh()).await {
                    Ok(Ok(true)) => println!(
                        "docair serve: files changed, documentation written again (version {})",
                        watcher.version.load(Ordering::SeqCst)
                    ),
                    Ok(Err(e)) => eprintln!("docair serve: {e:#}"),
                    _ => {}
                }
            }
        });
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        anyhow::Ok(())
    })?;
    Ok(ExitCode::SUCCESS)
}
