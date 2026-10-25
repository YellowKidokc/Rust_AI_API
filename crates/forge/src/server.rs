//! Local HTTP gateway (127.0.0.1 only). One port serves three audiences:
//!
//! * any OpenAI-style client: `POST /v1/chat/completions`, `GET /v1/models` (model = a route name,
//!   or a model that appears in a route, or `auto`)
//! * the existing HTML pages: `POST /api/generate` and `/api/atom-builder/raw` (`{prompt, system}`),
//!   `POST /api/audit`, and the Canon Workbench contract `GET /health`, `GET /search`, `GET /agenda`,
//!   `POST /candidate`
//! * humans and other agents: `GET /route/plan`, `GET /ledger/recent`, `GET /ledger/stats`,
//!   `GET /context/search`
//! * the home page at `/` (every API source and station: `GET /api/stations`, `GET|POST /api/apis`,
//!   `POST /api/apis/remove`, `POST /api/launch`, `/pages/{i}`; see `apis.rs`)
//! * the API workspace page at `/workspace`, backed by `GET /api/grid` (rows by permanent uid, columns by job),
//!   `GET|POST /api/runs`, `POST /api/runs/{id}/{approve|pause|resume|cancel}`, `GET /api/uid/...`,
//!   `GET /api/focus`
//!
//! Keys stay in this process. Browsers are only allowed from `file://` pages (Origin `null`) and
//! localhost, plus `server.cors_origins`, so a website you visit can't spend your credits.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router as Http};
use forge_core::canon::{self, CandidateInput, CanonError};
use forge_core::router::{RouteFailure, Routed};
use forge_core::runs;
use forge_core::{Forge, Message, RouteRequest};

use crate::apis;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::{AllowOrigin, CorsLayer};

type S = State<Arc<Forge>>;

pub fn app(forge: Arc<Forge>) -> Http {
    let extra: Vec<String> = forge.config.settings.server.cors_origins.clone();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin: &HeaderValue, _| {
            origin_ok(origin, &extra)
        }))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    Http::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/api/generate", post(generate))
        .route("/api/atom-builder/raw", post(generate))
        .route("/api/audit", post(audit))
        .route("/candidate", post(candidate))
        .route("/search", get(search))
        .route("/agenda", get(agenda))
        .route("/route/plan", get(plan))
        .route("/ledger/recent", get(ledger_recent))
        .route("/ledger/stats", get(ledger_stats))
        .route("/context/search", get(context_search))
        .route("/", get(home_page))
        .route("/home", get(home_page))
        .route("/workspace", get(workspace_page))
        .route("/api/grid", get(grid))
        .route("/api/runs", get(runs_list).post(runs_start))
        .route("/api/runs/{id}", get(runs_get))
        .route("/api/runs/{id}/{action}", post(runs_action))
        .route("/api/uid/orphans", get(uid_orphans))
        .route("/api/uid/{uid}", get(uid_show))
        .route("/api/focus", get(focus))
        .route("/api/templates", get(templates))
        .route("/workbench", get(pack_workbench))
        .route("/api/stations", get(stations))
        .route("/api/apis", get(apis_list).post(apis_add))
        .route("/api/apis/remove", post(apis_remove))
        .route("/api/launch", post(launch))
        .route("/pages/{i}", get(page))
        .layer(cors)
        .with_state(forge)
}

fn origin_ok(origin: &HeaderValue, extra: &[String]) -> bool {
    let Ok(o) = origin.to_str() else { return false };
    if o == "null" {
        return true;
    }
    for local in ["http://127.0.0.1", "http://localhost", "http://[::1]"] {
        if o == local
            || o.strip_prefix(local)
                .is_some_and(|rest| rest.starts_with(':'))
        {
            return true;
        }
    }
    extra.iter().any(|e| e == o)
}

