//! The cascade router. A route is an ordered ladder of (provider, model) rungs. For each request the
//! router decides which rungs are eligible right now (key present, allowed, not cooling down after a 429,
//! inside its time window, under its daily quota, under the daily budget, big enough context window),
//! then walks them in order: retry transient failures with back-off, move down a rung on anything else.
//! Every attempt is written to the ledger.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::{Config, RouteEntry, Tier};
use crate::limiter::AdaptiveLimiter;
use crate::provider::{self, Provider};
use crate::store::{now, CachedReply, CallRecord, Store};
use crate::types::{estimate_tokens, ChatReply, ChatRequest, LlmError, Message};

#[derive(Debug, Clone, Default)]
pub struct RouteRequest {
    pub route: Option<String>,
    pub messages: Vec<Message>,
    pub json_mode: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Skip rungs above this tier (e.g. `Free` = never spend money).
    pub max_tier: Option<Tier>,
    /// Use only this provider (and model, if given), ignoring the route.
    pub pin: Option<(String, Option<String>)>,
    pub task: String,
    pub doc_id: Option<String>,
    /// Always call a model, even if this exact request was answered before.
    pub no_cache: bool,
    /// Permanent paper id, recorded on every ledger row for this request.
    pub uid: Option<String>,
    /// Send without GLOBAL_FOCUS.md. Logged on the call row as `focus_sha256 = "off"`.
    pub no_focus: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanStep {
    pub rung: usize,
    pub provider: String,
    pub model: String,
    pub tier: Tier,
    pub eligible: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    pub provider: String,
    pub model: String,
    pub attempt: u32,
    pub ok: bool,
    pub error_kind: Option<String>,
    pub error: Option<String>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Routed {
    pub reply: ChatReply,
    pub route: String,
    pub provider: String,
    pub model: String,
    pub tier: Tier,
    pub cost_usd: f64,
    pub call_id: Option<i64>,
    pub attempts: Vec<Attempt>,
    /// Served from the response cache: no model was called and nothing was spent.
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct RouteFailure {
    pub message: String,
    pub plan: Vec<PlanStep>,
    pub attempts: Vec<Attempt>,
}

pub struct Router {
    cfg: Config,
    providers: HashMap<String, Arc<dyn Provider>>,
    unavailable: BTreeMap<String, String>,
    cooldowns: Mutex<HashMap<(String, String), Instant>>,
    pub limiter: Arc<AdaptiveLimiter>,
    pub store: Store,
}

impl Router {
    /// Build every provider in `providers.json` whose key is present.
    pub fn new(cfg: Config, store: Store) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.settings.request_timeout_seconds))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        let mut unavailable = BTreeMap::new();
        for name in cfg.providers.keys() {
            let Some(def) = cfg.provider(name) else {
                unavailable.insert(
                    name.clone(),
                    "alias_of points at an unknown provider".into(),
                );
                continue;
            };
            match provider::build(name, def, client.clone()) {
                Ok(p) => {
                    providers.insert(name.clone(), p);
                }
                Err(e) => {
                    unavailable.insert(name.clone(), e.to_string());
                }
            }
        }
        let limiter = AdaptiveLimiter::new(cfg.settings.max_concurrent_calls);
        Ok(Self {
            cfg,
            providers,
            unavailable,
            cooldowns: Mutex::new(HashMap::new()),
            limiter,
            store,
        })
    }

