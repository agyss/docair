//! A project the CLI or a server works on: its settings, its graph, and the steps that change them
//! (an answer taken in, edits applied, the documentation written again). Shared by `docair ask`,
//! `docair apply`, `docair serve` and the server (SPEC §10).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use docair_ai::{AiConfig, Reply, ViewChange};
use docair_core::config::Config;
use docair_core::edit::{self, PatchResult};
use docair_core::model::{Change, Graph};
use docair_core::{Options, scan};
use serde::{Deserialize, Serialize};

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

use crate::ApiError;

/***
 * Add a reply's edits to the waiting ones, one edit per part (docair-b7e): an edit of a part that
 * has one waiting is merged into it. Its summary and why win; each tag it adds is added once, and
 * one it removes goes (from the waiting additions, else it is removed). An edit of a glossary text,
 * section or whole file replaces the waiting one of the same file. Returns the parts whose waiting
 * edit was replaced.
 *
 * @id docair-server.project.merge_changes
 ***/
pub fn merge_changes(waiting: &mut Vec<Change>, new: &[Change]) -> Vec<String> {
    let whole = |c: &Change| ["text", "section", "file"].contains(&c.mode.as_str());
    let mut replaced = Vec::new();
    for c in new {
        let same = waiting.iter_mut().find(|w| {
            w.node == c.node
                && whole(w) == whole(c)
                && (!whole(c) || (w.mode == c.mode && w.file == c.file))
        });
        let Some(w) = same else {
            waiting.push(c.clone());
            continue;
        };
        if !replaced.contains(&c.node) {
            replaced.push(c.node.clone());
        }
        if whole(c) {
            *w = c.clone();
            continue;
        }
        for tag in &c.remove {
            if w.add.contains(tag) {
                w.add.retain(|t| t != tag);
            } else if !w.remove.contains(tag) {
                w.remove.push(tag.clone());
            }
        }
        for tag in &c.add {
            if w.remove.contains(tag) {
                w.remove.retain(|t| t != tag);
            } else if !w.add.contains(tag) {
                w.add.push(tag.clone());
            }
        }
        if c.summary.is_some() {
            w.summary = c.summary.clone();
        }
        if !c.why.is_empty() {
            w.why = c.why.clone();
        }
        (w.mode, w.file, w.line) = (c.mode.clone(), c.file.clone(), c.line);
    }
    replaced
}

/// What `docair apply` needs from the last reply.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Pending {
    #[serde(default)]
    pub changes: Vec<Change>,
    #[serde(default)]
    pub view: Option<ViewChange>,
}

/// One project with its settings and current graph.
pub struct ProjectSession {
    pub root: PathBuf,
    pub out: PathBuf,
    pub lenient: bool,
    pub config: Config,
    pub ai: AiConfig,
    pub graph: Graph,
    pub save: bool,
    /// An upload: no indexer runs on it, since an indexer builds the code and build scripts run.
    pub untrusted: bool,
    /// `docair serve --changes <A>`: the page also shows what the working tree changes since commit A.
    pub changes: Option<String>,
}

/// AI settings that ask nothing, for a session that only scans and applies.
pub fn no_ai() -> AiConfig {
    AiConfig {
        endpoint: String::new(),
        model: String::new(),
        api_key: None,
        key_var: String::new(),
        key_source: None,
        max_tokens: 0,
    }
}

impl ProjectSession {
    /// A project scanned, with the given AI settings; `save` records walkthroughs.
    pub fn load(
        root: &Path,
        out: Option<PathBuf>,
        lenient: bool,
        save: bool,
        ai: AiConfig,
    ) -> Result<ProjectSession> {
        Self::load_with(root, out, lenient, false, save, ai)
    }

    /// As `load`; `untrusted` for an upload, which no indexer reads.
    pub fn load_with(
        root: &Path,
        out: Option<PathBuf>,
        lenient: bool,
        untrusted: bool,
        save: bool,
        ai: AiConfig,
    ) -> Result<ProjectSession> {
        let config = docair_core::config::load(root)?;
        let graph = scan(
            root,
            docair_adapters::all(),
            &Options { lenient, untrusted },
        )?;
        Ok(ProjectSession {
            root: root.to_path_buf(),
            out: out.unwrap_or_else(|| root.join(".docair")),
            lenient,
            config,
            ai,
            graph,
            save,
            untrusted,
            changes: None,
        })
    }

    pub fn rescan(&mut self) -> Result<()> {
        self.config = docair_core::config::load(&self.root)?;
        let walkthroughs = std::mem::take(&mut self.graph.walkthroughs);
        self.graph = scan(
            &self.root,
            docair_adapters::all(),
            &Options {
                lenient: self.lenient,
                untrusted: self.untrusted,
            },
        )?;
        for w in walkthroughs {
            if !self.graph.walkthroughs.iter().any(|x| x.id == w.id) {
                self.graph.walkthroughs.push(w);
            }
        }
        Ok(())
    }

    /***
     * Write the graph and the page; with `changes`, the page with its changes section, and `changes.json`. When commit A cannot be read, the page has no changes section and the reason is printed.
     *
     * @calls docair-server.commits.compare_work "compares the working tree with commit A"
     * @calls docair-server.commits.changes_page "writes the page with its changes section"
     ***/
    pub fn write_outputs(&self) -> Result<PathBuf> {
        let html = self.out.join("docair.html");
        docair_core::emit::write_json(&self.graph, &self.out.join("docair.graph.json"))?;
        if let Some(a) = &self.changes {
            match crate::commits::compare_work(&self.root, a, &self.graph, self.lenient) {
                Ok(pair) => {
                    std::fs::write(&html, crate::commits::changes_page(&pair))?;
                    std::fs::write(
                        self.out.join("changes.json"),
                        serde_json::to_string_pretty(&pair.changes)? + "\n",
                    )?;
                    return Ok(html);
                }
                Err(e) => eprintln!("docair serve: the page has no changes section: {e:#}"),
            }
        }
        docair_viewer::write_html(&self.graph, &html)?;
        Ok(html)
    }

    /// Edits of the reply applied to the files on disk, in memory.
    pub fn patch(&self, changes: &[Change]) -> PatchResult {
        let known: HashSet<String> = self.graph.nodes.iter().map(|n| n.id.clone()).collect();
        let root = self.root.clone();
        edit::patch(
            changes,
            &move |p: &str| std::fs::read_to_string(root.join(p)).ok(),
            &known,
        )
    }