pub async fn serve(forge: Arc<Forge>, host: &str, port: u16) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind {addr}: {e}. Is another engine already on that port? Use --port to pick another."))?;
    eprintln!("forge gateway on http://{addr}  (db: {})", forge.store.path);
    eprintln!(
        "  OpenAI-compatible: http://{addr}/v1/chat/completions   routes: {}",
        forge.router.route_names().join(", ")
    );
    axum::serve(listener, app(forge)).await?;
    Ok(())
}

struct ApiError(StatusCode, Value);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<RouteFailure> for ApiError {
    fn from(f: RouteFailure) -> Self {
        ApiError(
            StatusCode::BAD_GATEWAY,
            json!({"error": {"message": f.message, "type": "route_failed"}, "plan": f.plan, "attempts": f.attempts}),
        )
    }
}

impl From<CanonError> for ApiError {
    fn from(e: CanonError) -> Self {
        match e {
            CanonError::Invalid(m) => {
                ApiError(StatusCode::UNPROCESSABLE_ENTITY, json!({"error": m}))
            }
            CanonError::Db(e) => ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": e.to_string()}),
            ),
        }
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({"error": e.to_string()}),
    )
}

async fn health(State(f): S) -> Json<Value> {
    Json(json!({
        "ok": true,
        "service": "forge",
        "version": env!("CARGO_PKG_VERSION"),
        "canon_root": f.store.path,
        "routes": f.router.route_names(),
        "default_route": f.config.settings.default_route_name(),
        "providers_ready": f.router.available_providers(),
        "providers_unavailable": f.router.unavailable_providers(),
    }))
}

async fn models(State(f): S) -> Json<Value> {
    let data: Vec<Value> = f
        .router
        .route_names()
        .into_iter()
        .map(|r| json!({"id": r, "object": "model", "owned_by": "forge"}))
        .collect();
    Json(json!({"object": "list", "data": data}))
}

#[derive(Deserialize)]
struct ChatBody {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<Message>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    response_format: Option<Value>,
    #[serde(default)]
    stream: Option<bool>,
}

/// (route, pinned provider+model)
type Resolved = (Option<String>, Option<(String, Option<String>)>);

/// `model` is a route name, `auto`/empty (default route), or a model listed in some route (pinned).
fn resolve_model(f: &Forge, model: Option<&str>) -> Result<Resolved, ApiError> {
    match model
        .map(str::trim)
        .filter(|m| !m.is_empty() && *m != "auto" && *m != "forge")
    {
        None => Ok((None, None)),
        Some(m) => {
            let m = m.strip_prefix("forge/").unwrap_or(m);
            if f.config.settings.routes.contains_key(m) {
                return Ok((Some(m.to_string()), None));
            }
            if let Some((p, model)) = f.router.route_for_model(m) {
                return Ok((None, Some((p, Some(model)))));
            }
            Err(ApiError(
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": format!("unknown model '{m}'. Use a route name: {}", f.router.route_names().join(", ")), "type": "invalid_request_error"}}),
            ))
        }
    }
}

async fn chat_completions(State(f): S, Json(b): Json<ChatBody>) -> Result<Json<Value>, ApiError> {
    if b.stream == Some(true) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            json!({"error": {"message": "streaming is not supported yet; send stream: false", "type": "invalid_request_error"}}),
        ));
    }
    let (route, pin) = resolve_model(&f, b.model.as_deref())?;
    let json_mode =
        b.response_format.as_ref().and_then(|r| r["type"].as_str()) == Some("json_object");
    let r = f
        .router
        .complete(RouteRequest {
            route,
            pin,
            messages: b.messages,
            json_mode,
            max_tokens: b.max_tokens,
            temperature: b.temperature,
            task: "http:chat".into(),
            ..Default::default()
        })
        .await?;
    Ok(Json(openai_shape(&r)))
}