    /// Replace or add a provider (tests, or a custom front end registering its own).
    pub fn insert_provider(&mut self, name: &str, p: Arc<dyn Provider>) {
        self.unavailable.remove(name);
        self.providers.insert(name.to_string(), p);
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn route_names(&self) -> Vec<String> {
        self.cfg.settings.routes.keys().cloned().collect()
    }

    pub fn available_providers(&self) -> Vec<String> {
        let mut v: Vec<String> = self.providers.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn unavailable_providers(&self) -> &BTreeMap<String, String> {
        &self.unavailable
    }

    fn ladder(&self, req: &RouteRequest) -> Result<(String, Vec<RouteEntry>), String> {
        if let Some((p, m)) = &req.pin {
            let model = m
                .clone()
                .or_else(|| self.cfg.provider(p).and_then(|d| d.default_model.clone()));
            let tier = self
                .find_entry(p, model.as_deref())
                .map(|e| e.tier)
                .unwrap_or_default();
            let mut e = self
                .find_entry(p, model.as_deref())
                .cloned()
                .unwrap_or(RouteEntry {
                    provider: p.clone(),
                    model: None,
                    tier,
                    daily_requests: None,
                    window_utc: None,
                    price_in: 0.0,
                    price_out: 0.0,
                    context_window: None,
                    max_output: None,
                });
            e.model = model;
            return Ok((format!("pin:{p}"), vec![e]));
        }
        let name = req
            .route
            .clone()
            .unwrap_or_else(|| self.cfg.settings.default_route_name());
        match self.cfg.settings.routes.get(&name) {
            Some(l) if !l.is_empty() => Ok((name, l.clone())),
            _ => Err(format!(
                "unknown or empty route '{name}'. Known routes: {}",
                self.route_names().join(", ")
            )),
        }
    }

    /// Find a configured rung for this provider/model in any route (for its prices and tier).
    pub fn find_entry(&self, provider: &str, model: Option<&str>) -> Option<&RouteEntry> {
        self.cfg
            .settings
            .routes
            .values()
            .flatten()
            .find(|e| e.provider == provider && (model.is_none() || e.model.as_deref() == model))
    }

    /// Which route contains a rung with this model name (OpenAI-style `model` → route resolution).
    pub fn route_for_model(&self, model: &str) -> Option<(String, String)> {
        self.cfg
            .settings
            .routes
            .values()
            .flatten()
            .find(|e| e.model.as_deref() == Some(model))
            .map(|e| (e.provider.clone(), model.to_string()))
    }

    /// Explain what would be tried, in order, and why each rung is in or out.
    pub fn plan(&self, req: &RouteRequest) -> Result<(String, Vec<PlanStep>), String> {
        let (route, ladder) = self.ladder(req)?;
        let est_in = estimate_tokens(req.messages.iter().map(|m| m.content.len()).sum());
        let out_cap = req.max_tokens.unwrap_or(self.cfg.settings.output_token_cap);
        let spend = self.store.spend_today().unwrap_or(0.0);
        let now_utc = chrono::Utc::now();
        let steps = ladder
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let model = e
                    .model
                    .clone()
                    .or_else(|| {
                        self.cfg
                            .provider(&e.provider)
                            .and_then(|d| d.default_model.clone())
                    })
                    .unwrap_or_default();
                let reason = self.ineligible_reason(
                    e,
                    &model,
                    est_in,
                    out_cap,
                    spend,
                    req.max_tier,
                    now_utc,
                );
                PlanStep {
                    rung: i,
                    provider: e.provider.clone(),
                    model,
                    tier: e.tier,
                    eligible: reason.is_none(),
                    reason: reason.unwrap_or_else(|| "eligible".into()),
                }
            })
            .collect();
        Ok((route, steps))
    }

