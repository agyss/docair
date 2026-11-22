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
/// … and at most this long after its first event, while changes keep coming (docs/REGENERATE.md §2.1).
const AT_MOST: Duration = Duration::from_secs(3);

/// What the watcher waits for: a file event, or a wake without one (a code index landed).
enum Heard {
    File(notify::Result<notify::Event>),
    Wake,
}
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
    changes: Option<docair_server::project::ComparedSides>,
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
 * Watch the project's files by the system's file events, and write the documentation again once a change has settled: 0.4 s after its last event, at most 3 s after its first (docs/REGENERATE.md §2.1), with the paths that changed; also when woken without one (a code index made in the background landed); without file events, look at the files every two seconds instead.
 *
 * @calls docair-server.project.watched "keeps changes that can change the documentation"
 * @calls docair-server.project.served-project.refresh_changed "scans again from the changed paths"
 ***/
fn watch(project: &ServedProject) {
    let (root, out) = project.folders();
    // File events name absolute paths: a root given as `.` would hold none of them.
    let root = root.canonicalize().unwrap_or(root);
    let out = out.canonicalize().unwrap_or(out);
    let refresh = |changed: &[PathBuf], force: bool| match project.refresh_changed(changed, force) {
        Ok(true) => println!(
            "docair serve: files changed, documentation written again (version {})",
            project.version.load(Ordering::SeqCst)
        ),
        Ok(false) => {}
        Err(e) => eprintln!("docair serve: {e:#}"),
    };
    let (tx, rx) = std::sync::mpsc::channel::<Heard>();
    let wake = tx.clone();
    project.set_wake(Box::new(move || {
        let _ = wake.send(Heard::Wake);
    }));
    let mut watcher = match notify::recommended_watcher(move |ev| {
        let _ = tx.send(Heard::File(ev));
    }) {
        Ok(w) => w,
        Err(e) => return look(&|| refresh(&[], false), &e),
    };
    if let Err(e) = notify::Watcher::watch(&mut watcher, &root, notify::RecursiveMode::Recursive) {
        return look(&|| refresh(&[], false), &e);
    }
    // The paths of an event that count: inside the project, outside the output and what a scan skips.
    let counted = |ev: &notify::Result<notify::Event>| -> Vec<PathBuf> {
        match ev {
            Ok(e) if !e.kind.is_access() => e
                .paths
                .iter()
                .filter(|p| docair_server::project::watched(&root, &out, p))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    };
    while let Ok(heard) = rx.recv() {
        let (mut changed, mut force) = match heard {
            Heard::Wake => (Vec::new(), true),
            Heard::File(ev) => (counted(&ev), false),
        };
        if changed.is_empty() && !force {
            continue;
        }
        // Wait until the files are quiet, at most AT_MOST: what counts restarts the wait, the rest is
        // dropped, so a build writing `target` does not hold the documentation back (docair-8l6.66).
        let first = std::time::Instant::now();
        let mut until = first + QUIET;
        loop {
            let left = until
                .min(first + AT_MOST)
                .saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(Heard::File(ev)) => {
                    let paths = counted(&ev);
                    if !paths.is_empty() {
                        for p in paths {
                            if !changed.contains(&p) {
                                changed.push(p);
                            }
                        }
                        until = std::time::Instant::now() + QUIET;
                    }
                }
                Ok(Heard::Wake) => force = true,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        refresh(&changed, force);
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