fn openai_shape(r: &Routed) -> Value {
    json!({
        "id": format!("forge-{}", r.call_id.unwrap_or(0)),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": r.model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": r.reply.text}, "finish_reason": r.reply.finish_reason.clone().unwrap_or_else(|| "stop".into())}],
        "usage": {"prompt_tokens": r.reply.prompt_tokens, "completion_tokens": r.reply.completion_tokens, "total_tokens": r.reply.prompt_tokens + r.reply.completion_tokens},
        "forge": {"route": r.route, "provider": r.provider, "tier": r.tier, "cost_usd": r.cost_usd, "call_id": r.call_id, "cached": r.cached, "attempts": r.attempts},
    })
}

#[derive(Deserialize)]
struct GenerateBody {
    prompt: String,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    route: Option<String>,
    #[serde(default)]
    json: bool,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    no_cache: bool,
}

async fn generate(State(f): S, Json(b): Json<GenerateBody>) -> Result<Json<Value>, ApiError> {
    let mut messages = Vec::new();
    if let Some(s) = b.system.filter(|s| !s.trim().is_empty()) {
        messages.push(Message::system(s));
    }
    messages.push(Message::user(b.prompt));
    let r = f
        .router
        .complete(RouteRequest {
            route: b.route,
            messages,
            json_mode: b.json,
            max_tokens: b.max_tokens,
            no_cache: b.no_cache,
            task: "http:generate".into(),
            ..Default::default()
        })
        .await?;
    Ok(Json(json!({
        "response": r.reply.text, "output": r.reply.text, "provider": r.provider, "model": r.model,
        "route": r.route, "cost_usd": r.cost_usd, "call_id": r.call_id, "cached": r.cached,
    })))
}

async fn audit(State(f): S, Json(b): Json<Value>) -> Result<Json<Value>, ApiError> {
    let id = f.store.audit_put(&b.to_string()).map_err(internal)?;
    Ok(Json(json!({"ok": true, "id": id})))
}

async fn candidate(State(f): S, body: axum::body::Bytes) -> Result<Response, ApiError> {
    let input: CandidateInput = serde_json::from_slice(&body).map_err(|e| {
        ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({"error": format!("candidate rejected: {e}. Put extra fields inside `atom`.")}),
        )
    })?;
    let receipt = canon::stage(&f.store, input)?;
    let status = if receipt.deduplicated {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(json!(receipt))).into_response())
}

#[derive(Deserialize)]
struct SearchQ {
    #[serde(default)]
    q: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    route: Option<String>,
    #[serde(default)]
    days: Option<u32>,
}

async fn search(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"results": canon::search(&f.store, q.q.trim(), q.limit.unwrap_or(10).min(200))?}),
    ))
}

async fn agenda(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"items": canon::agenda(&f.store, q.limit.unwrap_or(50).min(500))?}),
    ))
}

async fn plan(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    let req = RouteRequest {
        route: q.route,
        messages: vec![Message::user(q.q)],
        ..Default::default()
    };
    let (route, steps) = f
        .router
        .plan(&req)
        .map_err(|m| ApiError(StatusCode::BAD_REQUEST, json!({"error": m})))?;
    Ok(Json(
        json!({"route": route, "steps": steps, "limiter_ceiling": f.router.limiter.ceiling()}),
    ))
}

