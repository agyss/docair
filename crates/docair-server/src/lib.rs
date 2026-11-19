//! docair-server: generate a docair page from uploaded source files.
//!
//! `POST /api/scan` takes a multipart upload (one part per file, the part's
//! file name is the path inside the uploaded folder), runs the docair
//! pipeline on it and answers with the summary and the generated page.
//! `POST /api/ask` asks the configured AI model about a graph (SPEC §10.5).

/***
 * A project the CLI or a server works on: its graph, an answer taken in, edits applied, the documentation written again.
 ***/
pub mod project;

/***
 * Two commits of a project compared: their graphs from git worktrees, kept in the cache, and the flows the change touches.
 ***/
pub mod commits;

/***
 * More than one server pod: an upload's id names the pod that keeps it, and the other pods forward its requests there.
 ***/
pub mod peers;

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Multipart, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use docair_ai::{AiConfig, Env, Turn};
use docair_core::model::{Change, Diagnostic, Graph, Severity};
use docair_core::{Options, scan};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tower_http::services::{ServeDir, ServeFile};

/// At most this many questions are with the model at the same time.
const ASKING_AT_ONCE: usize = 2;
/// Longer questions are refused.
const MAX_QUESTION_CHARS: usize = 500;

/// Server settings. In Kubernetes they come from the `docair-server` ConfigMap.
#[derive(Debug, Clone)]
pub struct Config {
    pub max_upload_bytes: usize,
    pub max_files: usize,
    /// Serve the built frontend from here too (local runs without Kubernetes).
    pub static_dir: Option<PathBuf>,
    /// The model for `/api/ask`; `None` turns asking off.
    pub ai: Option<AiConfig>,
    /// The project `docair serve` serves: its documentation at `/`, and the graph the AI routes use
    /// when a request brings none.
    pub project: Option<Arc<project::ServedProject>>,
    /// Uploads kept as projects, so their pages can ask the AI and apply edits (`/api/projects/<id>/`).
    pub uploads: Arc<project::KeptUploads>,
    /// More than one server pod: this pod's address and the name of all of them (docs/SLICES.md §8).
    pub peers: Option<peers::Peers>,
    asking: Arc<Semaphore>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_upload_bytes: 25 * 1024 * 1024,
            max_files: 5000,
            static_dir: None,
            ai: None,
            project: None,
            uploads: Arc::default(),
            peers: None,
            asking: Arc::new(Semaphore::new(ASKING_AT_ONCE)),
        }
    }
}

impl Config {
    /***
     * Read the server settings from DOCAIR_* environment variables, which the Deployment fills from the ConfigMap.
     *
     * The AI settings come from DOCAIR_AI_ENDPOINT and DOCAIR_AI_MODEL (ConfigMap)
     * and DOCAIR_AI_API_KEY (Secret docair-ai).
     *
     * @id      docair-server.config
     * @reads   k8s.cm.docair-server -> DOCAIR_MAX_UPLOAD_MB, DOCAIR_MAX_FILES, DOCAIR_AI_ENDPOINT, DOCAIR_AI_MODEL
     * @reads   k8s.secret.docair-ai -> DOCAIR_AI_API_KEY
     * @calls   docair-ai.config.resolve "merges the AI settings"
     * @calls   docair-server.peers.peers.from_env "reads this pod's address and the other pods' name"
     * @flow    provision 12 "the server reads its settings from the ConfigMap"
     ***/
    pub fn from_env() -> Config {
        let num = |key: &str| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
        };
        let default = Config::default();
        Config {
            max_upload_bytes: num("DOCAIR_MAX_UPLOAD_MB")
                .map_or(default.max_upload_bytes, |mb| mb * 1024 * 1024),
            max_files: num("DOCAIR_MAX_FILES").unwrap_or(default.max_files),
            static_dir: std::env::var("DOCAIR_STATIC_DIR").ok().map(PathBuf::from),
            ai: AiConfig::resolve(None, &Env::load(None)),
            peers: peers::Peers::from_env(),
            ..default
        }
    }
}

/***
 * The HTTP routes: `GET /healthz` for the kubelet's probes, `POST /api/scan` for uploads, `GET /api/ai` and `POST /api/ask` for questions; with a project (`docair serve`), its documentation at `/`, `GET /api/version` and `POST /api/apply`.
 *
 * A question, `/api/word` or `/api/glossary` without a graph is about the
 * project the server serves; its answer says whether the documentation was
 * written again.
 *
 * Requests reach this router through the Service docair-server on port 8080.
 *
 * @id      docair-server.router
 * @calls   docair-server.upload_router "adds the probe, uploads and whether the AI is on"
 * @calls   docair-server.project.page (GET /) -> html
 * @calls   docair-server.project.version (GET /api/version) -> ServeVersion
 * @calls   docair-server.project.apply (POST /api/apply) -> ServeApplied
 * @calls   docair-server.project.tidy_list (GET /api/tidy) -> ServeTidy
 * @calls   docair-server.project.tidy_apply (POST /api/tidy: tags) -> ServeApplied
 * @calls   docair-server.project.explain (POST /api/explain: a, b) -> ChangeFindings
 * @calls   docair-server.ask (POST /api/ask: question, graph, history) -> Reply
 * @calls   docair-server.patch (POST /api/patch: changes, files) -> PatchAnswer
 * @calls   docair-server.word (POST /api/word: graph, scope) -> Reply
 * @calls   docair-server.glossary (POST /api/glossary: graph) -> Reply
 * @flow    upload 10 "the server routes POST /api/scan to its handler"
 ***/