    /***
     * Take a reply into the project: record its walkthrough, add its edits and view change to the
     * pending ones (a new conversation starts afresh), and write `pending.json` and `docair.patch`.
     * An edit of a part that already has one waiting is merged into it (`merge_changes`,
     * docair-b7e); the parts whose waiting edit it replaced come back. `docair.patch` holds every
     * edit waiting, what Apply writes.
     *
     * @calls docair-server.project.merge_changes "one edit per part waits"
     ***/
    pub fn take_reply(
        &mut self,
        reply: &Reply,
        fresh: bool,
    ) -> Result<(Pending, Option<PatchResult>, Vec<String>)> {
        if let Some(wt) = &reply.walkthrough {
            let mut wt = wt.clone();
            if self.save {
                let dir = self.config.walkthrough_dir().to_string();
                let path = docair_core::walkthrough::save(&self.root, &dir, &wt)?;
                wt.file = Some(format!("{}/{}.json", dir.trim_end_matches('/'), wt.id));
                println!("recorded  {}", path.display());
            }
            self.graph.walkthroughs.retain(|w| w.id != wt.id);
            self.graph.walkthroughs.push(wt);
        }
        // In a conversation, edits and view changes add up until they are applied.
        let mut pending = if fresh {
            Pending::default()
        } else {
            read_pending(&self.out).unwrap_or_default()
        };
        let replaced = merge_changes(&mut pending.changes, &reply.changes);
        if let Some(v) = &reply.view {
            let merged = pending.view.get_or_insert_with(ViewChange::default);
            for id in &v.hide {
                merged.show.retain(|x| x != id);
                if !merged.hide.contains(id) {
                    merged.hide.push(id.clone());
                }
            }
            for id in &v.show {
                merged.hide.retain(|x| x != id);
                if !merged.show.contains(id) {
                    merged.show.push(id.clone());
                }
            }
        }
        std::fs::create_dir_all(&self.out)?;
        std::fs::write(
            self.out.join("pending.json"),
            serde_json::to_string_pretty(&pending)?,
        )?;
        let result = if reply.changes.is_empty() {
            None
        } else {
            let result = self.patch(&pending.changes);
            std::fs::write(self.out.join("docair.patch"), &result.diff)?;
            Some(result)
        };
        Ok((pending, result, replaced))
    }

    /// Write edits into the files and the view change into docair.toml, then scan again; the files edited.
    pub fn apply(&mut self, pending: &Pending) -> Result<Vec<String>> {
        let mut edited = Vec::new();
        if !pending.changes.is_empty() {
            // Resolve again: the files may have changed since the reply.
            let mut changes = Vec::new();
            for c in &pending.changes {
                let mut fresh = c.clone();
                // Glossary text, a glossary section and whole files keep their mode (docs/MEANINGS.md §6).
                if !["text", "section", "file"].contains(&fresh.mode.as_str()) {
                    fresh.mode.clear();
                }
                match edit::resolve(&fresh, &self.graph.nodes) {
                    Ok(c) => changes.push(c),
                    Err(e) => eprintln!("warning: {e}"),
                }
            }
            let result = self.patch(&changes);
            for f in &result.files {
                let path = self.root.join(&f.path);
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&path, &f.after)
                    .with_context(|| format!("writing {}", path.display()))?;
                println!("edited    {}", f.path);
                edited.push(f.path.clone());
            }
            for p in &result.problems {
                eprintln!("warning: {p}");
            }
        }
        if let Some(view) = &pending.view {
            set_view(&self.root, &self.config, view)?;
            println!("edited    docair.toml [view]");
            edited.push("docair.toml".to_string());
        }
        self.rescan()?;
        let errors = self
            .graph
            .diagnostics
            .iter()
            .filter(|d| d.code.starts_with('E'))
            .count();
        println!(
            "rescanned {} nodes, {} edges, {errors} errors",
            self.graph.nodes.len(),
            self.graph.edges.len()
        );
        Ok(edited)
    }
}

