//! `docair serve`: the CLI as a web server for one project, and for uploads (SPEC §1.2, docs/SLICES.md §8).
//!
//! The server itself is docair-server's router with the project in its
//! settings: the project's documentation at `/`, the AI routes on the
//! server's own graph, uploads as the docair-server program takes them. This
//! file starts it, watches the project's files by the system's file events,
//! and stops it on Ctrl-C. `docair build` is the way to build the
//! documentation on demand, without a server.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use docair_server::project::ServedProject;
use tower_http::services::{ServeDir, ServeFile};

/// A change is acted on once the files have been quiet this long: a save or a checkout is many events.
const QUIET: Duration = Duration::from_millis(400);
/// Without file events (the system refused a watch), how often the files are looked at instead.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/***
 * `docair serve`: serve a project's documentation and the AI on it, and uploads, until Ctrl-C.
 *
 * The documentation is written again when the project's files change (the
 * system's file events, acted on once they are quiet for 0.4 s; changes in
 * the output folder and in what a scan skips do not count), after an answer
 * that brings a walkthrough, and after applied edits. Other files in the
 * output folder are served at their paths; with `--app`, the web app is
 * served at `/app/`.
 *
 * @calls docair-cli.ask.ai_config "reads the AI settings of the project"
 * @calls docair-server.project.served-project.open "scans the project and writes its documentation"
 * @calls docair-server.router "serves the project, the AI on it and uploads"
 * @calls docair-cli.serve.watch "writes the documentation again when files change"
 ***/
pub fn cmd_serve(
    root: &Path,
    out: Option<PathBuf>,
    lenient: bool,
    bind: &str,
    app: Option<PathBuf>,
    changes: Option<String>,
) -> Result<ExitCode> {
    let ai = crate::ask::ai_config(root)?;
    let project = Arc::new(ServedProject::open_comparing(
        root,
        out,
        lenient,
        ai.clone(),
        changes,
    )?);
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
        let watched = project.clone();
        std::thread::spawn(move || watch(&watched));
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        anyhow::Ok(())
    })?;
    Ok(ExitCode::SUCCESS)
}

/***
 * Watch the project's files by the system's file events, and write the documentation again once a change has settled; without file events, look at the files every two seconds instead.
 *
 * @calls docair-server.project.watched "keeps changes that can change the documentation"
 * @calls docair-server.project.served-project.refresh "scans again when the files changed"
 ***/
fn watch(project: &ServedProject) {
    let (root, out) = project.folders();
    let refresh = || match project.refresh() {
        Ok(true) => println!(
            "docair serve: files changed, documentation written again (version {})",
            project.version.load(Ordering::SeqCst)
        ),
        Ok(false) => {}
        Err(e) => eprintln!("docair serve: {e:#}"),
    };
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(e) => return look(&refresh, &e),
    };
    if let Err(e) = notify::Watcher::watch(&mut watcher, &root, notify::RecursiveMode::Recursive) {
        return look(&refresh, &e);
    }
    let counts = |ev: &notify::Result<notify::Event>| {
        ev.as_ref().is_ok_and(|e| {
            !e.kind.is_access()
                && e.paths
                    .iter()
                    .any(|p| docair_server::project::watched(&root, &out, p))
        })
    };
    while let Ok(ev) = rx.recv() {
        if !counts(&ev) {
            continue;
        }
        // Wait until the files are quiet: what counts restarts the wait, the rest is dropped.
        loop {
            match rx.recv_timeout(QUIET) {
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        refresh();
    }
}

/// Without file events: look at the files every `LOOK_EVERY`.
fn look(refresh: &dyn Fn(), why: &dyn std::fmt::Display) {
    eprintln!(
        "docair serve: no file events ({why}); looking at the files every {} s instead",
        LOOK_EVERY.as_secs()
    );
    loop {
        std::thread::sleep(LOOK_EVERY);
        refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved file is seen by its file event, and the documentation is written again; a change in the output folder is not.
    #[test]
    fn a_saved_file_writes_the_documentation_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("docair.toml"), "system = \"tiny\"\n").unwrap();
        let readme = dir.path().join("README.md");
        let block = |summary: &str| {
            format!(
                "# tiny\n\n```docair\n{summary}\n\n@id      app\n@kind    module\n@part-of tiny\n```\n"
            )
        };
        std::fs::write(&readme, block("The app.")).unwrap();
        let project = Arc::new(ServedProject::open(dir.path(), None, true, None).unwrap());
        let watched = project.clone();
        std::thread::spawn(move || watch(&watched));
        let version = || project.version.load(Ordering::SeqCst);
        let wait = |want: u64| {
            for _ in 0..100 {
                if version() >= want {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            false
        };
        // Let the watch start before the first change.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(dir.path().join(".docair").join("note.txt"), "an output").unwrap();
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(
            version(),
            1,
            "a change in the output folder writes nothing again"
        );
        std::fs::write(&readme, block("The app, changed.")).unwrap();
        assert!(wait(2), "the saved file wrote the documentation again");
        let html = std::fs::read_to_string(dir.path().join(".docair").join("docair.html")).unwrap();
        assert!(html.contains("changed"), "the new words are in the page");
    }
}