pub fn router(config: Config) -> Router {
    let limit = config.max_upload_bytes;
    let static_dir = config.static_dir.clone();
    let out = config.project.as_ref().map(|p| p.lock().out.clone());
    let mut routes = Router::new()
        .route("/api/ask", post(ask))
        .route("/api/patch", post(patch))
        .route("/api/word", post(word))
        .route("/api/glossary", post(glossary));
    if config.project.is_some() {
        routes = routes
            .route("/", get(project::page))
            .route("/api/version", get(project::version))
            .route("/api/apply", post(project::apply))
            .route(
                "/api/tidy",
                get(project::tidy_list).post(project::tidy_apply),
            )
            .route("/api/explain", post(project::explain));
    }
    let mut app = upload_router(config.clone()).merge(
        routes
            .layer(DefaultBodyLimit::max(limit))
            .with_state(config),
    );
    if let Some(out) = out {
        // A project's other outputs at their paths.
        return app.fallback_service(ServeDir::new(out));
    }
    if let Some(dir) = static_dir {
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }
    app
}

/***
 * The routes that need no project of their own: the probe, uploads, whether the AI is on, and the uploads kept as projects; `docair serve` adds its project's routes beside them.
 *
 * @calls docair-server.health (GET /healthz) -> ok
 * @calls docair-server.scan_upload (POST /api/scan: files) -> ScanResult
 * @calls docair-server.ai_status (GET /api/ai) -> AiStatus
 * @calls docair-server.kept_upload (GET /api/projects/<id>/…) -> the upload's page and outputs
 * @calls docair-server.kept_upload_post (POST /api/projects/<id>/api/…) -> the AI on the upload
 ***/
pub fn upload_router(config: Config) -> Router {
    let limit = config.max_upload_bytes;
    Router::new()
        .route("/healthz", get(health))
        .route("/api/scan", post(scan_upload))
        .route("/api/ai", get(ai_status))
        .route("/api/projects/{id}", kept())
        .route("/api/projects/{id}/", kept())
        .route("/api/projects/{id}/{*rest}", kept())
        .layer(DefaultBodyLimit::max(limit))
        .with_state(config)
}

/// A project's own routes, as `docair serve` has them at `/`: its page, the AI on it, applying, and its other outputs.
fn project_routes(config: Config) -> Router {
    let limit = config.max_upload_bytes;
    let out = config.project.as_ref().map(|p| p.lock().out.clone());
    let routes = Router::new()
        .route("/", get(project::page))
        .route("/api/ai", get(ai_status))
        .route("/api/version", get(project::version))
        .route("/api/apply", post(project::apply))
        .route(
            "/api/tidy",
            get(project::tidy_list).post(project::tidy_apply),
        )
        .route("/api/explain", post(project::explain))
        .route("/api/ask", post(ask))
        .route("/api/word", post(word))
        .route("/api/glossary", post(glossary))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(config);
    match out {
        Some(out) => routes.fallback_service(ServeDir::new(out)),
        None => routes,
    }
}

/***
 * An upload kept as a project, at `/api/projects/<id>/`: its page, and below it the routes a page `docair serve` serves calls (`api/ai`, `api/version`, `api/ask`, `api/word`, `api/glossary`, `api/apply`), on that upload.
 *
 * The web app shows the page in a sandboxed frame, whose origin is opaque, so the answers allow every origin (`Access-Control-Allow-Origin: *`); the server has no accounts or cookies to protect. An upload no longer kept answers 404: a short page that says so when the frame or a tab opens it, JSON for the page's own calls.
 *
 * @id      docair-server.kept_upload
 * @http    GET /api/projects/{id}/{rest}
 * @calls   docair-server.project.kept-uploads.get "finds the upload"
 * @calls   docair-server.peers.forward "sends a request for an upload another pod keeps to that pod"
 * @calls   docair-server.project.page (GET /api/projects/<id>/) -> html
 ***/
async fn kept_upload(State(config): State<Config>, req: Request) -> Response {
    kept_upload_any(config, req).await
}

/***
 * Ask, word, explain or apply on a kept upload (`POST /api/projects/<id>/api/…`), as `docair serve` does for its project.
 *
 * @id      docair-server.kept_upload_post
 * @http    POST /api/projects/{id}/{rest}
 * @calls   docair-server.ask (POST /api/projects/<id>/api/ask: question, history, context) -> ServeAnswer
 * @calls   docair-server.project.apply (POST /api/projects/<id>/api/apply) -> ServeApplied
 ***/
async fn kept_upload_post(State(config): State<Config>, req: Request) -> Response {
    kept_upload_any(config, req).await
}