pub fn read_pending(out: &Path) -> Result<Pending> {
    let path = out.join("pending.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("no edits to apply ({} is missing)", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

/// `[view]` in docair.toml: what is hidden goes into `hide`; what is shown again
/// leaves `hide`, or, when it lies inside a hidden section, goes into `show`.
fn set_view(root: &Path, config: &Config, change: &ViewChange) -> Result<()> {
    let path = root.join("docair.toml");
    let (before, after) = view_text(root, config, change);
    std::fs::write(&path, &after)?;
    if let Err(e) = docair_core::config::load(root) {
        std::fs::write(&path, before)?;
        return Err(e.context("the edited docair.toml does not parse; left unchanged"));
    }
    Ok(())
}

/// docair.toml before and after a view change, nothing written.
fn view_text(root: &Path, config: &Config, change: &ViewChange) -> (String, String) {
    let view = config.view.clone().unwrap_or_default();
    let mut hide = view.hide;
    let mut show = view.show;
    for id in &change.hide {
        show.retain(|s| s != id);
        if !hide.contains(id) {
            hide.push(id.clone());
        }
    }
    for id in &change.show {
        if hide.contains(id) {
            hide.retain(|h| h != id);
        } else if !show.contains(id) {
            show.push(id.clone());
        }
    }
    let list = |ids: &[String]| {
        ids.iter()
            .map(|h| format!("\"{h}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut wanted = vec![format!("hide = [{}]", list(&hide))];
    if !show.is_empty() {
        wanted.push(format!("show = [{}]", list(&show)));
    }
    let path = root.join("docair.toml");
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = before.lines().map(str::to_string).collect();
    match lines.iter().position(|l| l.trim() == "[view]") {
        Some(header) => {
            let mut end = lines[header + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['))
                .map_or(lines.len(), |k| header + 1 + k);
            // Drop the old hide and show lines of the table, then put the new ones first.
            let mut k = header + 1;
            while k < end {
                let t = lines[k].trim_start();
                if t.starts_with("hide") || t.starts_with("show") {
                    lines.remove(k);
                    end -= 1;
                } else {
                    k += 1;
                }
            }
            for (j, l) in wanted.into_iter().enumerate() {
                lines.insert(header + 1 + j, l);
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("# What the page shows when it opens (SPEC §11).".into());
            lines.push("[view]".into());
            lines.extend(wanted);
        }
    }
    let after = lines.join("\n") + "\n";
    (before, after)
}

// ── Serving a project (docair serve) ──

/// The project a server serves.
pub struct ServedProject {
    session: Mutex<ProjectSession>,
    /// Grows each time the documentation is written again.
    pub version: AtomicU64,
    pub ai_on: bool,
    /// The project's files as they were when last scanned: names, sizes and times.
    seen: Mutex<u64>,
}

impl std::fmt::Debug for ServedProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServedProject")
            .field("root", &self.lock().root)
            .field("version", &self.version.load(Ordering::SeqCst))
            .field("ai_on", &self.ai_on)
            .finish()
    }
}

impl ServedProject {
    /// A project scanned and its documentation written; `ai`: the settings to ask with, none for AI off.
    pub fn open(
        root: &Path,
        out: Option<PathBuf>,
        lenient: bool,
        ai: Option<AiConfig>,
    ) -> Result<ServedProject> {
        Self::open_comparing(root, out, lenient, ai, None)
    }

    /// As `open`; with `changes`, the page shows what the working tree changes since that commit, written again with each change.
    pub fn open_comparing(
        root: &Path,
        out: Option<PathBuf>,
        lenient: bool,
        ai: Option<AiConfig>,
        changes: Option<String>,
    ) -> Result<ServedProject> {
        let ai_on = ai.is_some();
        let mut session = ProjectSession::load(root, out, lenient, true, ai.unwrap_or_else(no_ai))?;
        session.changes = changes;
        session.write_outputs()?;
        let seen = files_seen(root, &session.out);
        Ok(ServedProject {
            session: Mutex::new(session),
            version: AtomicU64::new(1),
            ai_on,
            seen: Mutex::new(seen),
        })
    }

    /// An upload, scanned once as untrusted and its documentation written; it is asked about and edited like a project.
    pub fn open_upload(root: &Path, ai: Option<AiConfig>) -> Result<ServedProject> {
        let ai_on = ai.is_some();
        let session =
            ProjectSession::load_with(root, None, true, true, true, ai.unwrap_or_else(no_ai))?;
        session.write_outputs()?;
        let seen = files_seen(root, &session.out);
        Ok(ServedProject {
            session: Mutex::new(session),
            version: AtomicU64::new(1),
            ai_on,
            seen: Mutex::new(seen),
        })
    }

    /// The project's folder and its output folder.
    pub fn folders(&self) -> (PathBuf, PathBuf) {
        let s = self.lock();
        (s.root.clone(), s.out.clone())
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, ProjectSession> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The documentation written again as it is, e.g. with findings just stored; the new version.
    pub fn rewrite(&self) -> Result<u64> {
        let s = self.lock();
        self.written(&s)
    }

    /// The documentation written again; the new version.
    fn written(&self, session: &ProjectSession) -> Result<u64> {
        session.write_outputs()?;
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) =
            files_seen(&session.root, &session.out);
        Ok(self.version.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /***
     * Scan the project again and write its documentation again when its files changed since they were last scanned.
     ***/
    pub fn refresh(&self) -> Result<bool> {
        let (root, out) = {
            let s = self.lock();
            (s.root.clone(), s.out.clone())
        };
        let now = files_seen(&root, &out);
        if *self.seen.lock().unwrap_or_else(|e| e.into_inner()) == now {
            return Ok(false);
        }
        let mut s = self.lock();
        s.rescan()?;
        self.written(&s)?;
        Ok(true)
    }
}

/// A fingerprint of the project's files: every name, size and time of change, outside the outputs,
/// version control, build output and dependencies.
/// Whether a change at `path` can change the documentation: inside the project, outside the output
/// folder and outside what a scan skips (hidden folders, `target`, `node_modules`, `dist`).
pub fn watched(root: &Path, out: &Path, path: &Path) -> bool {
    if path.starts_with(out) {
        return false;
    }
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let dirs: Vec<_> = rel.components().collect();
    // The last component is the file itself; a hidden file (.env) may matter, a hidden folder never.
    !dirs[..dirs.len().saturating_sub(1)].iter().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        name.starts_with('.') || ["target", "node_modules", "dist"].contains(&name.as_ref())
    })
}

fn files_seen(root: &Path, out: &Path) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut todo = vec![root.to_path_buf()];
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    while let Some(dir) = todo.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in read.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if name.starts_with('.')
                    || ["target", "node_modules", "dist"].contains(&name.as_str())
                    || path == out
                {
                    continue;
                }
                todo.push(path);
            } else {
                let when = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                entries.push((path.to_string_lossy().to_string(), meta.len(), when));
            }
        }
    }
    entries.sort();
    for (p, len, when) in entries {
        mix(p.as_bytes());
        mix(&len.to_le_bytes());
        mix(&when.to_le_bytes());
    }
    h
}

/// The documentation the CLI wrote, never cached: it is written again while the page is open.
pub(crate) async fn page(State(config): State<crate::Config>) -> Response {
    let Some(s) = config.project.clone() else {
        return ApiError(
            StatusCode::NOT_FOUND,
            "this server serves no project".into(),
        )
        .into_response();
    };
    let path = s.lock().out.join("docair.html");
    match tokio::fs::read_to_string(&path).await {
        Ok(html) => ([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response(),
        Err(e) => ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}: {e}", path.display()),
        )
        .into_response(),
    }
}

/// `GET /api/version`: grows each time the documentation is written again; the page polls it.
#[derive(Debug, Serialize, Deserialize)]
pub struct ServeVersion {
    pub version: u64,
}

pub(crate) async fn version(State(config): State<crate::Config>) -> Json<ServeVersion> {
    Json(ServeVersion {
        version: config
            .project
            .as_ref()
            .map_or(0, |s| s.version.load(Ordering::SeqCst)),
    })
}

/// The answer to `/api/ask`, `/api/word` and `/api/glossary`.
#[derive(Debug, Serialize)]
pub struct ServeAnswer {
    reply: Reply,
    /// The documentation was written again: the page reloads.
    regenerated: bool,
    version: u64,
    /// The edits waiting to be applied, as a unified diff; empty when there are none.
    patch: String,
    /// How many edits wait to be applied (`POST /api/apply`).
    pending: usize,
    /// The reply as the page's chat shows it, written by docair-viewer (its `reply_html`).
    html: String,
}

/***
 * A reply taken into the project, the documentation written again when it brought a walkthrough, and the reply as the page's chat shows it.
 *
 * @calls docair-server.project.project-session.take_reply "takes the reply into the session"
 * @calls docair-viewer.page_ai.reply_html "writes the reply as the chat shows it"
 ***/
pub(crate) fn answered(
    s: &ServedProject,
    reply: Reply,
    fresh: bool,
) -> Result<ServeAnswer, ApiError> {
    let mut session = s.lock();
    // A view change from the chat only hides in the page, as the reader's hiding (docair-702.12):
    // it never waits to be written into docair.toml. The reply still carries it, for the page.
    let kept = Reply {
        view: None,
        ..reply.clone()
    };
    let (pending, result, replaced) = session.take_reply(&kept, fresh)?;
    let regenerated = reply.walkthrough.is_some();
    let version = if regenerated {
        s.written(&session)?
    } else {
        s.version.load(Ordering::SeqCst)
    };
    let patch = result.map(|r| r.diff).unwrap_or_default();
    let pending = pending.changes.len() + usize::from(pending.view.is_some());
    // A Reply always turns into JSON; Null would only leave the chat with no answer to show.
    let mut html = docair_viewer::page_ai::reply_html(
        &serde_json::to_value(&reply).unwrap_or_default(),
        &patch,
        pending,
    );
    // An edit merged into the one already waiting for its part says so (docair-b7e).
    if !replaced.is_empty() {
        html.push_str(&format!(
            r#"<div class="muted" data-test="replaced">This reply's edit of {} replaced the one already waiting for it.</div>"#,
            replaced
                .iter()
                .map(|n| format!(
                    "<code>{}</code>",
                    n.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(ServeAnswer {
        reply,
        regenerated,
        version,
        patch,
        pending,
        html,
    })
}

/// The answer to `POST /api/apply`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ServeApplied {
    /// The files written: the project's files the edits went into, and `docair.toml` for a view change.
    edited: Vec<String>,
    regenerated: bool,
    version: u64,
}

/***
 * `POST /api/apply`: write the waiting edits into the project's files and the view change into docair.toml, scan again, and write the documentation again.
 *
 * @calls docair-server.project.project-session.apply "writes the edits and scans again"
 ***/
pub(crate) async fn apply(
    State(config): State<crate::Config>,
) -> Result<Json<ServeApplied>, ApiError> {
    let Some(shared) = config.project.clone() else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "this server serves no project".into(),
        ));
    };
    let done = tokio::task::spawn_blocking(move || -> Result<ServeApplied, ApiError> {
        let mut session = shared.lock();
        let pending: Pending = read_pending(&session.out).unwrap_or_default();
        if pending.changes.is_empty() && pending.view.is_none() {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "there are no edits waiting to be applied".into(),
            ));
        }
        let edited = session.apply(&pending)?;
        std::fs::remove_file(session.out.join("pending.json")).ok();
        let version = shared.written(&session)?;
        Ok(ServeApplied {
            edited,
            regenerated: true,
            version,
        })
    })
    .await??;
    Ok(Json(done))
}

/// The tags the code repeats in the served project, and the patch that removes them (`GET /api/tidy`).
#[derive(Debug, Serialize)]
pub struct ServeTidy {
    pub tags: Vec<docair_core::tidy::RepeatedTag>,
    /// The edits that remove them all, as a unified diff.
    pub patch: String,
}

/// Which repeated tags to remove (`POST /api/tidy`): by `node` and `tag`; none means all.
#[derive(Debug, Default, Deserialize)]
pub struct TidyChoice {
    #[serde(default)]
    pub tags: Option<Vec<TidyPick>>,
}

/// One repeated tag picked for removal.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct TidyPick {
    pub node: String,
    pub tag: String,
}