/// Every station of every API source (`apis.rs`): the main pipeline's `console.py list` rows unchanged, then the other
/// registered sources' stations, each tagged with `source` and `group`. A clear 503 when nothing can be read: the page
/// and the tray never fall back to a typed list. Errors of single sources are listed next to the stations.
async fn stations(State(f): S) -> Result<Json<Value>, ApiError> {
    let (sources, reg_error) = apis::resolve(&f.config.paths.base).await;
    let stations: Vec<Value> = sources.iter().flat_map(|r| r.stations.clone()).collect();
    let mut errors: Vec<Value> = sources
        .iter()
        .filter_map(|r| {
            r.error
                .as_ref()
                .map(|e| json!({"source": r.source.name, "error": e}))
        })
        .collect();
    if let Some(e) = reg_error {
        errors.push(json!({"source": "config/apis.json", "error": e}));
    }
    if stations.is_empty() {
        let msg = errors
            .first()
            .and_then(|e| e["error"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                "pipeline not found: add your API folder on the home page (or set PIPELINE_SYSTEM to API\\_system)"
                    .into()
            });
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": msg, "errors": errors}),
        ));
    }
    let mut workspace = json!({"EXCLUDED_FROM_PAGE": [], "groups": [["", "Front stations"]]});
    if let Some(source) = sources.iter().find(|s| s.main) {
        if let Some(console) = apis::pipeline_console(std::path::Path::new(&source.source.path)) {
            let policy = console
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("config/workspace.json");
            match std::fs::read_to_string(&policy)
                .map_err(|e| e.to_string())
                .and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()))
            {
                Ok(value) => workspace = value,
                Err(e) => errors.push(json!({"source": policy.display().to_string(), "error": e})),
            }
        }
    }
    Ok(Json(
        json!({"stations": stations, "errors": errors, "workspace": workspace}),
    ))
}

/// The API sources (main pipeline, registered folders, pages, links) with their station counts.
async fn apis_list(State(f): S) -> Json<Value> {
    let (sources, reg_error) = apis::resolve(&f.config.paths.base).await;
    Json(json!({
        "registry": apis::registry_path(&f.config.paths.base).display().to_string(),
        "registry_error": reg_error,
        "sources": sources.iter().map(|r| r.summary()).collect::<Vec<_>>(),
    }))
}

async fn apis_add(State(f): S, Json(s): Json<apis::Source>) -> Result<Json<Value>, ApiError> {
    apis::add(&f.config.paths.base, s)
        .map(|s| Json(json!({"added": s})))
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, json!({"error": e})))
}

#[derive(Deserialize)]
struct PathBody {
    path: String,
}

async fn apis_remove(State(f): S, Json(b): Json<PathBody>) -> Result<Json<Value>, ApiError> {
    apis::remove(&f.config.paths.base, &b.path)
        .map(|removed| Json(json!({"removed": removed})))
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, json!({"error": e})))
}

/// A registered `.html` page (Atom Builder, Reconciliation ...), served so the home page can link it.
async fn page(State(f): S, Path(i): Path<usize>) -> Result<axum::response::Html<String>, ApiError> {
    let reg = apis::load(&f.config.paths.base).map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": format!("{e:#}")}),
        )
    })?;
    let s = reg
        .sources
        .get(i)
        .filter(|s| apis::kind(&s.path) == apis::Kind::Page)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, json!({"error": "no such page"})))?;
    std::fs::read_to_string(&s.path)
        .map(axum::response::Html)
        .map_err(|e| {
            ApiError(
                StatusCode::NOT_FOUND,
                json!({"error": format!("{}: {e}", s.path)}),
            )
        })
}

/// Run a station's button (or open its folder) from the page: only a path the current station feed lists, never one
/// the request makes up. Same as double-clicking it: `start "" <path>` in its own window.
async fn launch(State(f): S, Json(b): Json<PathBody>) -> Result<Json<Value>, ApiError> {
    let (sources, _) = apis::resolve(&f.config.paths.base).await;
    let listed = sources.iter().flat_map(|r| r.stations.iter()).any(|st| {
        st["front"].as_str() == Some(b.path.as_str())
            || st["buttons"].as_array().is_some_and(|bs| {
                bs.iter()
                    .any(|x| x["path"].as_str() == Some(b.path.as_str()))
            })
    });
    if !listed {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            json!({"error": "not a station button or folder in the current feed"}),
        ));
    }
    if !cfg!(windows) {
        return Err(ApiError(
            StatusCode::NOT_IMPLEMENTED,
            json!({"error": "launching stations is Windows-only"}),
        ));
    }
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &b.path])
        .spawn()
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": e.to_string()}),
            )
        })?;
    Ok(Json(json!({"started": b.path})))
}