    #[allow(clippy::too_many_arguments)]
    fn ineligible_reason(
        &self,
        e: &RouteEntry,
        model: &str,
        est_in: u32,
        out_cap: u32,
        spend: f64,
        max_tier: Option<Tier>,
        now_utc: chrono::DateTime<chrono::Utc>,
    ) -> Option<String> {
        if model.is_empty() {
            return Some("no model set and provider has no default_model".into());
        }
        if !self.cfg.provider_allowed(&e.provider) {
            return Some("not in allowed_providers".into());
        }
        if !self.cfg.providers.contains_key(&e.provider) {
            return Some("unknown provider".into());
        }
        if let Some(why) = self.unavailable.get(&e.provider) {
            return Some(why.clone());
        }
        if let Some(t) = max_tier {
            if e.tier > t {
                return Some(format!("tier {} above max {}", e.tier.as_str(), t.as_str()));
            }
        }
        if let Some(until) = self
            .cooldowns
            .lock()
            .unwrap()
            .get(&(e.provider.clone(), model.to_string()))
        {
            let now = Instant::now();
            if *until > now {
                return Some(format!(
                    "cooling down after 429 ({}s left)",
                    (*until - now).as_secs()
                ));
            }
        }
        if let Some(w) = &e.window_utc {
            match in_window(w, now_utc) {
                Some(false) => return Some(format!("outside window {w} UTC")),
                None => return Some(format!("bad window_utc '{w}' (want HH:MM-HH:MM)")),
                Some(true) => {}
            }
        }
        if let Some(limit) = e.daily_requests {
            let used = self.store.requests_today(&e.provider, model).unwrap_or(0);
            if used >= limit {
                return Some(format!("daily quota used ({used}/{limit})"));
            }
        }
        if e.price_in > 0.0 || e.price_out > 0.0 {
            if let Some(budget) = self.cfg.settings.budget.daily_usd {
                if spend >= budget {
                    return Some(format!(
                        "daily budget reached (${spend:.2} of ${budget:.2})"
                    ));
                }
            }
        }
        if let Some(ctx) = e.context_window {
            if est_in + out_cap > ctx {
                return Some(format!(
                    "~{est_in} input + {out_cap} output tokens exceeds context window {ctx}"
                ));
            }
        }
        None
    }

    /// GLOBAL_FOCUS.md text and its SHA-256, if the file exists and isn't blank. Read per call so an
    /// edit takes effect on the next request.
    pub fn focus(&self) -> Option<(String, String)> {
        let path = self
            .cfg
            .paths
            .focus_path(self.cfg.settings.focus_file.as_deref());
        let raw = std::fs::read_to_string(path).ok()?;
        let text = raw
            .strip_prefix('\u{feff}')
            .unwrap_or(&raw)
            .trim()
            .to_string();
        if text.is_empty() {
            return None;
        }
        let sha = hex::encode(Sha256::digest(text.as_bytes()));
        Some((text, sha))
    }