/// The repeated tags of the session's graph, resolved into edits, and their patch.
fn tidy_edits(
    session: &ProjectSession,
    picked: Option<&[TidyPick]>,
) -> (
    Vec<docair_core::tidy::RepeatedTag>,
    Vec<Change>,
    PatchResult,
) {
    let mut found = docair_core::tidy::repeated(&session.graph);
    if let Some(p) = picked {
        found.retain(|r| p.iter().any(|t| t.node == r.node && t.tag == r.tag));
    }
    let changes: Vec<Change> = docair_core::tidy::changes(&found)
        .iter()
        .filter_map(|c| edit::resolve(c, &session.graph.nodes).ok())
        .collect();
    let result = session.patch(&changes);
    (found, changes, result)
}

/***
 * `GET /api/tidy`: the tags the code repeats in the served project, each with the code that says the same, and the patch that removes them.
 *
 * @calls docair-core.tidy.repeated "finds the tags the code repeats"
 ***/
pub(crate) async fn tidy_list(
    State(config): State<crate::Config>,
) -> Result<Json<ServeTidy>, ApiError> {
    let shared = config.project.clone().ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "this server serves no project".into(),
        )
    })?;
    let answer = tokio::task::spawn_blocking(move || {
        let session = shared.lock();
        let (tags, _, result) = tidy_edits(&session, None);
        ServeTidy {
            tags,
            patch: result.diff,
        }
    })
    .await?;
    Ok(Json(answer))
}

/***
 * `POST /api/tidy`: remove the repeated tags picked (all when none are named), scan again and write the documentation again.
 *
 * @calls docair-core.tidy.changes "turns the picked tags into edits"
 * @calls docair-server.project.project-session.apply "writes the edits and scans again"
 ***/
pub(crate) async fn tidy_apply(
    State(config): State<crate::Config>,
    body: Option<Json<TidyChoice>>,
) -> Result<Json<ServeApplied>, ApiError> {
    let shared = config.project.clone().ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "this server serves no project".into(),
        )
    })?;
    let choice = body.map(|Json(c)| c).unwrap_or_default();
    let done = tokio::task::spawn_blocking(move || -> Result<ServeApplied, ApiError> {
        let mut session = shared.lock();
        let (found, changes, _) = tidy_edits(&session, choice.tags.as_deref());
        if found.is_empty() {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "no tag repeats the code".into(),
            ));
        }
        let edited = session.apply(&Pending {
            changes,
            view: None,
        })?;
        let version = shared.written(&session)?;
        Ok(ServeApplied {
            edited,
            regenerated: true,
            version,
        })
    })
    .await??;
    Ok(Json(done))
}

/// `POST /api/explain`: which commits to compare; without `b`, the working tree.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ExplainRequest {
    pub a: String,
    #[serde(default)]
    pub b: Option<String>,
}

// ── Explain jobs (docs/EXPLAIN.md §8, docair-8l6.11) ──

/// A job is kept this long after it ends, so a page that reloaded still finds it.
const JOB_KEPT_FOR: std::time::Duration = std::time::Duration::from_secs(600);

/// An explain job as `GET /api/explain/<job>` answers it.
#[derive(Debug, Clone, Serialize)]
pub struct ExplainJob {
    pub job: String,
    pub a: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
    pub running: bool,
    /// `comparing` (reading the commits), `asking` (the model), `done` or `failed`.
    pub phase: String,
    #[serde(flatten)]
    pub progress: docair_ai::explain::ExplainProgress,
    /// The time so far, or the time it took.
    pub seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the page shows, as it is: *The AI reviews 30 pieces in 2 requests · 1 answered · 1:12*.
    pub text: String,
    /// The same with a bar of the requests answered and the changes made so far, as the page shows
    /// it while the job runs (docair-8l6.65).
    pub html: String,
    /// The documentation's version once it was written again with the findings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// The findings, once the job is done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings: Option<serde_json::Value>,
}

struct JobState {
    answer: ExplainJob,
    started: std::time::Instant,
    ended: Option<std::time::Instant>,
}

impl JobState {
    /// The answer as it is now: its time and words worked out at this moment.
    fn now(&self) -> ExplainJob {
        let took = self.ended.unwrap_or_else(std::time::Instant::now) - self.started;
        let mut a = self.answer.clone();
        a.seconds = took.as_secs();
        a.text = job_text(&a);
        a.html = job_html(&a);
        a
    }
}

/// Text as HTML, what is in backticks as code: `` `orders.audit` `` → `<code>orders.audit</code>`.
fn code_html(t: &str) -> String {
    let t = t
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    if t.matches('`').count() % 2 == 1 {
        return t;
    }
    t.split('`')
        .enumerate()
        .map(|(i, x)| {
            if i % 2 == 1 {
                format!("<code>{x}</code>")
            } else {
                x.to_string()
            }
        })
        .collect()
}

/***
 * What the page shows of a running job (docair-8l6.65): a bar of the requests answered with the job's
 * words, then each change made so far, as soon as it is made: its title, how many pieces, what was
 * found off, why.
 ***/
fn job_html(j: &ExplainJob) -> String {
    let p = &j.progress;
    let mut out = format!(
        r#"<div class="explain-now"><progress max="{}" value="{}"></progress> <span data-test="explain-text">{}</span></div>"#,
        p.requests.max(1),
        p.answered.min(p.requests.max(1)),
        code_html(&j.text)
    );
    if !p.found.is_empty() && j.phase != "done" {
        out.push_str(&format!(
            r#"<p class="muted">{} so far; the last request may still merge them and add pieces.</p><ol class="review-list" data-test="review-so-far">"#,
            count(p.found.len(), "change", "changes")
        ));
        for c in &p.found {
            let off = if c.off.is_empty() {
                " · ✓ nothing found off".to_string()
            } else {
                format!(" · ⚠ {} found off", c.off.len())
            };
            out.push_str(&format!(
                r#"<li><b>{}</b> <span class="muted">· {}{off}</span>{}</li>"#,
                code_html(&c.title),
                count(c.how.len(), "piece", "pieces"),
                if c.why.is_empty() {
                    String::new()
                } else {
                    format!(r#"<div class="muted">{}</div>"#, code_html(&c.why))
                }
            ));
        }
        out.push_str("</ol>");
    }
    out
}

/// `n thing` or `n things`.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What the page says about a job.
fn job_text(j: &ExplainJob) -> String {
    let time = format!("{}:{:02}", j.seconds / 60, j.seconds % 60);
    let p = &j.progress;
    match j.phase.as_str() {
        "comparing" => format!(
            "Comparing {} with {} · {time}",
            j.a,
            j.b.as_deref().unwrap_or("the working tree")
        ),
        "failed" => format!(
            "The AI could not explain the changes: {}",
            j.error.as_deref().unwrap_or("it failed")
        ),
        _ if p.pieces == 0 => "Every change is reviewed already".to_string(),
        "done" => {
            let found = j.findings.as_ref();
            // Every piece is a change (docair-8l6.63): a piece the AI put in none counts as one of its own.
            let len = |k: &str| found.map_or(0, |f| f[k].as_array().map_or(0, Vec::len));
            format!(
                "Reviewed: {} in {} · {time}",
                count(len("changes") + len("other"), "change", "changes"),
                count(p.requests, "request", "requests")
            )
        }
        _ => {
            let mut t = format!(
                "The AI reviews {} in {} · {} answered · {time}",
                count(p.pieces - p.left, "piece", "pieces"),
                count(p.requests, "request", "requests"),
                p.answered
            );
            if p.left > 0 {
                t.push_str(&format!(
                    " · {} left for later",
                    count(p.left, "piece", "pieces")
                ));
            }
            t
        }
    }
}

/// The explain jobs of a server: one runs at a time, each kept a while after it ends.
#[derive(Default)]
pub struct ExplainJobs {
    jobs: Mutex<Vec<std::sync::Arc<Mutex<JobState>>>>,
}

impl std::fmt::Debug for ExplainJobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.jobs.lock().map_or(0, |j| j.len());
        f.debug_struct("ExplainJobs").field("jobs", &n).finish()
    }
}