/// The home page: every API source and station on one page. `<base>/ui/home.html` overrides the built-in copy.
async fn home_page(State(f): S) -> axum::response::Html<String> {
    const BUILT_IN: &str = include_str!("../../../ui/home.html");
    let disk = std::fs::read_to_string(f.config.paths.base.join("ui").join("home.html"));
    axum::response::Html(disk.unwrap_or_else(|_| BUILT_IN.to_string()))
}

async fn pack_workbench() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../../../ui/workbench.html"))
}

/// The workspace page: `<base>/ui/api-workspace.html` if present (edit without rebuilding), else the
/// copy built into the binary.
async fn workspace_page(State(f): S) -> axum::response::Html<String> {
    const BUILT_IN: &str = include_str!("../../../ui/api-workspace.html");
    let disk = std::fs::read_to_string(f.config.paths.base.join("ui").join("api-workspace.html"));
    axum::response::Html(disk.unwrap_or_else(|_| BUILT_IN.to_string()))
}

#[derive(Deserialize)]
struct GridQ {
    run: Option<String>,
}

async fn grid(State(f): S, Query(q): Query<GridQ>) -> Result<Json<Value>, ApiError> {
    runs::grid(&f.store, q.run.as_deref())
        .map(Json)
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, json!({"error": format!("{e:#}")})))
}

async fn runs_list(State(f): S) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"runs": runs::list(&f.store, 50).map_err(internal)?}),
    ))
}

async fn runs_get(State(f): S, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    runs::get(&f.store, &id)
        .map(|r| Json(json!(r)))
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, json!({"error": format!("{e:#}")})))
}

#[derive(Deserialize)]
struct StartReq {
    path: String,
    /// `[["summary","claims"],["review"]]`
    stages: Vec<Vec<String>>,
    #[serde(default = "mode_all")]
    mode: String,
    out: Option<String>,
    route: Option<String>,
    workers: Option<usize>,
    #[serde(default)]
    redo: bool,
    /// Required above 10 documents (the CLI asks; a page has to say so).
    #[serde(default)]
    confirm: bool,
}

fn mode_all() -> String {
    "all".into()
}

fn bad(e: impl std::fmt::Display) -> ApiError {
    ApiError(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({"error": e.to_string()}),
    )
}

/// Run in the background; the page polls `/api/grid`.
fn spawn_run(f: &Arc<Forge>, id: String) {
    let f = Arc::clone(f);
    tokio::spawn(async move {
        if let Err(e) = runs::execute(&f, &id).await {
            tracing::warn!("run {id}: {e:#}");
        }
    });
}

async fn runs_start(State(f): S, Json(b): Json<StartReq>) -> Result<Json<Value>, ApiError> {
    let mode: runs::Mode = b.mode.parse().map_err(bad)?;
    let path = std::path::PathBuf::from(&b.path);
    let docs = forge_core::pipeline::collect(&path).map_err(bad)?;
    if docs.is_empty() {
        return Err(bad(format!("no .md/.txt documents at {}", b.path)));
    }
    if docs.len() > 10 && !b.confirm {
        return Err(ApiError(
            StatusCode::CONFLICT,
            json!({"error": "more than 10 documents: resend with confirm: true", "documents": docs.len(), "needs_confirm": true}),
        ));
    }
    let spec = runs::RunSpec {
        stages: b.stages,
        outbox: b
            .out
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::mcp::default_outbox(&path)),
        route: b.route,
        redo: b.redo,
        workers: b.workers.unwrap_or(4).clamp(1, 60),
    };
    let r = runs::create(&f, mode, spec, docs).map_err(bad)?;
    spawn_run(&f, r.id.clone());
    Ok(Json(json!(r)))
}