/// The routes of a kept upload: GET, POST, and the preflight a JSON POST from the sandboxed frame sends first.
fn kept() -> axum::routing::MethodRouter<Config> {
    get(kept_upload)
        .post(kept_upload_post)
        .options(kept_upload_preflight)
}

async fn kept_upload_preflight(State(config): State<Config>, req: Request) -> Response {
    kept_upload_any(config, req).await
}

/// Any request to a kept upload, answered for every origin.
async fn kept_upload_any(config: Config, req: Request) -> Response {
    let mut res = if req.method() == Method::OPTIONS {
        let mut r = StatusCode::NO_CONTENT.into_response();
        let h = r.headers_mut();
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type"),
        );
        h.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("3600"),
        );
        r
    } else {
        kept_upload_answer(config, req).await
    };
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    res
}

/// What a reader sees who opens an upload the server no longer keeps (docair-702.33).
const UPLOAD_GONE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Upload no longer kept</title>
<style>
:root { color-scheme: light dark; --fg: #1d2330; --muted: #5b6475; --bg: #fbfbfd; }
@media (prefers-color-scheme: dark) { :root { --fg: #e6e9f0; --muted: #a3abbb; --bg: #161a22; } }
body { margin: 0; padding: 48px 16px; background: var(--bg); color: var(--fg); font: 16px/1.5 system-ui, sans-serif; }
main { max-width: 34rem; margin: 0 auto; }
h1 { font-size: 1.3rem; margin: 0 0 0.6rem; }
p { margin: 0 0 0.8rem; } .muted { color: var(--muted); font-size: 0.92rem; }
</style></head>
<body><main data-upload-gone>
<h1>This upload is no longer kept</h1>
<p>Upload the folder again to see its page and ask the AI about it.</p>
<p class="muted">The server keeps an upload for an hour after its last use, at most 8 at a time, and none across a restart. A page you downloaded still works, without the AI.</p>
</main></body></html>
"#;

/// The answer of the upload's own routes, the path below `/api/projects/<id>` as the request's path.
async fn kept_upload_answer(config: Config, mut req: Request) -> Response {
    use tower::ServiceExt;
    let path = req.uri().path().to_string();
    let below = path.strip_prefix("/api/projects/").unwrap_or("");
    let (id, rest) = below.split_once('/').unwrap_or((below, ""));
    let Some(project) = config.uploads.get(id) else {
        // A page opened in the frame or a new tab gets a page that says so; the page's own calls get JSON.
        let wants_page = req.method() == Method::GET
            && req
                .headers()
                .get(header::ACCEPT)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|a| a.contains("text/html"));
        // Kept by another server pod: the request goes there (docs/SLICES.md §8).
        if let Some(p) = &config.peers
            && !req.headers().contains_key(peers::FORWARDED)
            && let Some(to) = peers::pod_of(id)
            && let Some(res) = peers::forward(p, &to, req, config.max_upload_bytes).await
        {
            return res;
        }
        if wants_page {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                UPLOAD_GONE,
            )
                .into_response();
        }
        return ApiError(
            StatusCode::NOT_FOUND,
            "this upload is no longer kept; upload the folder again".into(),
        )
        .into_response();
    };
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let Ok(uri) = format!("/{rest}{query}").parse::<Uri>() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *req.uri_mut() = uri;
    let mut own = config.clone();
    own.project = Some(project);
    match project_routes(own).oneshot(req).await {
        Ok(r) => r.into_response(),
        Err(e) => match e {},
    }
}

/***
 * Liveness and readiness probe: answers `ok` as soon as the server accepts connections.
 *
 * @id docair-server.health
 * @http GET /healthz
 ***/
async fn health() -> &'static str {
    "ok"
}

/// What the frontend gets back for one upload.
#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub system: String,
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub flows: usize,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: Vec<Diagnostic>,
    /// The generated, self-contained docair page.
    pub html: String,
    /// The graph, so the browser can send it back with a question.
    pub graph: Graph,
    /// Where the server keeps the upload as a project: its page, which asks the AI and applies edits on the server's copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/***
 * Take one upload: save the files to a scratch folder, scan them as a project, keep it, answer with the summary, the page and where the page lives.
 *
 * The upload is kept as a project (`KeptUploads`): its page at `/api/projects/<id>/` asks the AI about it, and applied edits change the server's copy. It is dropped with its folder after an hour unused, or when more uploads come.
 *
 * @id      docair-server.scan_upload
 * @http    POST /api/scan
 * @calls   docair-server.save_files (multipart files) -> file count
 * @calls   docair-server.project.served-project.open_upload (folder) -> the upload as a project
 * @calls   docair-server.project.kept-uploads.keep "keeps it under an id"
 * @flow    upload 11 "the handler takes the upload"
 ***/
