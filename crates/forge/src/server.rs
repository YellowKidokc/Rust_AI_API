//! Local HTTP gateway (127.0.0.1 only). One port serves three audiences:
//!
//! * any OpenAI-style client: `POST /v1/chat/completions`, `GET /v1/models` (model = a route name,
//!   or a model that appears in a route, or `auto`)
//! * the existing HTML pages: `POST /api/generate` and `/api/atom-builder/raw` (`{prompt, system}`),
//!   `POST /api/audit`, and the Canon Workbench contract `GET /health`, `GET /search`, `GET /agenda`,
//!   `POST /candidate`
//! * humans and other agents: `GET /route/plan`, `GET /ledger/recent`, `GET /ledger/stats`,
//!   `GET /context/search`
//!
//! Keys stay in this process. Browsers are only allowed from `file://` pages (Origin `null`) and
//! localhost, plus `server.cors_origins`, so a website you visit can't spend your credits.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router as Http};
use forge_core::canon::{self, CandidateInput, CanonError};
use forge_core::router::{RouteFailure, Routed};
use forge_core::{Forge, Message, RouteRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::cors::{AllowOrigin, CorsLayer};

type S = State<Arc<Forge>>;

pub fn app(forge: Arc<Forge>) -> Http {
    let extra: Vec<String> = forge.config.settings.server.cors_origins.clone();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin: &HeaderValue, _| origin_ok(origin, &extra)))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([axum::http::header::CONTENT_TYPE, axum::http::header::AUTHORIZATION]);

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
        .layer(cors)
        .with_state(forge)
}

fn origin_ok(origin: &HeaderValue, extra: &[String]) -> bool {
    let Ok(o) = origin.to_str() else { return false };
    if o == "null" {
        return true;
    }
    for local in ["http://127.0.0.1", "http://localhost", "http://[::1]"] {
        if o == local || o.strip_prefix(local).is_some_and(|rest| rest.starts_with(':')) {
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
    eprintln!("  OpenAI-compatible: http://{addr}/v1/chat/completions   routes: {}", forge.router.route_names().join(", "));
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
            CanonError::Invalid(m) => ApiError(StatusCode::UNPROCESSABLE_ENTITY, json!({"error": m})),
            CanonError::Db(e) => ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
        }
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()}))
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
    let data: Vec<Value> = f.router.route_names().into_iter().map(|r| json!({"id": r, "object": "model", "owned_by": "forge"})).collect();
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
    match model.map(str::trim).filter(|m| !m.is_empty() && *m != "auto" && *m != "forge") {
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
    let json_mode = b.response_format.as_ref().and_then(|r| r["type"].as_str()) == Some("json_object");
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
        ApiError(StatusCode::UNPROCESSABLE_ENTITY, json!({"error": format!("candidate rejected: {e}. Put extra fields inside `atom`.")}))
    })?;
    let receipt = canon::stage(&f.store, input)?;
    let status = if receipt.deduplicated { StatusCode::OK } else { StatusCode::CREATED };
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
    Ok(Json(json!({"results": canon::search(&f.store, q.q.trim(), q.limit.unwrap_or(10).min(200))?})))
}

async fn agenda(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"items": canon::agenda(&f.store, q.limit.unwrap_or(50).min(500))?})))
}

async fn plan(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    let req = RouteRequest { route: q.route, messages: vec![Message::user(q.q)], ..Default::default() };
    let (route, steps) = f.router.plan(&req).map_err(|m| ApiError(StatusCode::BAD_REQUEST, json!({"error": m})))?;
    Ok(Json(json!({"route": route, "steps": steps, "limiter_ceiling": f.router.limiter.ceiling()})))
}

async fn ledger_recent(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"calls": f.store.recent_calls(q.limit.unwrap_or(20).min(500)).map_err(internal)?})))
}

async fn ledger_stats(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    let days = q.days.unwrap_or(1);
    Ok(Json(
        json!({"days": days, "spend_today_usd": f.store.spend_today().map_err(internal)?, "by_model": f.store.stats(days).map_err(internal)?}),
    ))
}

async fn context_search(State(f): S, Query(q): Query<SearchQ>) -> Result<Json<Value>, ApiError> {
    let hits = f.store.context_search(&q.q, q.limit.unwrap_or(5).min(50) as usize, q.kind.as_deref()).map_err(internal)?;
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
        let mut cfg = Config::from_strings(DEFAULT_PROVIDERS, DEFAULT_SETTINGS, std::path::PathBuf::from("."), None).unwrap();
        cfg.settings.default_route = Some("test".into());
        Forge::with_store(cfg, forge_core::Store::open_memory().unwrap()).unwrap()
    }

    async fn call(app: Http, req: Request<Body>) -> (StatusCode, Value, axum::http::HeaderMap) {
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), headers)
    }

    fn post(uri: &str, body: Value) -> Request<Body> {
        Request::post(uri).header("content-type", "application/json").header("origin", "null").body(Body::from(body.to_string())).unwrap()
    }

    #[tokio::test]
    async fn openai_endpoint_routes_through_mock_and_logs() {
        let f = forge();
        let (s, v, h) = call(
            app(f.clone()),
            post("/v1/chat/completions", json!({"model": "test", "messages": [{"role": "user", "content": "hello"}]})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["choices"][0]["message"]["content"].as_str().unwrap().starts_with("[mock:mock-1]"));
        assert_eq!(v["forge"]["provider"], "mock");
        assert_eq!(h.get("access-control-allow-origin").unwrap(), "null");
        assert_eq!(f.store.recent_calls(5).unwrap().len(), 1);
        let (s, _, _) = call(app(f), post("/v1/chat/completions", json!({"model": "nope", "messages": []}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn foreign_origins_get_no_cors_grant() {
        let req = Request::get("/health").header("origin", "https://evil.example").body(Body::empty()).unwrap();
        let (s, _, h) = call(app(forge()), req).await;
        assert_eq!(s, StatusCode::OK);
        assert!(h.get("access-control-allow-origin").is_none());
        assert!(origin_ok(&HeaderValue::from_static("http://localhost:5173"), &[]));
        assert!(!origin_ok(&HeaderValue::from_static("http://localhost.evil.com"), &[]));
    }

    #[tokio::test]
    async fn canon_contract_round_trip() {
        let f = forge();
        let body = json!({"atom_id": "A042", "author": "david", "atom": {"object_type": "CLAIM", "register": "THEOLOGICAL"}});
        let (s, r, _) = call(app(f.clone()), post("/candidate", body.clone())).await;
        assert_eq!(s, StatusCode::CREATED, "{r}");
        assert_eq!(r["note"], "accepted and staged. this is not canon.");
        let (s, r2, _) = call(app(f.clone()), post("/candidate", body)).await;
        assert_eq!((s, r2["candidate_id"].clone()), (StatusCode::OK, r["candidate_id"].clone()));
        let (s, e, _) = call(app(f.clone()), post("/candidate", json!({"atom": {}, "reconciliation": []}))).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(e["error"].as_str().unwrap().contains("reconciliation"));
        let (_, found, _) = call(app(f.clone()), Request::get("/search?q=A042&limit=5").body(Body::empty()).unwrap()).await;
        assert_eq!(found["results"][0]["payload"]["register"], "THEOLOGICAL");
        let (_, gen, _) = call(app(f), post("/api/generate", json!({"prompt": "x", "system": "y"}))).await;
        assert_eq!(gen["provider"], "mock");
    }
}
