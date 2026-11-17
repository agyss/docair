//! docair-server: generate a docair page from uploaded source files.
//!
//! `POST /api/scan` takes a multipart upload (one part per file, the part's
//! file name is the path inside the uploaded folder), runs the docair
//! pipeline on it and answers with the summary and the generated page.
//! `POST /api/ask` asks the configured AI model about a graph (SPEC §10.5).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use docair_ai::{AiConfig, Env, Reply, Turn};
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
    asking: Arc<Semaphore>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_upload_bytes: 25 * 1024 * 1024,
            max_files: 5000,
            static_dir: None,
            ai: None,
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
            ..default
        }
    }
}

/***
 * The HTTP routes: `GET /healthz` for the kubelet's probes, `POST /api/scan` for uploads, `GET /api/ai` and `POST /api/ask` for questions.
 *
 * Requests reach this router through the Service docair-server on port 8080.
 *
 * @id      docair-server.router
 * @calls   docair-server.health (GET /healthz) -> ok
 * @calls   docair-server.scan_upload (POST /api/scan: files) -> ScanResult
 * @calls   docair-server.ai_status (GET /api/ai) -> AiStatus
 * @calls   docair-server.ask (POST /api/ask: question, graph, history) -> Reply
 * @calls   docair-server.patch (POST /api/patch: changes, files) -> PatchAnswer
 * @calls   docair-server.word (POST /api/word: graph, scope) -> Reply
 * @calls   docair-server.glossary (POST /api/glossary: graph) -> Reply
 * @flow    upload 10 "the server routes POST /api/scan to its handler"
 ***/
pub fn router(config: Config) -> Router {
    let limit = config.max_upload_bytes;
    let static_dir = config.static_dir.clone();
    let mut app = Router::new()
        .route("/healthz", get(health))
        .route("/api/scan", post(scan_upload))
        .route("/api/ai", get(ai_status))
        .route("/api/ask", post(ask))
        .route("/api/patch", post(patch))
        .route("/api/word", post(word))
        .route("/api/glossary", post(glossary))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(config);
    if let Some(dir) = static_dir {
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }
    app
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
}

/***
 * Take one upload: save the files to a scratch folder, scan them, answer with the summary and the page.
 *
 * The scratch folder is deleted when the request ends, so the server keeps no state between requests.
 *
 * @id      docair-server.scan_upload
 * @http    POST /api/scan
 * @calls   docair-server.save_files (multipart files) -> file count
 * @calls   docair-server.run_scan (folder) -> ScanResult
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
    let root = dir.path().to_path_buf();
    let result = tokio::task::spawn_blocking(move || run_scan(&root, files)).await??;
    Ok(Json(result))
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
 * @id      docair-server.run_scan
 * @calls   docair-core.scan (folder, Rust adapter, lenient) -> Graph
 * @calls   docair-viewer.render_html (graph) -> html
 * @reads   upload-dir -> source files
 * @flow    upload 13 "the docair pipeline runs on the upload"
 ***/
pub fn run_scan(root: &Path, files: usize) -> anyhow::Result<ScanResult> {
    let entries: Vec<_> = std::fs::read_dir(root)?.filter_map(Result::ok).collect();
    let scan_root = match entries.as_slice() {
        [only] if only.path().is_dir() => only.path(),
        _ => root.to_path_buf(),
    };
    let graph = scan(
        &scan_root,
        docair_adapters::all(),
        &Options { lenient: true },
    )?;
    let errors = graph
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    Ok(ScanResult {
        system: graph.system.clone(),
        files,
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
        flows: graph.flows.len(),
        errors,
        warnings: graph.diagnostics.len() - errors,
        diagnostics: graph.diagnostics.iter().take(100).cloned().collect(),
        html: docair_viewer::render_html(&graph),
        graph,
    })
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
    pub graph: Graph,
    /// Earlier turns of the conversation, oldest first.
    #[serde(default)]
    pub history: Vec<Turn>,
    /// What is selected on the chart, e.g. `docair-web.app (component)`.
    #[serde(default)]
    pub context: Option<String>,
}

/***
 * Ask the model about the graph the browser sends along, with the conversation so far; answer with the checked reply.
 *
 * The key stays in the cluster. At most two questions run at once; more get 429.
 *
 * @id      docair-server.ask
 * @http    POST /api/ask
 * @calls   docair-ai.ask.ask (question, graph, history) -> Reply
 ***/
async fn ask(
    State(config): State<Config>,
    Json(req): Json<AskRequest>,
) -> Result<Json<Reply>, ApiError> {
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
    let reply = tokio::task::spawn_blocking(move || {
        docair_ai::ask(&ai, &graph, &question, &history, context.as_deref())
    })
    .await?
    .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(reply))
}

#[derive(Debug, Deserialize)]
pub struct WordRequest {
    pub graph: Graph,
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
) -> Result<Json<Reply>, ApiError> {
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
    let reply = tokio::task::spawn_blocking(move || {
        docair_ai::word(&ai, &graph, scope.as_deref(), limit, all)
    })
    .await?
    .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(reply))
}

#[derive(Debug, Deserialize)]
pub struct GlossaryRequest {
    pub graph: Graph,
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
) -> Result<Json<Reply>, ApiError> {
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
    let graph = req.graph;
    let reply =
        tokio::task::spawn_blocking(move || docair_ai::glossary::explain(&ai, &graph, limit))
            .await?
            .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    Ok(Json(reply))
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
        assert!(json["html"].as_str().unwrap().contains("docair-graph"));
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

    #[tokio::test]
    async fn empty_uploads_are_rejected() {
        let (status, _) = post(&[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