async fn scan_upload(
    State(config): State<Config>,
    mut multipart: Multipart,
) -> Result<Json<ScanResult>, ApiError> {
    let dir = tempfile::Builder::new()
        .prefix("docair-upload-")
        .tempdir()?;
    let files = save_files(&mut multipart, dir.path(), &config).await?;
    if files == 0 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "the upload contained no files".into(),
        ));
    }
    let root = scan_root(dir.path())?;
    let ai = config.ai.clone();
    let (project, mut result) = tokio::task::spawn_blocking(move || {
        let project = project::ServedProject::open_upload(&root, ai)?;
        let result = {
            let s = project.lock();
            let html = std::fs::read_to_string(s.out.join("docair.html"))?;
            scan_result(s.graph.clone(), html, files)
        };
        anyhow::Ok((project, result))
    })
    .await??;
    let pod = config.peers.as_ref().map(peers::Peers::prefix);
    let id = config
        .uploads
        .keep_as(dir, Arc::new(project), pod.as_deref());
    result.page = Some(format!("/api/projects/{id}/"));
    Ok(Json(result))
}

/// The folder to scan: a single uploaded folder is scanned from inside, so its name becomes the system's name.
fn scan_root(root: &Path) -> std::io::Result<PathBuf> {
    let entries: Vec<_> = std::fs::read_dir(root)?.filter_map(Result::ok).collect();
    Ok(match entries.as_slice() {
        [only] if only.path().is_dir() => only.path(),
        _ => root.to_path_buf(),
    })
}

/// The summary of a scan, with its page.
fn scan_result(graph: Graph, html: String, files: usize) -> ScanResult {
    let errors = graph
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    ScanResult {
        system: graph.system.clone(),
        files,
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
        flows: graph.flows.len(),
        errors,
        warnings: graph.diagnostics.len() - errors,
        diagnostics: graph.diagnostics.iter().take(100).cloned().collect(),
        html,
        graph,
        page: None,
    }
}

/***
 * Write every uploaded file into the scratch folder, keeping its path inside the uploaded folder.
 *
 * Paths are checked first: absolute paths and `..` are rejected, so an upload can never write outside the scratch folder.
 *
 * @id      docair-server.save_files
 * @writes  upload-dir (uploaded files)
 * @flow    upload 12 "the files land in a scratch folder"
 ***/
pub async fn save_files(
    multipart: &mut Multipart,
    root: &Path,
    config: &Config,
) -> Result<usize, ApiError> {
    let mut count = 0;
    while let Some(field) = multipart.next_field().await? {
        let Some(name) = field.file_name().map(str::to_string) else {
            continue;
        };
        let Some(rel) = safe_path(&name) else {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!("unsafe file path `{name}`"),
            ));
        };
        count += 1;
        if count > config.max_files {
            return Err(ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("more than {} files", config.max_files),
            ));
        }
        let data = field.bytes().await?;
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, &data).await?;
    }
    Ok(count)
}