async fn runs_action(
    State(f): S,
    Path((id, action)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let r = match action.as_str() {
        "approve" => runs::approve(&f.store, &id),
        "resume" => runs::resume(&f.store, &id),
        "pause" => runs::pause(&f.store, &id),
        "cancel" => runs::cancel(&f.store, &id),
        _ => {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                json!({"error": "want approve | pause | resume | cancel"}),
            ))
        }
    }
    .map_err(|e| ApiError(StatusCode::CONFLICT, json!({"error": format!("{e:#}")})))?;
    if r.status == "running" {
        spawn_run(&f, id);
    }
    Ok(Json(json!(r)))
}

async fn uid_orphans(State(f): S) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"orphans": forge_core::uid::orphans(&f.store, 200).map_err(internal)?}),
    ))
}

async fn uid_show(State(f): S, Path(uid): Path<String>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "uid": uid,
        "versions": forge_core::uid::versions(&f.store, &uid).map_err(internal)?,
        "outputs": f.store.derivatives_by_uid(&uid, 200).map_err(internal)?,
    })))
}

async fn templates(State(f): S) -> Json<Value> {
    Json(json!({"templates": forge_core::template::list(&f.config.paths.templates_dir)}))
}

async fn focus(State(f): S) -> Json<Value> {
    let path = f
        .config
        .paths
        .focus_path(f.config.settings.focus_file.as_deref());
    match f.router.focus() {
        Some((text, sha)) => {
            Json(json!({"path": path, "active": true, "sha256": sha, "text": text}))
        }
        None => Json(json!({"path": path, "active": false})),
    }
}

async fn ledger_recent(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"calls": f.store.recent_calls(q.limit.unwrap_or(20).min(500)).map_err(internal)?}),
    ))
}

async fn ledger_stats(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    let days = q.days.unwrap_or(1);
    Ok(Json(
        json!({"days": days, "spend_today_usd": f.store.spend_today().map_err(internal)?, "by_model": f.store.stats(days).map_err(internal)?}),
    ))
}