impl ExplainJobs {
    fn kept(&self) -> std::sync::MutexGuard<'_, Vec<std::sync::Arc<Mutex<JobState>>>> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.retain(|j| {
            let s = j.lock().unwrap_or_else(|e| e.into_inner());
            s.ended.is_none_or(|t| t.elapsed() < JOB_KEPT_FOR)
        });
        jobs
    }

    /// The job with this id, as it is now.
    pub fn get(&self, id: &str) -> Option<ExplainJob> {
        self.kept().iter().find_map(|j| {
            let s = j.lock().unwrap_or_else(|e| e.into_inner());
            (s.answer.job == id).then(|| s.now())
        })
    }
}

/***
 * `POST /api/explain`: start explaining what the served project changes since commit `a` (or between `a` and `b`), and answer the job at once (202); `GET /api/explain/<job>` says how far it is. A second call while a job runs answers that job (the same commits) or 409 (others). A project without git history (an upload) has no commits to compare: 409; AI off: 503.
 *
 * @calls docair-server.project.run_job "explains in the background"
 ***/
pub(crate) async fn explain(
    State(config): State<crate::Config>,
    Json(req): Json<ExplainRequest>,
) -> Result<Response, ApiError> {
    let shared = config.project.clone().ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "this server serves no project".into(),
        )
    })?;
    let root = shared.lock().root.clone();
    if crate::commits::git(&root, &["rev-parse", "--show-toplevel"]).is_err() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "no commits to compare: this project has no git history (an upload)".into(),
        ));
    }
    let Some(ai) = config.ai.clone() else {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "AI is off: the server has no DOCAIR_AI_ENDPOINT and DOCAIR_AI_MODEL".into(),
        ));
    };
    let job = {
        let mut jobs = config.explains.kept();
        let running = jobs.iter().find_map(|j| {
            let s = j.lock().unwrap_or_else(|e| e.into_inner());
            s.answer.running.then(|| s.now())
        });
        if let Some(r) = running {
            if r.a == req.a && r.b == req.b {
                return Ok((StatusCode::ACCEPTED, Json(r)).into_response());
            }
            return Err(ApiError(
                StatusCode::CONFLICT,
                format!("an explain job runs already ({}): {}", r.job, r.text),
            ));
        }
        let job = std::sync::Arc::new(Mutex::new(JobState {
            answer: ExplainJob {
                job: upload_id(),
                a: req.a.clone(),
                b: req.b.clone(),
                running: true,
                phase: "comparing".into(),
                progress: Default::default(),
                seconds: 0,
                error: None,
                text: String::new(),
                html: String::new(),
                version: None,
                findings: None,
            },
            started: std::time::Instant::now(),
            ended: None,
        }));
        jobs.push(job.clone());
        job
    };
    let answer = job.lock().unwrap_or_else(|e| e.into_inner()).now();
    tokio::task::spawn_blocking(move || run_job(&job, &shared, &ai, &req));
    Ok((StatusCode::ACCEPTED, Json(answer)).into_response())
}

/***
 * `GET /api/explain/<job>`: how far an explain job is, its words for the page, and at the end its findings and the documentation's new version; 404 for a job no longer kept.
 ***/
pub(crate) async fn explain_job(
    State(config): State<crate::Config>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ExplainJob>, ApiError> {
    config.explains.get(&id).map(Json).ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            "this explain job is no longer kept; explain again".into(),
        )
    })
}

/***
 * An explain job: compare the commits (the working tree as B is the graph the server already has), send the pieces no stored change holds to the model with the intent (docs/EXPLAIN.md §9), store the review as `docair explain` does, and write the page again when it shows the changes; each step goes into the job.
 *
 * @calls docair-server.commits.compare "compares the commits"
 * @calls docair-ai.explain.explain_with "reviews the change, saying how far it is"
 ***/
fn run_job(
    job: &std::sync::Arc<Mutex<JobState>>,
    shared: &std::sync::Arc<ServedProject>,
    ai: &AiConfig,
    req: &ExplainRequest,
) {
    let set =
        |f: &dyn Fn(&mut ExplainJob)| f(&mut job.lock().unwrap_or_else(|e| e.into_inner()).answer);
    let done = (|| -> Result<(serde_json::Value, Option<u64>)> {
        let (root, lenient, comparing) = {
            let s = shared.lock();
            (s.root.clone(), s.lenient, s.changes.is_some())
        };
        let pair = match req.b.as_deref() {
            None => {
                let work = shared.lock().graph.clone();
                crate::commits::compare_work(&root, &req.a, &work, lenient)
            }
            Some(b) => crate::commits::compare(&root, &req.a, Some(b), lenient),
        }?;
        set(&|j| j.phase = "asking".into());
        let settings = docair_core::config::load(&pair.root)
            .map(|c| c.explain.unwrap_or_default())
            .unwrap_or_default();
        let file = crate::commits::findings_file(&pair);
        let kept: Option<docair_ai::explain::ChangeFindings> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
        let max = settings
            .max_requests
            .unwrap_or(docair_ai::explain::MAX_REQUESTS);
        let found = docair_ai::explain::explain_with(
            ai,
            (&pair.a, &pair.b),
            &pair.changes,
            &pair.intent,
            kept.as_ref(),
            max,
            &|p| set(&|j| j.progress = p.clone()),
        )?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&file, serde_json::to_string_pretty(&found)? + "\n")?;
        // A page with a changes section shows the findings: it is written again, and reloads.
        let version = if comparing {
            Some(shared.rewrite()?)
        } else {
            None
        };
        Ok((serde_json::to_value(&found)?, version))
    })();
    let mut s = job.lock().unwrap_or_else(|e| e.into_inner());
    s.ended = Some(std::time::Instant::now());
    s.answer.running = false;
    match done {
        Ok((findings, version)) => {
            s.answer.phase = "done".into();
            s.answer.findings = Some(findings);
            s.answer.version = version;
        }
        Err(e) => {
            s.answer.phase = "failed".into();
            s.answer.error = Some(format!("{e:#}"));
        }
    }
}

// ── Uploads kept as projects ──

/// At most this many uploads are kept; the one used longest ago goes first.
const KEPT_UPLOADS: usize = 8;
/// An upload not asked about for this long is dropped, with its folder.
const KEPT_FOR: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/***
 * The uploads a server keeps as projects, so the page of an upload can ask the AI and apply edits like a page `docair serve` serves (docs/SLICES.md §8).
 *
 * Each has an id of 32 hex digits that is hard to guess; the server has no accounts, so the id only keeps the uploads apart. A kept upload's folder is deleted when it is dropped.
 ***/