    pub async fn complete(&self, mut req: RouteRequest) -> Result<Routed, RouteFailure> {
        // Focus goes in before the cache key is computed, so changing the focus can't serve a stale reply.
        let focus_sha = if req.no_focus {
            Some("off".to_string())
        } else if let Some((text, sha)) = self.focus() {
            req.messages.insert(0, Message::system(text));
            Some(sha)
        } else {
            None
        };
        let (route, plan) = self.plan(&req).map_err(|m| RouteFailure {
            message: m,
            plan: vec![],
            attempts: vec![],
        })?;
        let s = &self.cfg.settings;
        let prompt_text: String = req
            .messages
            .iter()
            .map(|m| format!("[{}]\n{}\n", m.role, m.content))
            .collect();
        let prompt_sha = hex::encode(Sha256::digest(prompt_text.as_bytes()));
        let mut attempts = Vec::new();

        let use_cache = s.cache.enabled && !req.no_cache;
        let cache_key = cache_key(&route, &req, s.output_token_cap, s.temperature);
        if use_cache {
            if let Ok(Some(hit)) = self.store.cache_get(&cache_key) {
                let tier = hit
                    .tier
                    .as_deref()
                    .and_then(|t| serde_json::from_value(serde_json::json!(t)).ok())
                    .unwrap_or_default();
                let t = now();
                let rec = CallRecord {
                    task: if req.task.is_empty() {
                        "adhoc".into()
                    } else {
                        req.task.clone()
                    },
                    doc_id: req.doc_id.clone(),
                    uid: req.uid.clone(),
                    focus_sha256: focus_sha.clone(),
                    route: Some(route.clone()),
                    provider: hit.provider.clone(),
                    model: hit.model.clone(),
                    tier: hit.tier.clone().unwrap_or_default(),
                    status: "CACHED".into(),
                    prompt_tokens: hit.prompt_tokens,
                    completion_tokens: hit.completion_tokens,
                    finish_reason: hit.finish_reason.clone(),
                    prompt_sha256: prompt_sha.clone(),
                    started_at: t.clone(),
                    finished_at: t,
                    ..Default::default()
                };
                let call_id = self.store.record_call(&rec).ok();
                self.receipt(&rec);
                return Ok(Routed {
                    reply: ChatReply {
                        text: hit.text,
                        prompt_tokens: hit.prompt_tokens,
                        completion_tokens: hit.completion_tokens,
                        finish_reason: hit.finish_reason,
                        model: hit.model.clone(),
                    },
                    route,
                    provider: hit.provider,
                    model: hit.model,
                    tier,
                    cost_usd: 0.0,
                    call_id,
                    attempts,
                    cached: true,
                });
            }
        }

        for step in plan.iter().filter(|p| p.eligible) {
            let Some(provider) = self.providers.get(&step.provider).cloned() else {
                continue;
            };
            let entry = self.find_entry(&step.provider, Some(&step.model)).cloned();
            let (price_in, price_out) = entry
                .as_ref()
                .map(|e| (e.price_in, e.price_out))
                .unwrap_or((0.0, 0.0));
            let mut max_tokens = req.max_tokens.unwrap_or(s.output_token_cap);
            if let Some(m) = entry.as_ref().and_then(|e| e.max_output) {
                max_tokens = max_tokens.min(m);
            }
            let chat = ChatRequest {
                model: step.model.clone(),
                messages: req.messages.clone(),
                temperature: req.temperature.unwrap_or(s.temperature),
                max_tokens,
                json_mode: req.json_mode,
            };

            for attempt in 1..=s.retry.attempts.max(1) {
                let started = now();
                let t0 = Instant::now();
                let result = {
                    let _permit = self.limiter.acquire().await;
                    provider.complete(&chat).await
                };
                let latency_ms = t0.elapsed().as_millis() as u64;
                self.limiter.record(result.is_ok());

                let mut rec = CallRecord {
                    task: if req.task.is_empty() {
                        "adhoc".into()
                    } else {
                        req.task.clone()
                    },
                    doc_id: req.doc_id.clone(),
                    uid: req.uid.clone(),
                    focus_sha256: focus_sha.clone(),
                    route: Some(route.clone()),
                    provider: step.provider.clone(),
                    model: step.model.clone(),
                    tier: step.tier.as_str().into(),
                    rung: step.rung as u32,
                    attempt,
                    latency_ms,
                    prompt_sha256: prompt_sha.clone(),
                    prompt_text: s.ledger.store_prompts.then(|| prompt_text.clone()),
                    started_at: started,
                    finished_at: now(),
                    ..Default::default()
                };

                match result {
                    Ok(reply) => {
                        let cost = (reply.prompt_tokens as f64 * price_in
                            + reply.completion_tokens as f64 * price_out)
                            / 1e6;
                        rec.status = "SUCCESS".into();
                        rec.prompt_tokens = reply.prompt_tokens;
                        rec.completion_tokens = reply.completion_tokens;
                        rec.cost_usd = cost;
                        rec.finish_reason = reply.finish_reason.clone();
                        rec.output_text = Some(reply.text.clone());
                        let call_id = self.store.record_call(&rec).ok();
                        self.receipt(&rec);
                        if use_cache {
                            let _ = self.store.cache_put(
                                &cache_key,
                                &CachedReply {
                                    provider: step.provider.clone(),
                                    model: step.model.clone(),
                                    tier: Some(step.tier.as_str().into()),
                                    text: reply.text.clone(),
                                    prompt_tokens: reply.prompt_tokens,
                                    completion_tokens: reply.completion_tokens,
                                    finish_reason: reply.finish_reason.clone(),
                                },
                            );
                        }
                        attempts.push(Attempt {
                            provider: step.provider.clone(),
                            model: step.model.clone(),
                            attempt,
                            ok: true,
                            error_kind: None,
                            error: None,
                            latency_ms,
                        });
                        return Ok(Routed {
                            reply,
                            route,
                            provider: step.provider.clone(),
                            model: step.model.clone(),
                            tier: step.tier,
                            cost_usd: cost,
                            call_id,
                            attempts,
                            cached: false,
                        });
                    }
                    Err(err) => {
                        rec.status = "ERROR".into();
                        rec.error_kind = Some(err.kind().into());
                        rec.error = Some(err.to_string());
                        if let LlmError::Truncated { completion_tokens } = err {
                            rec.completion_tokens = completion_tokens;
                        }
                        let _ = self.store.record_call(&rec);
                        self.receipt(&rec);
                        tracing::warn!("{} {} attempt {attempt}: {err}", step.provider, step.model);
                        attempts.push(Attempt {
                            provider: step.provider.clone(),
                            model: step.model.clone(),
                            attempt,
                            ok: false,
                            error_kind: Some(err.kind().into()),
                            error: Some(err.to_string()),
                            latency_ms,
                        });

                        let mut wait = None;
                        if let LlmError::RateLimited { retry_after, .. } = &err {
                            let cool = retry_after.unwrap_or(Duration::from_secs(60));
                            self.cooldowns.lock().unwrap().insert(
                                (step.provider.clone(), step.model.clone()),
                                Instant::now() + cool,
                            );
                            if s.retry.cascade_on_429 {
                                break;
                            }
                            wait = *retry_after;
                        }
                        if !err.retryable() || attempt == s.retry.attempts.max(1) {
                            break;
                        }
                        tokio::time::sleep(wait.unwrap_or_else(|| {
                            backoff(attempt, s.retry.base_seconds, s.retry.max_seconds)
                        }))
                        .await;
                    }
                }
            }
        }

        let message = if plan.iter().any(|p| p.eligible) {
            format!("route '{route}': every eligible rung failed")
        } else {
            format!(
                "route '{route}': no eligible rung ({})",
                plan.iter()
                    .map(|p| format!("{}: {}", p.provider, p.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        };
        Err(RouteFailure {
            message,
            plan,
            attempts,
        })
    }
}

impl Router {
    /// One JSON line per call in `LOGS/forge_receipts.jsonl`: never the prompt or the key.
    fn receipt(&self, rec: &CallRecord) {
        if self.store.path == ":memory:" {
            return;
        }
        let line = serde_json::json!({
            "task": rec.task, "provider": rec.provider, "model": rec.model, "route": rec.route, "rung": rec.rung,
            "attempt": rec.attempt, "status": rec.status, "error": rec.error, "finish_reason": rec.finish_reason,
            "prompt_tokens": rec.prompt_tokens, "completion_tokens": rec.completion_tokens, "cost_usd": rec.cost_usd,
            "elapsed_seconds": rec.latency_ms as f64 / 1000.0, "started_at": rec.started_at, "finished_at": rec.finished_at,
            "note": rec.doc_id,
        });
        let path = self.cfg.paths.receipts_path();
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(&self.cfg.paths.logs_dir)?;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            writeln!(f, "{line}")
        };
        if let Err(e) = write() {
            tracing::warn!("receipt not written to {}: {e}", path.display());
        }
    }
}

/// Same route (or pin), same messages, same mode and limits -> same key. Tier/budget filters are not
/// part of it: a cached answer costs nothing whichever rung produced it.
fn cache_key(route: &str, req: &RouteRequest, default_cap: u32, default_temp: f32) -> String {
    let v = serde_json::json!({
        "route": route,
        "pin": req.pin,
        "messages": req.messages,
        "json": req.json_mode,
        "max_tokens": req.max_tokens.unwrap_or(default_cap),
        "temperature": req.temperature.unwrap_or(default_temp),
    });
    hex::encode(Sha256::digest(crate::canon::canonical_json(&v).as_bytes()))
}

fn backoff(attempt: u32, base: f64, max: f64) -> Duration {
    let secs = (base * 2f64.powi(attempt as i32 - 1)).min(max);
    let jitter = 0.75 + fastrand::f64() * 0.5;
    Duration::from_secs_f64((secs * jitter).max(0.0))
}

/// `Some(true)` if `now` is inside `"HH:MM-HH:MM"` (UTC, may wrap midnight); `None` if unparsable.
pub fn in_window(spec: &str, now: chrono::DateTime<chrono::Utc>) -> Option<bool> {
    use chrono::Timelike;
    let parse = |s: &str| -> Option<u32> {
        let (h, m) = s.trim().split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (h < 24 && m < 60).then_some(h * 60 + m)
    };
    let (a, b) = spec.split_once('-')?;
    let (start, end) = (parse(a)?, parse(b)?);
    let t = now.hour() * 60 + now.minute();
    Some(if start <= end {
        t >= start && t < end
    } else {
        t >= start || t < end
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MockProvider;
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn router(settings: &str) -> Router {
        let providers = r#"{"a":{"base":"mock://","wire":"mock","no_key":true},
                            "b":{"base":"mock://","wire":"mock","no_key":true},
                            "c":{"base":"https://x","key":"FORGE_TEST_KEY_THAT_IS_NOT_SET"}}"#;
        let cfg = Config::from_strings(providers, settings, PathBuf::from("."), None).unwrap();
        Router::new(cfg, Store::open_memory().unwrap()).unwrap()
    }

    const LADDER: &str = r#"{"retry":{"attempts":2,"base_seconds":0,"max_seconds":0},
        "routes":{"default":[{"provider":"c","model":"x"},{"provider":"a","model":"free-1","tier":"free","daily_requests":1},
                             {"provider":"b","model":"paid-1","tier":"premium","price_in":1000000,"price_out":0}]}}"#;

    fn ask(text: &str) -> RouteRequest {
        RouteRequest {
            messages: vec![Message::user(text)],
            task: "test".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn cascades_past_missing_key_and_429_then_records_everything() {
        let mut r = router(LADDER);
        r.insert_provider(
            "a",
            Arc::new(MockProvider::scripted(
                "a",
                vec![Err(LlmError::RateLimited {
                    retry_after: None,
                    body: "slow down".into(),
                })],
            )),
        );
        let out = r.complete(ask("hi")).await.unwrap();
        assert_eq!(out.provider, "b");
        assert_eq!(out.attempts.len(), 2);
        assert!(out.cost_usd > 0.0);
        // "a" is now cooling down and over its daily quota; plan says why.
        let (_, plan) = r.plan(&ask("hi")).unwrap();
        assert!(plan[0].reason.contains("FORGE_TEST_KEY_THAT_IS_NOT_SET"));
        assert!(!plan[1].eligible);
        assert_eq!(r.store.recent_calls(10).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn retries_transient_errors_on_same_rung_and_never_returns_truncation() {
        let mut r = router(LADDER);
        r.insert_provider(
            "a",
            Arc::new(MockProvider::scripted(
                "a",
                vec![
                    Err(LlmError::Server {
                        status: 503,
                        body: String::new(),
                    }),
                    MockProvider::ok("fine"),
                ],
            )),
        );
        let out = r.complete(ask("hi")).await.unwrap();
        assert_eq!(
            (out.provider.as_str(), out.reply.text.as_str()),
            ("a", "fine")
        );

        let mut r = router(LADDER);
        r.insert_provider(
            "a",
            Arc::new(MockProvider::scripted(
                "a",
                vec![Err(LlmError::Truncated {
                    completion_tokens: 8000,
                })],
            )),
        );
        let out = r.complete(ask("hi")).await.unwrap();
        assert_eq!(
            out.provider, "b",
            "a truncated reply must cascade, never be returned"
        );
    }

    #[tokio::test]
    async fn budget_and_max_tier_keep_paid_rungs_out() {
        let r = router(&LADDER.replace("\"retry\"", "\"budget\":{\"daily_usd\":0},\"retry\""));
        let (_, plan) = r.plan(&ask("x")).unwrap();
        assert!(plan[2].reason.starts_with("daily budget reached"));
        let r = router(LADDER);
        let req = RouteRequest {
            max_tier: Some(Tier::Free),
            ..ask("x")
        };
        let (_, plan) = r.plan(&req).unwrap();
        assert!(plan[2].reason.contains("above max"));
    }

    #[tokio::test]
    async fn exhausted_route_reports_every_reason() {
        let mut r = router(LADDER);
        r.insert_provider(
            "a",
            Arc::new(MockProvider::scripted(
                "a",
                vec![Err(LlmError::Auth {
                    status: 401,
                    body: String::new(),
                })],
            )),
        );
        r.insert_provider(
            "b",
            Arc::new(MockProvider::scripted(
                "b",
                vec![Err(LlmError::BadRequest {
                    status: 400,
                    body: String::new(),
                })],
            )),
        );
        let err = r.complete(ask("x")).await.unwrap_err();
        assert_eq!(err.attempts.len(), 2);
        assert!(err.message.contains("every eligible rung failed"));
    }

    #[tokio::test]
    async fn identical_requests_are_served_from_cache_unless_no_cache() {
        let r =
            router(r#"{"routes":{"default":[{"provider":"a","model":"m","price_in":1000000}]}}"#);
        let first = r.complete(ask("same question")).await.unwrap();
        assert!(!first.cached && first.cost_usd > 0.0);
        let second = r.complete(ask("same question")).await.unwrap();
        assert!(second.cached);
        assert_eq!(
            (second.reply.text.as_str(), second.cost_usd),
            (first.reply.text.as_str(), 0.0)
        );
        let fresh = r
            .complete(RouteRequest {
                no_cache: true,
                ..ask("same question")
            })
            .await
            .unwrap();
        assert!(!fresh.cached);
        // Cache hits don't use up a free tier's daily quota.
        assert_eq!(r.store.requests_today("a", "m").unwrap(), 2);
    }

    #[tokio::test]
    async fn focus_rides_every_call_and_a_focus_change_misses_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let providers = r#"{"a":{"base":"mock://","wire":"mock","no_key":true}}"#;
        let settings = r#"{"routes":{"default":[{"provider":"a","model":"m"}]}}"#;
        let cfg =
            Config::from_strings(providers, settings, dir.path().to_path_buf(), None).unwrap();
        let r = Router::new(cfg, Store::open_memory().unwrap()).unwrap();
        let focus = dir.path().join("GLOBAL_FOCUS.md");
        let sha_of = |r: &Router| -> Option<String> {
            r.store
                .lock()
                .query_row(
                    "SELECT focus_sha256 FROM calls ORDER BY id DESC LIMIT 1",
                    [],
                    |x| x.get(0),
                )
                .unwrap()
        };

        assert!(!r.complete(ask("q")).await.unwrap().cached);
        assert_eq!(sha_of(&r), None, "no focus file: nothing recorded");

        std::fs::write(&focus, "Focus: Law 4 only.").unwrap();
        let one = r.complete(ask("q")).await.unwrap();
        assert!(
            !one.cached,
            "adding a focus must not reuse the focus-less reply"
        );
        let sha1 = sha_of(&r).unwrap();
        assert!(
            r.complete(ask("q")).await.unwrap().cached,
            "same focus, same question: cached"
        );
        assert_eq!(sha_of(&r).unwrap(), sha1, "cache hits record the focus too");

        std::fs::write(&focus, "Focus: Law 9 only.").unwrap();
        assert!(
            !r.complete(ask("q")).await.unwrap().cached,
            "changed focus must miss the cache"
        );
        assert_ne!(sha_of(&r).unwrap(), sha1);

        let off = r
            .complete(RouteRequest {
                no_focus: true,
                ..ask("q")
            })
            .await
            .unwrap();
        assert!(
            off.cached,
            "--no-focus is the same request as the focus-less first one"
        );
        assert_eq!(sha_of(&r).as_deref(), Some("off"));
    }

    #[test]
    fn windows_wrap_midnight() {
        let at = |h, m| chrono::Utc.with_ymd_and_hms(2026, 1, 1, h, m, 0).unwrap();
        assert_eq!(in_window("16:30-00:30", at(23, 0)), Some(true));
        assert_eq!(in_window("16:30-00:30", at(0, 15)), Some(true));
        assert_eq!(in_window("16:30-00:30", at(12, 0)), Some(false));
        assert_eq!(in_window("09:00-17:00", at(9, 0)), Some(true));
        assert_eq!(in_window("nonsense", at(9, 0)), None);
    }
}