/// A relative path that stays inside the upload folder, or `None`.
pub fn safe_path(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let mut out = PathBuf::new();
    for part in Path::new(&normalized).components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/***
 * Run the docair pipeline on the saved files and render the page.
 *
 * An upload of one folder is scanned from inside that folder, so the folder's name becomes the system's name.
 *
 * An upload is untrusted: docair runs no indexer on it, since an indexer builds the code and build scripts run.
 *
 * @id      docair-server.run_scan
 * @calls   docair-core.scan (folder, Rust adapter, lenient, untrusted) -> Graph
 * @calls   docair-viewer.render_html (graph) -> html
 * @reads   upload-dir -> source files
 * @flow    upload 13 "the docair pipeline runs on the upload"
 ***/
pub fn run_scan(root: &Path, files: usize) -> anyhow::Result<ScanResult> {
    let graph = scan(
        &scan_root(root)?,
        docair_adapters::all(),
        &Options {
            lenient: true,
            untrusted: true,
        },
    )?;
    let html = docair_viewer::render_html(&graph);
    Ok(scan_result(graph, html, files))
}

/// Whether the web app may show the Ask box. Never the key or the endpoint.
#[derive(Debug, Serialize)]
pub struct AiStatus {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/***
 * Tell the web app whether asking is on, and which model answers.
 *
 * @id   docair-server.ai_status
 * @http GET /api/ai
 ***/
async fn ai_status(State(config): State<Config>) -> Json<AiStatus> {
    Json(AiStatus {
        enabled: config.ai.is_some(),
        model: config.ai.map(|a| a.model),
    })
}

#[derive(Debug, Deserialize)]
pub struct AskRequest {
    pub question: String,
    /// The graph to ask about; none asks about the project the server serves.
    #[serde(default)]
    pub graph: Option<Graph>,
    /// Earlier turns of the conversation, oldest first.
    #[serde(default)]
    pub history: Vec<Turn>,
    /// What the question points at: one part's id (what is selected), or a list of typed refs,
    /// `part:<id>`, `data:<id>`, `term:<word>`, `line:<k>@<address>`, `edges:<i>,<j>` (docair-5n2).
    #[serde(default)]
    pub context: Option<AskContext>,
}

/// A question's context: one id, as the page sent before, or a list of typed refs.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AskContext {
    One(String),
    Many(Vec<String>),
}

impl AskContext {
    fn refs(self) -> Vec<String> {
        match self {
            AskContext::One(s) => vec![s],
            AskContext::Many(v) => v,
        }
    }
}

/// Ask with what the question points at in words; refs that name nothing join the reply's problems.
fn ask_about(
    ai: &AiConfig,
    graph: &Graph,
    question: &str,
    history: &[Turn],
    refs: &[String],
) -> anyhow::Result<docair_ai::Reply> {
    let (about, problems) = docair_ai::ask_context::about(graph, refs);
    let mut reply = docair_ai::ask(ai, graph, question, history, about.as_deref())?;
    reply.problems.extend(problems);
    Ok(reply)
}

/***
 * Ask the model about the graph the browser sends along, with the conversation so far; answer with the checked reply.
 *
 * The key stays in the cluster. At most two questions run at once; more get 429.
 *
 * @id      docair-server.ask
 * @http    POST /api/ask
 * @calls   docair-ai.ask.ask (question, graph, history) -> Reply
 * @calls   docair-ai.ask_context.about "turns what the question points at into words"
 ***/
async fn ask(
    State(config): State<Config>,
    Json(req): Json<AskRequest>,
) -> Result<Response, ApiError> {
    let Some(ai) = config.ai.clone() else {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "AI is off: the server has no DOCAIR_AI_ENDPOINT and DOCAIR_AI_MODEL".into(),
        ));
    };
    let question = req.question.trim().to_string();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("ask a question of 1 to {MAX_QUESTION_CHARS} characters"),
        ));
    }
    let Ok(_slot) = config.asking.clone().try_acquire_owned() else {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "the server is already asking two questions; try again in a moment".into(),
        ));
    };
    let AskRequest {
        graph,
        history,
        context,
        ..
    } = req;
    let refs = context.map(AskContext::refs).unwrap_or_default();
    let Some(graph) = graph else {
        // The project's own graph; the answer is taken into the project.
        let project = served(&config)?;
        let graph = project.lock().graph.clone();
        let fresh = history.is_empty();
        let answer = tokio::task::spawn_blocking(move || {
            let reply = ask_about(&ai, &graph, &question, &history, &refs)
                .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
            project::answered(&project, reply, fresh)
        })
        .await??;
        return Ok(Json(answer).into_response());
    };
    let reply =
        tokio::task::spawn_blocking(move || ask_about(&ai, &graph, &question, &history, &refs))
            .await?
            .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(reply).into_response())
}

/// The project this server serves, for a request that brings no graph.
fn served(config: &Config) -> Result<Arc<project::ServedProject>, ApiError> {
    config.project.clone().ok_or_else(|| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "send the graph: this server serves no project of its own".into(),
        )
    })
}

#[derive(Debug, Deserialize)]
pub struct WordRequest {
    #[serde(default)]
    pub graph: Option<Graph>,
    /// Only lines that touch this node or anything inside it.
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Also lines that show only a type.
    #[serde(default)]
    pub all: bool,
}

/***
 * Let the model write words for the lines that say nothing; answer with the words (shown at once) and the edits that put them into the code.
 *
 * @id      docair-server.word
 * @http    POST /api/word
 * @calls   docair-ai.word.word (graph, scope) -> words + edits
 ***/
async fn word(
    State(config): State<Config>,
    Json(req): Json<WordRequest>,
) -> Result<Response, ApiError> {
    let Some(ai) = config.ai.clone() else {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "AI is off: the server has no DOCAIR_AI_ENDPOINT and DOCAIR_AI_MODEL".into(),
        ));
    };
    let Ok(_slot) = config.asking.clone().try_acquire_owned() else {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "the server is already asking two questions; try again in a moment".into(),
        ));
    };
    let limit = req.limit.unwrap_or(docair_ai::word::BATCH).clamp(1, 80);
    let WordRequest {
        graph, scope, all, ..
    } = req;
    let Some(graph) = graph else {
        let project = served(&config)?;
        let graph = project.lock().graph.clone();
        let answer = tokio::task::spawn_blocking(move || {
            let reply = docair_ai::word(&ai, &graph, scope.as_deref(), limit, all)
                .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
            project::answered(&project, reply, true)
        })
        .await??;
        return Ok(Json(answer).into_response());
    };
    let reply = tokio::task::spawn_blocking(move || {
        docair_ai::word(&ai, &graph, scope.as_deref(), limit, all)
    })
    .await?
    .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(reply).into_response())
}