#[derive(Default)]
pub struct KeptUploads {
    kept: Mutex<Vec<KeptUpload>>,
}

struct KeptUpload {
    id: String,
    project: std::sync::Arc<ServedProject>,
    used: std::time::Instant,
    _folder: tempfile::TempDir,
}

impl std::fmt::Debug for KeptUploads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.kept.lock().map_or(0, |k| k.len());
        f.debug_struct("KeptUploads").field("kept", &n).finish()
    }
}

impl KeptUploads {
    /// Keep an upload and its folder; its id.
    pub fn keep(
        &self,
        folder: tempfile::TempDir,
        project: std::sync::Arc<ServedProject>,
    ) -> String {
        self.keep_as(folder, project, None)
    }

    /// As `keep`, the id starting with `pod` and a `-` when there is more than one server pod (docs/SLICES.md §8).
    pub fn keep_as(
        &self,
        folder: tempfile::TempDir,
        project: std::sync::Arc<ServedProject>,
        pod: Option<&str>,
    ) -> String {
        let id = match pod {
            Some(p) => format!("{p}-{}", upload_id()),
            None => upload_id(),
        };
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        sweep(&mut kept);
        while kept.len() >= KEPT_UPLOADS {
            let oldest = kept
                .iter()
                .enumerate()
                .min_by_key(|(_, k)| k.used)
                .map(|(i, _)| i)
                .expect("not empty");
            kept.remove(oldest);
        }
        kept.push(KeptUpload {
            id: id.clone(),
            project,
            used: std::time::Instant::now(),
            _folder: folder,
        });
        id
    }

    /// The upload with this id, if it is still kept; it counts as used now.
    pub fn get(&self, id: &str) -> Option<std::sync::Arc<ServedProject>> {
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        sweep(&mut kept);
        let k = kept.iter_mut().find(|k| k.id == id)?;
        k.used = std::time::Instant::now();
        Some(k.project.clone())
    }
}

/// Drop the uploads not used for `KEPT_FOR`.
fn sweep(kept: &mut Vec<KeptUpload>) {
    kept.retain(|k| k.used.elapsed() < KEPT_FOR);
}