async fn context_search(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    let hits = f
        .store
        .context_search(
            &q.q,
            q.limit.unwrap_or(5).min(50) as usize,
            q.kind.as_deref(),
        )
        .map_err(internal)?;
    Ok(Json(json!({"hits": hits})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use forge_core::config::{Config, DEFAULT_PROVIDERS, DEFAULT_SETTINGS};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn forge() -> Arc<Forge> {
        let mut cfg = Config::from_strings(
            DEFAULT_PROVIDERS,
            DEFAULT_SETTINGS,
            std::path::PathBuf::from("."),
            None,
        )
        .unwrap();
        cfg.settings.default_route = Some("test".into());
        Forge::with_store(cfg, forge_core::Store::open_memory().unwrap()).unwrap()
    }

    async fn call(app: Http, req: Request<Body>) -> (StatusCode, Value, axum::http::HeaderMap) {
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            headers,
        )
    }

    fn post(uri: &str, body: Value) -> Request<Body> {
        Request::post(uri)
            .header("content-type", "application/json")
            .header("origin", "null")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn openai_endpoint_routes_through_mock_and_logs() {
        let f = forge();
        let (s, v, h) = call(
            app(f.clone()),
            post(
                "/v1/chat/completions",
                json!({"model": "test", "messages": [{"role": "user", "content": "hello"}]}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .starts_with("[mock:mock-1]"));
        assert_eq!(v["forge"]["provider"], "mock");
        assert_eq!(h.get("access-control-allow-origin").unwrap(), "null");
        assert_eq!(f.store.recent_calls(5).unwrap().len(), 1);
        let (s, _, _) = call(
            app(f),
            post(
                "/v1/chat/completions",
                json!({"model": "nope", "messages": []}),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn foreign_origins_get_no_cors_grant() {
        let req = Request::get("/health")
            .header("origin", "https://evil.example")
            .body(Body::empty())
            .unwrap();
        let (s, _, h) = call(app(forge()), req).await;
        assert_eq!(s, StatusCode::OK);
        assert!(h.get("access-control-allow-origin").is_none());
        assert!(origin_ok(
            &HeaderValue::from_static("http://localhost:5173"),
            &[]
        ));
        assert!(!origin_ok(
            &HeaderValue::from_static("http://localhost.evil.com"),
            &[]
        ));
    }

    #[tokio::test]
    async fn canon_contract_round_trip() {
        let f = forge();
        let body = json!({"atom_id": "A042", "author": "david", "atom": {"object_type": "CLAIM", "register": "THEOLOGICAL"}});
        let (s, r, _) = call(app(f.clone()), post("/candidate", body.clone())).await;
        assert_eq!(s, StatusCode::CREATED, "{r}");
        assert_eq!(r["note"], "accepted and staged. this is not canon.");
        let (s, r2, _) = call(app(f.clone()), post("/candidate", body)).await;
        assert_eq!(
            (s, r2["candidate_id"].clone()),
            (StatusCode::OK, r["candidate_id"].clone())
        );
        let (s, e, _) = call(
            app(f.clone()),
            post("/candidate", json!({"atom": {}, "reconciliation": []})),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(e["error"].as_str().unwrap().contains("reconciliation"));
        let (_, found, _) = call(
            app(f.clone()),
            Request::get("/search?q=A042&limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(found["results"][0]["payload"]["register"], "THEOLOGICAL");
        let (_, gen, _) = call(
            app(f),
            post("/api/generate", json!({"prompt": "x", "system": "y"})),
        )
        .await;
        assert_eq!(gen["provider"], "mock");
    }

    #[tokio::test]
    async fn stations_feed_is_the_pipeline_console_and_never_a_typed_list() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = |base: std::path::PathBuf| {
            Config::from_strings(DEFAULT_PROVIDERS, DEFAULT_SETTINGS, base, None).unwrap()
        };
        // No pipeline here: a clear 503, not an empty or built-in list.
        let f = Forge::with_store(
            cfg(dir.path().to_path_buf()),
            forge_core::Store::open_memory().unwrap(),
        )
        .unwrap();
        let resp = app(f)
            .oneshot(Request::get("/api/stations").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

        // A pipeline _system whose console.py prints the feed: forge passes it through unchanged.
        std::fs::create_dir_all(dir.path().join("engine")).unwrap();
        std::fs::write(
            dir.path().join("engine/console.py"),
            "import json\nprint(json.dumps({'stations': [{'number': '101', 'label': '101_FIS_INVENTORY', 'family': '10_FIS'}]}))\n",
        )
        .unwrap();
        let f = Forge::with_store(
            cfg(dir.path().to_path_buf()),
            forge_core::Store::open_memory().unwrap(),
        )
        .unwrap();
        let resp = app(f)
            .oneshot(Request::get("/api/stations").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["stations"][0]["family"], "10_FIS");
    }

    #[tokio::test]
    async fn home_page_lists_every_source_on_one_feed_and_launches_only_listed_paths() {
        let dir = tempfile::tempdir().unwrap();
        let f = Forge::with_store(
            Config::from_strings(
                DEFAULT_PROVIDERS,
                DEFAULT_SETTINGS,
                dir.path().to_path_buf(),
                None,
            )
            .unwrap(),
            forge_core::Store::open_memory().unwrap(),
        )
        .unwrap();
        let (st, _, _) = call(
            app(f.clone()),
            Request::get("/").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::OK);

        // A main pipeline and a folder of other stations (Atoms), both registered from the page.
        let api = dir.path().join("API");
        std::fs::create_dir_all(api.join("_system/engine")).unwrap();
        std::fs::write(
            api.join("_system/engine/console.py"),
            "import json\nprint(json.dumps({'stations': [{'label': '101_FIS_INVENTORY', 'family': '10_FIS', 'buttons': []}]}))\n",
        )
        .unwrap();
        let atoms = dir.path().join("Atoms").join("201_ATOM_BUILDER");
        std::fs::create_dir_all(&atoms).unwrap();
        std::fs::write(atoms.join("1 RUN HERE.bat"), "").unwrap();
        for (path, group) in [(&api, ""), (&dir.path().join("Atoms"), "Atoms & Axioms")] {
            let (st, body, _) = call(
                app(f.clone()),
                post(
                    "/api/apis",
                    json!({"name": "", "group": group, "path": path.display().to_string()}),
                ),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{body}");
        }
        let (st, bad, _) = call(
            app(f.clone()),
            post(
                "/api/apis",
                json!({"name": "x", "path": dir.path().join("nope").display().to_string()}),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");

        let (st, list, _) = call(
            app(f.clone()),
            Request::get("/api/apis").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(list["sources"][0]["kind"], "pipeline");
        assert_eq!(list["sources"][0]["main"], true);
        assert_eq!(list["sources"][1]["kind"], "folder");

        let (st, feed, _) = call(
            app(f.clone()),
            Request::get("/api/stations").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{feed}");
        assert_eq!(feed["stations"][0]["family"], "10_FIS");
        assert!(feed["stations"][0].get("group").is_none());
        assert_eq!(feed["stations"][1]["group"], "Atoms & Axioms");
        assert_eq!(feed["stations"][1]["label"], "201_ATOM_BUILDER");

        // Only a path the feed lists may be launched.
        let (st, _, _) = call(
            app(f.clone()),
            post(
                "/api/launch",
                json!({"path": "C:\\Windows\\System32\\calc.exe"}),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn workspace_page_and_a_gated_run_through_http() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("templates")).unwrap();
        std::fs::write(
            dir.path().join("templates/a.md"),
            "---\nsuffix: A\n---\n{{ document }}",
        )
        .unwrap();
        let inbox = dir.path().join("INBOX");
        std::fs::create_dir_all(&inbox).unwrap();
        for i in 0..2 {
            std::fs::write(inbox.join(format!("p{i}.md")), format!("paper {i}")).unwrap();
        }
        let mut cfg = Config::from_strings(
            DEFAULT_PROVIDERS,
            DEFAULT_SETTINGS,
            dir.path().to_path_buf(),
            None,
        )
        .unwrap();
        cfg.settings.default_route = Some("test".into());
        let f = Forge::with_store(cfg, forge_core::Store::open_memory().unwrap()).unwrap();

        let resp = app(f.clone())
            .oneshot(Request::get("/workspace").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&html).contains("<html"));

        let (st, run, _) = call(
            app(f.clone()),
            post("/api/runs", json!({"path": inbox.display().to_string(), "stages": [["a"]], "mode": "first-then-approve"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{run}");
        let id = run["id"].as_str().unwrap().to_string();
        // The run executes in the background; wait for the gate.
        let mut grid = Value::Null;
        for _ in 0..200 {
            let (_, g, _) = call(
                app(f.clone()),
                Request::get(format!("/api/grid?run={id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            if g["run"]["status"] == "awaiting_approval" {
                grid = g;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(grid["rows"][0]["cells"]["a"]["status"], "ok", "{grid}");
        assert_eq!(grid["rows"][1]["cells"]["a"]["status"], "awaiting_approval");
        assert_eq!(grid["rows"][0]["uid"].as_str().unwrap().len(), 26);

        let (st, _, _) = call(
            app(f.clone()),
            post(&format!("/api/runs/{id}/resume"), json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT, "the gate needs approve");
        let (st, _, _) = call(
            app(f.clone()),
            post(&format!("/api/runs/{id}/approve"), json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        for _ in 0..200 {
            if forge_core::runs::get(&f.store, &id).unwrap().status == "done" {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("run never finished");
    }
}