#[derive(Debug, Deserialize)]
pub struct GlossaryRequest {
    #[serde(default)]
    pub graph: Option<Graph>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/***
 * Let the model explain the words of the docs that the glossary does not explain yet; answer with the entries (shown at once) and the edit that writes them.
 *
 * @id      docair-server.glossary
 * @http    POST /api/glossary
 * @calls   docair-ai.glossary.explain (graph) -> entries + edit "asks for the missing terms"
 ***/
async fn glossary(
    State(config): State<Config>,
    Json(req): Json<GlossaryRequest>,
) -> Result<Response, ApiError> {
    let Some(ai) = config.ai.clone() else {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "AI is off: the server has no DOCAIR_AI_ENDPOINT and DOCAIR_AI_MODEL".into(),
        ));
    };
    let Ok(_slot) = config.asking.clone().try_acquire_owned() else {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "the server is already asking two questions; try again in a moment".into(),
        ));
    };
    let limit = req.limit.unwrap_or(15).clamp(1, 40);
    let Some(graph) = req.graph else {
        let project = served(&config)?;
        let graph = project.lock().graph.clone();
        let answer = tokio::task::spawn_blocking(move || {
            let reply = docair_ai::glossary::explain(&ai, &graph, limit)
                .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
            project::answered(&project, reply, true)
        })
        .await??;
        return Ok(Json(answer).into_response());
    };
    let reply =
        tokio::task::spawn_blocking(move || docair_ai::glossary::explain(&ai, &graph, limit))
            .await?
            .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(reply).into_response())
}

/// A file the browser holds, by its path relative to the scanned root.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileText {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct PatchRequest {
    pub changes: Vec<Change>,
    pub files: Vec<FileText>,
    /// Ids of the graph, to check the refs the edits add.
    #[serde(default)]
    pub ids: Vec<String>,
}

/// The edited files and the patch for the real repository.
#[derive(Debug, Serialize)]
pub struct PatchAnswer {
    pub files: Vec<FileText>,
    pub diff: String,
    pub applied: Vec<usize>,
    pub problems: Vec<String>,
}

/***
 * Apply proposed edits to the files the browser sends; answer with the new texts and a unified diff.
 *
 * The server keeps nothing: the browser holds the files, and decides whether to use the new texts.
 *
 * @id      docair-server.patch
 * @http    POST /api/patch
 * @calls   docair-core.edit.patch (changes, files) -> new files + diff
 ***/
async fn patch(Json(req): Json<PatchRequest>) -> Result<Json<PatchAnswer>, ApiError> {
    if req.changes.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "no edits".into()));
    }
    let files: std::collections::HashMap<String, String> =
        req.files.into_iter().map(|f| (f.path, f.text)).collect();
    let known: std::collections::HashSet<String> = req.ids.into_iter().collect();
    let result = docair_core::edit::patch(&req.changes, &|p: &str| files.get(p).cloned(), &known);
    Ok(Json(PatchAnswer {
        files: result
            .files
            .into_iter()
            .map(|f| FileText {
                path: f.path,
                text: f.after,
            })
            .collect(),
        diff: result.diff,
        applied: result.applied,
        problems: result.problems,
    }))
}

/// An error answered as `{ "error": "…" }` with a status code.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