/// 128 bits from the standard library's randomly seeded hasher, the time and a counter.
fn upload_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let part = |salt: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(n);
        h.write_u128(t);
        h.write_u64(salt);
        h.finish()
    };
    format!("{:016x}{:016x}", part(1), part(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_changes_edit_docair_toml_in_place() {
        let dir = std::env::temp_dir().join(format!("docair-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let toml = "system = \"x\"\n\n[view]\ntests = false\n\n[ai]\nmodel = \"m\"\n";
        std::fs::write(dir.join("docair.toml"), toml).unwrap();
        let config = docair_core::config::load(&dir).unwrap();
        let change = ViewChange {
            hide: vec!["terraform".into(), "web".into()],
            show: vec![],
        };
        set_view(&dir, &config, &change).unwrap();
        let after = std::fs::read_to_string(dir.join("docair.toml")).unwrap();
        assert_eq!(
            after,
            "system = \"x\"\n\n[view]\nhide = [\"terraform\", \"web\"]\ntests = false\n\n[ai]\nmodel = \"m\"\n"
        );
        let config = docair_core::config::load(&dir).unwrap();
        set_view(
            &dir,
            &config,
            &ViewChange {
                hide: vec![],
                show: vec!["web".into(), "terraform.kind".into()],
            },
        )
        .unwrap();
        let after = std::fs::read_to_string(dir.join("docair.toml")).unwrap();
        assert!(
            after.contains(
                "[view]\nhide = [\"terraform\"]\nshow = [\"terraform.kind\"]\ntests = false"
            ),
            "a part inside a hidden section goes into show: {after}"
        );

        std::fs::write(dir.join("docair.toml"), "system = \"x\"\n").unwrap();
        let config = docair_core::config::load(&dir).unwrap();
        set_view(&dir, &config, &change).unwrap();
        let after = std::fs::read_to_string(dir.join("docair.toml")).unwrap();
        assert!(
            after.ends_with("[view]\nhide = [\"terraform\", \"web\"]\n"),
            "{after}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn tiny() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("docair.toml"), "system = \"tiny\"\n").unwrap();
        std::fs::write(
            dir.path().join("README.md"),
            "# tiny\n\n```docair\nThe app.\n\n@id      app\n@kind    module\n@part-of tiny\n```\n",
        )
        .unwrap();
        dir
    }

    async fn call(
        router: axum::Router,
        req: axum::http::Request<axum::body::Body>,
    ) -> (StatusCode, String) {
        use tower::ServiceExt;
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    fn served(dir: &Path) -> (std::sync::Arc<ServedProject>, axum::Router) {
        let project = std::sync::Arc::new(ServedProject::open(dir, None, true, None).unwrap());
        let config = crate::Config {
            project: Some(project.clone()),
            ..Default::default()
        };
        (project, crate::router(config))
    }

    /// A change counts when it is in the project, outside the outputs and outside what a scan skips.
    #[test]
    fn changes_in_outputs_and_skipped_folders_do_not_count() {
        let (root, out) = (Path::new("/p"), Path::new("/p/.docair"));
        assert!(watched(root, out, Path::new("/p/src/lib.rs")));
        assert!(watched(root, out, Path::new("/p/.env")));
        assert!(!watched(root, out, Path::new("/p/.docair/docair.html")));
        assert!(!watched(root, out, Path::new("/p/target/debug/x")));
        assert!(!watched(root, out, Path::new("/p/.git/index")));
        assert!(!watched(root, out, Path::new("/p/web/node_modules/a/b.js")));
        assert!(!watched(root, out, Path::new("/elsewhere/a.rs")));
    }

    /// Two replies edit one part: one edit waits, with the newer summary and each tag once; an
    /// edit of another part waits beside it (docair-b7e).
    #[test]
    fn edits_of_one_part_wait_as_one() {
        let edit = |node: &str, add: &[&str], remove: &[&str], summary: Option<&str>| Change {
            node: node.into(),
            mode: "insert".into(),
            file: "src/main.rs".into(),
            line: 22,
            add: add.iter().map(|t| t.to_string()).collect(),
            remove: remove.iter().map(|t| t.to_string()).collect(),
            summary: summary.map(str::to_string),
            why: "w".into(),
        };
        let mut waiting = vec![edit(
            "cli",
            &["@out args: Cli to main", "@part-of cli"],
            &[],
            Some("Old."),
        )];
        let replaced = merge_changes(
            &mut waiting,
            &[
                edit(
                    "cli",
                    &["@part-of cli", "@see main"],
                    &["@out args: Cli to main"],
                    Some("New."),
                ),
                edit("main", &["@calls cli"], &[], None),
            ],
        );
        assert_eq!(replaced, ["cli"]);
        assert_eq!(waiting.len(), 2, "{waiting:#?}");
        assert_eq!(waiting[0].summary.as_deref(), Some("New."));
        assert_eq!(waiting[0].add, ["@part-of cli", "@see main"]);
        assert!(
            waiting[0].remove.is_empty(),
            "a tag only proposed is dropped, not removed"
        );
        assert_eq!(waiting[1].node, "main");
        // Applied, the merged edit writes one block.
        let src = "fn a() {}\n".repeat(21) + "pub struct Cli;\n";
        let known: HashSet<String> = ["cli", "main"].iter().map(|s| s.to_string()).collect();
        let result = edit::patch(&waiting[..1], &|_: &str| Some(src.clone()), &known);
        assert_eq!(result.files.len(), 1, "{:?}", result.problems);
        assert_eq!(
            result.files[0].after.matches("/***").count(),
            1,
            "{}",
            result.files[0].after
        );
    }

    /// A running review shows a bar of the requests answered and each change as soon as it is made (docair-8l6.65).
    #[test]
    fn a_running_review_shows_its_changes_so_far() {
        let change = docair_ai::explain::ReviewChange {
            title: "Checkout reaches `orders.audit`.".into(),
            how: vec![docair_ai::explain::HowStep {
                piece: "path:orders.audit".into(),
                words: "x".into(),
            }],
            why: "Auditors need a trail.".into(),
            off: vec![docair_ai::explain::FoundOff {
                kind: "risk".into(),
                source: "none".into(),
                place: String::new(),
                says: String::new(),
                code: "Nothing tests it.".into(),
                parts: vec![],
            }],
            check: String::new(),
            recorded: docair_ai::explain::Recorded {
                by: "model".into(),
                model: Some("m".into()),
                at: "t".into(),
            },
        };
        let mut job = ExplainJob {
            job: "j".into(),
            a: "HEAD~1".into(),
            b: None,
            running: true,
            phase: "asking".into(),
            progress: docair_ai::explain::ExplainProgress {
                pieces: 4,
                requests: 2,
                started: 2,
                answered: 1,
                found: vec![change],
                ..Default::default()
            },
            seconds: 5,
            error: None,
            text: String::new(),
            html: String::new(),
            version: None,
            findings: None,
        };
        job.text = job_text(&job);
        let html = job_html(&job);
        assert!(
            html.contains(r#"<progress max="2" value="1"></progress>"#),
            "{html}"
        );
        assert!(html.contains("1 answered"), "{html}");
        assert!(
            html.contains(r#"<ol class="review-list" data-test="review-so-far"><li><b>Checkout reaches <code>orders.audit</code>.</b> <span class="muted">· 1 piece · ⚠ 1 found off</span><div class="muted">Auditors need a trail.</div></li></ol>"#),
            "{html}"
        );
        job.phase = "done".into();
        assert!(
            !job_html(&job).contains("review-so-far"),
            "done, the page reloads with the review"
        );
    }

    /// An answer comes with the HTML the page's chat shows, written by docair-viewer.
    #[test]
    fn an_answer_comes_with_the_chats_html() {
        let dir = tiny();
        let (project, _) = served(dir.path());
        let reply = Reply {
            answer: "The app <is> one module.".to_string(),
            walkthrough: None,
            view: None,
            changes: Vec::new(),
            wording: Vec::new(),
            glossary: Vec::new(),
            problems: vec!["`ghost` is no part".to_string()],
            usage: docair_ai::Usage {
                input: 0,
                output: 0,
            },
            seconds: 0.0,
            requests: 1,
        };
        let a = answered(&project, reply, true).unwrap();
        assert!(!a.regenerated && a.pending == 0 && a.patch.is_empty());
        assert!(
            a.html
                .contains(r#"<p class="answer">The app &lt;is&gt; one module.</p>"#),
            "{}",
            a.html
        );
        assert!(a.html.contains("1 parts left out"), "{}", a.html);
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["html"].as_str(), Some(a.html.as_str()));
    }

    /// A view change from the chat only hides in the page: nothing waits, docair.toml stays as it is.
    #[test]
    fn a_view_change_from_the_chat_only_hides_in_the_page() {
        let dir = tiny();
        let (project, _) = served(dir.path());
        let reply = Reply {
            answer: "Hid the app.".to_string(),
            walkthrough: None,
            view: Some(ViewChange {
                hide: vec!["app".to_string()],
                show: Vec::new(),
            }),
            changes: Vec::new(),
            wording: Vec::new(),
            glossary: Vec::new(),
            problems: Vec::new(),
            usage: docair_ai::Usage {
                input: 0,
                output: 0,
            },
            seconds: 0.0,
            requests: 1,
        };
        let a = answered(&project, reply, true).unwrap();
        assert_eq!(a.pending, 0);
        assert!(a.patch.is_empty(), "{}", a.patch);
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(
            json["reply"]["view"]["hide"][0], "app",
            "the page gets the view change"
        );
        assert!(read_pending(&project.lock().out).map_or(true, |p| p.view.is_none()));
        let toml = std::fs::read_to_string(dir.path().join("docair.toml")).unwrap();
        assert!(!toml.contains("[view]"), "{toml}");
    }

    /// The tags the code repeats are listed, removed on request, and the documentation written again.
    #[tokio::test]
    async fn tidy_lists_and_removes_the_tags_the_code_repeats() {
        use axum::body::Body;
        use axum::http::Request;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("docair.toml"), "system = \"tidyp\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let block = |id: &str, calls: &str| {
            format!(
                "/***\n * {id}.\n *\n * @id      app.{id}\n * @kind    function\n * @part-of tidyp\n{calls} ***/\n"
            )
        };
        let lib = format!(
            "{}pub fn run() {{\n    load();\n}}\n\n{}pub fn load() {{}}\n",
            block("run", " * @calls   app.load\n"),
            block("load", "")
        );
        std::fs::write(dir.path().join("src").join("lib.rs"), &lib).unwrap();
        let (project, router) = served(dir.path());
        let get = || Request::get("/api/tidy").body(Body::empty()).unwrap();
        let (status, json) = call(router.clone(), get()).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let listed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(listed["tags"][0]["tag"], "@calls app.load", "{json}");
        assert!(
            listed["patch"]
                .as_str()
                .unwrap()
                .contains("- * @calls   app.load"),
            "{json}"
        );
        let post = || {
            Request::post("/api/tidy")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap()
        };
        let (status, json) = call(router.clone(), post()).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert!(json.contains("src/lib.rs"), "{json}");
        let after = std::fs::read_to_string(dir.path().join("src").join("lib.rs")).unwrap();
        assert!(!after.contains("@calls"), "{after}");
        assert!(
            project.version.load(Ordering::SeqCst) >= 2,
            "the documentation was written again"
        );
        let (_, json) = call(router.clone(), get()).await;
        assert!(json.contains(r#""tags":[]"#), "{json}");
        let (status, _) = call(router, post()).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    /// `--changes` on a folder without git history: the page is written without a changes section, and says why.
    #[test]
    fn a_page_without_commits_has_no_changes_section() {
        let dir = tiny();
        let project =
            ServedProject::open_comparing(dir.path(), None, true, None, Some("HEAD~1".into()))
                .unwrap();
        let html = std::fs::read_to_string(dir.path().join(".docair").join("docair.html")).unwrap();
        assert!(html.contains("docair-packed"), "the page is written");
        assert!(
            !html.contains(r#"id="btn-changes""#),
            "with no changes section"
        );
        assert_eq!(
            project.rewrite().unwrap(),
            2,
            "written again, a new version"
        );
    }

    /// docair-8l6.11: POST /api/explain answers a job at once; the job compares, asks a model
    /// (a fake one here) and says how far it is; at the end it holds the review, the store has
    /// it, and the page shown with --changes is written again. A second POST while it runs
    /// answers the same job.
    #[tokio::test(flavor = "multi_thread")]
    async fn explaining_runs_as_a_job_and_says_how_far_it_is() {
        use axum::body::Body;
        use axum::http::Request;
        // A fake model: every piece it is asked about makes one change, "the app keeps the order".
        let model = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(|Json(req): Json<serde_json::Value>| async move {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let user = req["messages"][1]["content"].as_str().unwrap_or("").to_string();
                let input: serde_json::Value =
                    serde_json::from_str(&user[user.find('{').unwrap_or(0)..]).unwrap_or_default();
                let how: Vec<serde_json::Value> = input["pieces"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| serde_json::json!({ "piece": p["id"], "words": "This piece keeps the order." }))
                    .collect();
                let content = serde_json::json!({ "links": [], "changes": [{ "title": "The app now keeps the order.", "how": how }] }).to_string();
                Json(serde_json::json!({
                    "choices": [{ "message": { "content": content } }],
                    "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(axum::serve(listener, model).into_future());
        let ai = AiConfig {
            endpoint,
            model: "fake".into(),
            max_tokens: 100,
            ..no_ai()
        };
        // A project in git: commit A, then a new step in the working tree.
        let dir = tiny();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args([
                    "-c",
                    "user.name=docair test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        let readme = |extra: &str| {
            format!(
                "# tiny\n\n```docair\nThe user.\n\n@id      user\n@kind    external\n@part-of tiny\n@calls   app\n@flow    buy 1 \"clicks Buy\"\n```\n\n```docair\nThe app.\n\n@id      app\n@kind    module\n@part-of tiny\n@flow    buy 2 \"totals the cart\"\n{extra}```\n"
            )
        };
        std::fs::write(dir.path().join("README.md"), readme("")).unwrap();
        git(&["init", "-q", "-b", "main"]);
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "a"]);
        std::fs::write(
            dir.path().join("README.md"),
            readme("@calls   orders\n```\n\n```docair\nThe orders.\n\n@id      orders\n@kind    module\n@part-of tiny\n@flow    buy 3 \"keeps the order\"\n"),
        )
        .unwrap();
        let project = std::sync::Arc::new(
            ServedProject::open_comparing(
                dir.path(),
                None,
                true,
                Some(ai.clone()),
                Some("HEAD".into()),
            )
            .unwrap(),
        );
        let router = crate::router(crate::Config {
            ai: Some(ai),
            project: Some(project.clone()),
            ..Default::default()
        });
        let post = || {
            Request::post("/api/explain")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"a":"HEAD"}"#))
                .unwrap()
        };
        let (status, body) = call(router.clone(), post()).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let first: serde_json::Value = serde_json::from_str(&body).unwrap();
        let id = first["job"].as_str().unwrap().to_string();
        assert_eq!(first["running"], true);
        let (_, again) = call(router.clone(), post()).await;
        let again: serde_json::Value = serde_json::from_str(&again).unwrap();
        assert_eq!(
            again["job"], first["job"],
            "a second POST while it runs answers the same job"
        );
        let mut seen_asking = false;
        let mut job = serde_json::Value::Null;
        for _ in 0..300 {
            let (status, body) = call(
                router.clone(),
                Request::get(format!("/api/explain/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            job = serde_json::from_str(&body).unwrap();
            if job["phase"] == "asking" {
                seen_asking = true;
                assert!(
                    job["text"]
                        .as_str()
                        .unwrap()
                        .starts_with("The AI reviews 1 piece in 1 request"),
                    "{job}"
                );
            }
            if job["running"] == false {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(job["phase"], "done", "{job}");
        assert!(seen_asking, "the job said it was asking the model");
        assert!(
            job["text"]
                .as_str()
                .unwrap()
                .starts_with("Reviewed: 1 change in 1 request"),
            "{job}"
        );
        assert_eq!(
            job["findings"]["changes"][0]["title"],
            "The app now keeps the order."
        );
        assert!(
            job["version"].as_u64().unwrap() >= 2,
            "the page was written again: {job}"
        );
        let stored = std::fs::read_dir(dir.path().join("docs/changes"))
            .unwrap()
            .count();
        assert_eq!(stored, 1, "the findings are stored");
        // Asked again, nothing is new: the store covers every piece.
        let (_, body) = call(router.clone(), post()).await;
        let next: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_ne!(
            next["job"], first["job"],
            "a new job once the first has ended"
        );
        let (status, _) = call(
            router,
            Request::get("/api/explain/0123456789abcdef")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// A project without git history (an upload, or a folder) has no commits to compare.
    #[tokio::test]
    async fn explaining_needs_commits_to_compare() {
        use axum::body::Body;
        use axum::http::Request;
        let dir = tiny();
        let (_, router) = served(dir.path());
        let (status, body) = call(
            router,
            Request::post("/api/explain")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"a":"HEAD~1"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(body.contains("no commits to compare"), "{body}");
    }

    #[tokio::test]
    async fn a_project_is_served_and_written_again_when_its_files_change() {
        use axum::body::Body;
        use axum::http::Request;
        let dir = tiny();
        let (project, router) = served(dir.path());
        let (status, html) = call(
            router.clone(),
            Request::get("/").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.to_lowercase().contains("<html"), "a page");
        let (status, _) = call(
            router.clone(),
            Request::get("/docair.graph.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the other outputs are served at their paths"
        );
        let (_, v) = call(
            router.clone(),
            Request::get("/api/version").body(Body::empty()).unwrap(),
        )
        .await;
        let v: ServeVersion = serde_json::from_str(&v).unwrap();
        assert!(
            !project.refresh().unwrap(),
            "nothing changed, nothing written"
        );
        std::fs::write(
            dir.path().join("NOTES.md"),
            "# notes\n\n```docair\nThe notes.\n\n@id      notes\n@kind    module\n@part-of tiny\n```\n",
        )
        .unwrap();
        assert!(
            project.refresh().unwrap(),
            "a new file: scanned and written again"
        );
        let (_, w) = call(
            router,
            Request::get("/api/version").body(Body::empty()).unwrap(),
        )
        .await;
        let w: ServeVersion = serde_json::from_str(&w).unwrap();
        assert_eq!(w.version, v.version + 1);
        assert!(project.lock().graph.nodes.iter().any(|n| n.id == "notes"));
    }

    #[tokio::test]
    async fn without_ai_asking_says_so_and_applying_nothing_is_refused() {
        use axum::body::Body;
        use axum::http::Request;
        let dir = tiny();
        let (_, router) = served(dir.path());
        let ask = Request::post("/api/ask")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"question":"What is it?"}"#))
            .unwrap();
        let (status, body) = call(router.clone(), ask).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        let (status, _) = call(
            router,
            Request::post("/api/apply").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn a_server_without_a_project_needs_the_graph() {
        use axum::body::Body;
        use axum::http::Request;
        let mut config = crate::Config::default();
        config.ai = docair_ai::AiConfig::resolve(
            Some(&docair_core::config::AiSettings {
                endpoint: Some("http://127.0.0.1:9/v1".into()),
                model: Some("m".into()),
                ..Default::default()
            }),
            &docair_ai::Env::default(),
        );
        let router = crate::router(config);
        let ask = Request::post("/api/ask")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"question":"What is it?"}"#))
            .unwrap();
        let (status, body) = call(router.clone(), ask).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, _) = call(
            router,
            Request::get("/api/version").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "no project, no version route"
        );
    }
}