impl From<axum::extract::multipart::MultipartError> for ApiError {
    fn from(e: axum::extract::multipart::MultipartError) -> Self {
        ApiError(e.status(), e.body_text())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    fn multipart(files: &[(&str, &str)]) -> (String, Vec<u8>) {
        let boundary = "docair-test-boundary";
        let mut body = Vec::new();
        for (name, content) in files {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: text/plain\r\n\r\n{content}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    async fn post(files: &[(&str, &str)]) -> (StatusCode, serde_json::Value) {
        let (content_type, body) = multipart(files);
        let request = Request::post("/api/scan")
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();
        let response = router(Config::default()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// An upload is kept as a project: its page and its own routes under `/api/projects/<id>/`, open to any origin.
    #[tokio::test]
    async fn an_upload_is_kept_and_served_as_a_project() {
        let app = router(Config::default());
        let call = |req: Request<Body>| {
            let app = app.clone();
            async move {
                let res = app.oneshot(req).await.unwrap();
                let status = res.status();
                let cors = res
                    .headers()
                    .get("access-control-allow-origin")
                    .map(|v| v.to_str().unwrap().to_string());
                let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
                (status, cors, String::from_utf8_lossy(&bytes).to_string())
            }
        };
        let (content_type, body) = multipart(&[(
            "shop/README.md",
            "# Shop\n\n```docair\nThe shop.\n\n@id      app\n@kind    module\n@part-of shop\n```\n",
        )]);
        let (status, _, json) = call(
            Request::post("/api/scan")
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        let page = json["page"].as_str().unwrap().to_string();
        let (status, cors, html) = call(Request::get(&page).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cors.as_deref(), Some("*"));
        assert!(html.contains("docair-packed"), "the upload's page");
        let (status, _, version) = call(
            Request::get(format!("{page}api/version"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(version, r#"{"version":1}"#);
        let (status, _, graph) = call(
            Request::get(format!("{page}docair.graph.json"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(graph.contains("\"app\""), "its other outputs");
        // The frame's origin is opaque: a JSON POST asks first.
        let (status, cors, _) = call(
            Request::options(format!("{page}api/ask"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(cors.as_deref(), Some("*"));
        // Asking about the upload without a model says so.
        let (status, _, said) = call(
            Request::post(format!("{page}api/ask"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"question":"How does it work?"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{said}");
        let (status, _, gone) = call(
            Request::get("/api/projects/0123456789abcdef0123456789abcdef/")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(gone.contains("\"error\""), "a call gets JSON: {gone}");
        // Opened in the frame or a new tab, a dropped upload is a page that says so.
        let (status, cors, gone) = call(
            Request::get("/api/projects/0123456789abcdef0123456789abcdef/")
                .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(cors.as_deref(), Some("*"));
        assert!(gone.contains("This upload is no longer kept"), "{gone}");
    }

    /// Two server pods: an upload kept by one is reached through the other; an id naming no server pod is not forwarded.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pod_forwards_an_upload_it_does_not_keep() {
        let listen = || async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap() };
        let (la, lb) = (listen().await, listen().await);
        let (a, b) = (
            la.local_addr().unwrap().to_string(),
            lb.local_addr().unwrap().to_string(),
        );
        let pod = |me: &str| Config {
            peers: Some(peers::Peers {
                me: me.to_string(),
                lookup: b.clone(),
            }),
            ..Config::default()
        };
        let (app_a, app_b) = (router(pod(&a)), router(pod(&b)));
        tokio::spawn(axum::serve(la, app_a.clone()).into_future());
        tokio::spawn(axum::serve(lb, app_b.clone()).into_future());
        let call = |app: Router, req: Request<Body>| async move {
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
            (status, String::from_utf8_lossy(&bytes).to_string())
        };
        let (content_type, body) = multipart(&[(
            "shop/README.md",
            "# Shop\n\n```docair\nThe shop.\n\n@id      app\n@kind    module\n@part-of shop\n```\n",
        )]);
        let (status, json) = call(
            app_b.clone(),
            Request::post("/api/scan")
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        let page = json["page"].as_str().unwrap().to_string();
        let id = page
            .trim_start_matches("/api/projects/")
            .trim_end_matches('/');
        assert_eq!(
            peers::pod_of(id).as_deref(),
            Some(b.as_str()),
            "the id names pod B"
        );
        // Pod A keeps nothing: it forwards the page and a question to B.
        let (status, html) = call(
            app_a.clone(),
            Request::get(&page)
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{html}");
        assert!(html.contains("docair-packed"), "B's page through A");
        let (status, said) = call(
            app_a.clone(),
            Request::post(format!("{page}api/ask"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"question":"How does it work?"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "B's answer, AI off: {said}"
        );
        // An id naming an address that is no server pod is not forwarded.
        let forged = format!(
            "/api/projects/{}-0123456789abcdef0123456789abcdef/",
            peers::Peers {
                me: "127.0.0.1:1".into(),
                lookup: String::new()
            }
            .prefix()
        );
        let (status, gone) = call(
            app_a.clone(),
            Request::get(&forged).body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(gone.contains("no longer kept"), "{gone}");
        // A forwarded request is never forwarded again.
        let (status, _) = call(
            app_a,
            Request::get(&page)
                .header(peers::FORWARDED, "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn paths_stay_inside_the_upload() {
        assert_eq!(
            safe_path("shop/src/main.rs"),
            Some(PathBuf::from("shop/src/main.rs"))
        );
        assert_eq!(safe_path("./a\\b.rs"), Some(PathBuf::from("a/b.rs")));
        assert_eq!(safe_path("../etc/passwd"), None);
        assert_eq!(safe_path("/etc/passwd"), None);
        assert_eq!(safe_path("a/../../b"), None);
        assert_eq!(safe_path(""), None);
    }

    #[tokio::test]
    async fn health_answers_ok() {
        let response = router(Config::default())
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_upload_becomes_a_page() {
        let main_rs = "/*** The API. @http POST /api/orders ***/\npub fn create() {}\n";
        let (status, json) = post(&[
            ("shop/src/main.rs", main_rs),
            ("shop/README.md", "# Shop\n"),
        ])
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(
            json["system"], "shop",
            "a single uploaded folder names the system"
        );
        assert_eq!(json["files"], 2);
        assert!(json["nodes"].as_u64().unwrap() >= 2);
        assert!(json["html"].as_str().unwrap().contains("docair-packed"));
        assert!(
            json["page"]
                .as_str()
                .is_some_and(|p| p.starts_with("/api/projects/")),
            "{json}"
        );
    }

    #[tokio::test]
    async fn unsafe_paths_are_rejected() {
        let (status, json) = post(&[("../evil.rs", "fn x() {}")]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["error"].as_str().unwrap().contains("unsafe"));
    }

    async fn send(config: Config, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = router(config).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn the_upload_answer_carries_the_graph() {
        let (status, json) = post(&[("a/src/main.rs", "/*** A. ***/\npub fn a() {}\n")]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["graph"]["schema"], "docair.graph/0");
    }

    #[tokio::test]
    async fn asking_without_ai_is_off() {
        let (status, json) = send(
            Config::default(),
            Request::get("/api/ai").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["enabled"], false);

        let graph = run_scan_on(&[("a/src/main.rs", "/*** A. ***/\npub fn a() {}\n")]);
        let body = serde_json::json!({ "question": "how?", "graph": graph }).to_string();
        let request = Request::post("/api/ask")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let (status, json) = send(Config::default(), request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{json}");
    }

    #[tokio::test]
    async fn the_ai_status_names_the_model_but_not_the_key() {
        let mut config = Config::default();
        config.ai = AiConfig::resolve(
            Some(&docair_core::config::AiSettings {
                endpoint: Some("http://127.0.0.1:9/v1".into()),
                model: Some("m".into()),
                ..Default::default()
            }),
            &Env::default(),
        );
        if let Some(ai) = config.ai.as_mut() {
            ai.api_key = Some("secret-key".into());
        }
        let (status, json) = send(
            config.clone(),
            Request::get("/api/ai").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["enabled"], true);
        assert_eq!(json["model"], "m");
        assert!(!json.to_string().contains("secret-key"));

        // An unreachable endpoint is a 502, and the message never shows the key.
        let graph = run_scan_on(&[("a/src/main.rs", "/*** A. ***/\npub fn a() {}\n")]);
        let body = serde_json::json!({ "question": "how?", "graph": graph }).to_string();
        let request = Request::post("/api/ask")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let (status, json) = send(config, request).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{json}");
        assert!(!json.to_string().contains("secret-key"));
    }

    #[tokio::test]
    async fn patches_are_made_from_the_files_the_browser_sends() {
        let body = serde_json::json!({
            "changes": [{ "node": "a", "mode": "edit", "file": "src/a.rs", "line": 1,
                          "add": ["@calls b"], "remove": [], "why": "w" }],
            "files": [{ "path": "src/a.rs", "text": "/***\n * A.\n *\n * @id a\n ***/\nfn a() {}\n" }],
            "ids": ["a", "b"]
        })
        .to_string();
        let request = Request::post("/api/patch")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let (status, json) = send(Config::default(), request).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["applied"], serde_json::json!([0]));
        assert!(
            json["files"][0]["text"]
                .as_str()
                .unwrap()
                .contains(" * @calls b\n")
        );
        assert!(json["diff"].as_str().unwrap().contains("+ * @calls b"));
        assert_eq!(json["problems"], serde_json::json!([]));
    }

    fn run_scan_on(files: &[(&str, &str)]) -> Graph {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        run_scan(dir.path(), files.len()).unwrap().graph
    }

    /// An uploaded `docair.toml` can't make the server build the code (an indexer runs
    /// build scripts) or read a code index outside the upload.
    #[test]
    fn an_upload_runs_no_code_and_reads_nothing_outside() {
        let dir = tempfile::tempdir().unwrap();
        let upload = dir.path().join("upload");
        let up = upload.join("up");
        let files = [
            (
                "Cargo.toml",
                "[package]\nname = \"up\"\nversion = \"0.1.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n",
            ),
            (
                "build.rs",
                "fn main() { std::fs::write(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/RAN\"), \"\").unwrap(); }\n",
            ),
            ("src/main.rs", "/***\n * @id up.main\n ***/\nfn main() {}\n"),
            (
                "docair.toml",
                "system = \"up\"\n[code]\nrun_indexers = true\nscip = [\"../../outside.scip\", \"/dev/zero\"]\n",
            ),
        ];
        for (name, content) in files {
            let path = up.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        std::fs::write(dir.path().join("outside.scip"), "").unwrap();
        let graph = run_scan(&upload, files.len()).unwrap().graph;
        assert!(!up.join("RAN").exists(), "the upload's build script ran");
        let refused = graph
            .diagnostics
            .iter()
            .filter(|d| d.code == "W113" && d.message.contains("no file inside the project"))
            .count();
        assert_eq!(refused, 2, "{:?}", graph.diagnostics);
    }

    /// The other paths an uploaded `docair.toml` names stay inside the upload too.
    #[test]
    fn an_upload_names_no_files_outside() {
        let dir = tempfile::tempdir().unwrap();
        let upload = dir.path().join("upload");
        let up = upload.join("up");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("report.json"), r#"{"suites": []}"#).unwrap();
        std::fs::write(outside.join("meanings.toml"), "").unwrap();
        let report = outside.join("report.json");
        let toml = format!(
            "system = \"up\"\nwalkthroughs = \"../../outside\"\nmeanings = \"../../outside/meanings.toml\"\n[tests]\nresults = [\"{}\", \"../../outside/report.json\"]\n",
            report.display()
        );
        std::fs::create_dir_all(up.join("src")).unwrap();
        std::fs::write(up.join("docair.toml"), toml).unwrap();
        std::fs::write(
            up.join("src/main.rs"),
            "/***\n * @id up.main\n ***/\nfn main() {}\n",
        )
        .unwrap();
        let graph = run_scan(&upload, 2).unwrap().graph;
        let refused = |code: &str| {
            graph
                .diagnostics
                .iter()
                .filter(|d| d.code == code && d.message.contains("inside the project"))
                .count()
        };
        assert_eq!(
            (refused("W111"), refused("W113"), refused("W110")),
            (2, 1, 1),
            "{:?}",
            graph.diagnostics
        );
    }

    #[tokio::test]
    async fn empty_uploads_are_rejected() {
        let (status, _) = post(&[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
