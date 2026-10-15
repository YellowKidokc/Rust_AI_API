//! MCP server over stdio (newline-delimited JSON-RPC 2.0). Lets another AI (Claude Desktop, Claude Code,
//! Cursor, any MCP client) use the same router, ledger, context library, templates, pipeline and candidate
//! graph as the CLI and the HTTP gateway. Logs go to stderr only; stdout carries protocol messages.
//!
//! Register it, e.g. Claude Code:   claude mcp add forge -- C:\path\to\forge.exe mcp
//! Claude Desktop (claude_desktop_config.json):
//!   "mcpServers": { "forge": { "command": "C:\\path\\to\\forge.exe", "args": ["mcp"] } }

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use forge_core::canon::{self, CandidateInput};
use forge_core::pipeline::{self, RunOptions};
use forge_core::{template, Forge, Message, RouteRequest, Tier};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "forge is David Lowe's local AI workbench: one router over DeepSeek, Anthropic, OpenRouter, \
Gemini, Ollama and others, with a SQLite ledger, a searchable context library (axioms, definitions, prior work), \
Markdown job templates, an INBOX->OUTBOX pipeline, and a candidate graph where staged work waits for a human ruling. \
Start every session with forge_brief: it carries the project's concepts and rules. Before writing about the framework, \
pull the relevant canon with context_search instead of guessing. Staging a candidate is never admission to canon.";

pub async fn run(forge: Arc<Forge>) -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(msg) = rx.recv().await {
            let mut line = msg.to_string();
            line.push('\n');
            if out.write_all(line.as_bytes()).await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    eprintln!("forge mcp: ready on stdio (db: {})", forge.store.path);
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}));
                continue;
            }
        };
        let forge = forge.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Some(resp) = handle(&forge, msg).await {
                let _ = tx.send(resp);
            }
        });
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}

/// Handle one JSON-RPC message. Notifications (no `id`) get no reply.
pub async fn handle(forge: &Arc<Forge>, msg: Value) -> Option<Value> {
    if msg.is_array() {
        return Some(
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600, "message": "batches are not supported"}}),
        );
    }
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let id = id?;

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let version = if PROTOCOL_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSIONS[0]
            };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "forge", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call(forge, &name, &args).await {
                Ok(v) => {
                    let text = if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        serde_json::to_string_pretty(&v).unwrap_or_default()
                    };
                    let mut r =
                        json!({"content": [{"type": "text", "text": text}], "isError": false});
                    if v.is_object() {
                        r["structuredContent"] = v;
                    }
                    r
                }
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    })
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

fn tools() -> Vec<Value> {
    vec![
        json!({"name": "forge_brief", "description": "Read first. The project brief (concepts, rules, vocabulary), the configured routes, which providers are ready, the templates, and how much is in the context library.",
               "inputSchema": obj(json!({}), &[])}),
        json!({"name": "forge_ask", "description": "Send a prompt through the router (DeepSeek by default; route 'free' for free tiers, 'premium' for Claude Opus, 'local' for Ollama). Optionally prepends the most relevant context-library snippets. Every call is logged with tokens and cost.",
               "inputSchema": obj(json!({
                   "prompt": {"type": "string"},
                   "system": {"type": "string"},
                   "route": {"type": "string", "description": "Route name from settings.json; omit for the default."},
                   "json": {"type": "boolean", "description": "Ask for a JSON reply."},
                   "use_context": {"type": "boolean", "description": "Prepend top context-library matches for the prompt."},
                   "context_query": {"type": "string", "description": "Search terms for the context (default: the prompt)."},
                   "max_tokens": {"type": "integer"},
                   "max_tier": {"type": "string", "enum": ["local", "free", "discount", "standard", "premium"], "description": "Never use a rung above this tier."},
                   "no_cache": {"type": "boolean", "description": "Call a model even if this exact request was answered before."}
               }), &["prompt"])}),
        json!({"name": "forge_route_plan", "description": "Show which provider/model rungs a route would try right now, in order, and why any are skipped (missing key, quota used, cooling down after 429, budget reached, outside time window).",
               "inputSchema": obj(json!({"route": {"type": "string"}, "prompt": {"type": "string", "description": "Optional, for the context-window check."}}), &[])}),
        json!({"name": "context_search", "description": "Full-text search (BM25) over the context library: axioms, laws, definitions, prior results. Use before writing about the framework.",
               "inputSchema": obj(json!({"query": {"type": "string"}, "limit": {"type": "integer"}, "kind": {"type": "string", "description": "Only this kind (e.g. axiom, law, definition)."}}), &["query"])}),
        json!({"name": "context_add", "description": "Add or replace one snippet in the context library (same key replaces).",
               "inputSchema": obj(json!({"key": {"type": "string"}, "title": {"type": "string"}, "body": {"type": "string"}, "kind": {"type": "string"}, "source": {"type": "string"}, "tags": {"type": "string"}}), &["key", "title", "body"])}),
        json!({"name": "doc_state", "description": "A document's pipeline state: last job, status, source hash, last output, notes, recent events, and derivatives (which outputs came from which version of the text).",
               "inputSchema": obj(json!({"doc_id": {"type": "string", "description": "File name, e.g. 'Law 4.md'."}}), &["doc_id"])}),
        json!({"name": "doc_note", "description": "Attach a note to a document's state (carried into its history).",
               "inputSchema": obj(json!({"doc_id": {"type": "string"}, "note": {"type": "string"}}), &["doc_id", "note"])}),
        json!({"name": "pipeline_run", "description": "Run a job template over a file or folder (INBOX style) and write OUTBOX results. Unchanged documents are skipped; redo also bypasses the response cache. A note's front matter can pick its route (route:, or priority/compute/tags words such as draft, deep, private). More than 10 documents requires confirm=true. Use dry_run to see the exact prompt without calling a model.",
               "inputSchema": obj(json!({
                   "path": {"type": "string"}, "job": {"type": "string", "description": "Template name (see forge_brief)."},
                   "route": {"type": "string"}, "outbox": {"type": "string"}, "redo": {"type": "boolean"},
                   "dry_run": {"type": "boolean"}, "confirm": {"type": "boolean"}, "workers": {"type": "integer"}
               }), &["path"])}),
        json!({"name": "template_show", "description": "Show a job template's front matter and body.",
               "inputSchema": obj(json!({"name": {"type": "string"}}), &["name"])}),
        json!({"name": "canon_stage", "description": "Stage a candidate (claim, axiom, reconciliation proposal) into the candidate graph. Never admission. Extra fields belong inside `atom`; identical content returns the existing record.",
               "inputSchema": obj(json!({
                   "text": {"type": "string"}, "atom": {"type": "object"}, "atom_id": {"type": "string"},
                   "schema_version": {"type": "string"}, "source_uri": {"type": "string"}, "source_span": {},
                   "author": {"type": "string"}, "depends_on": {"type": "array", "items": {"type": "string"}}, "note": {"type": "string"}
               }), &[])}),
        json!({"name": "canon_search", "description": "Find staged candidates by atom id, candidate id, or text.",
               "inputSchema": obj(json!({"q": {"type": "string"}, "limit": {"type": "integer"}}), &["q"])}),
        json!({"name": "canon_agenda", "description": "What waits on a human ruling: open conflicts and candidates whose checks raised reasons.",
               "inputSchema": obj(json!({"limit": {"type": "integer"}}), &[])}),
        json!({"name": "ledger", "description": "Recent calls (provider, model, status, tokens, cost) and per-model totals.",
               "inputSchema": obj(json!({"limit": {"type": "integer"}, "days": {"type": "integer"}}), &[])}),
    ]
}

fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn n(a: &Value, k: &str) -> Option<u64> {
    a.get(k).and_then(Value::as_u64)
}

fn b(a: &Value, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}

async fn call(forge: &Arc<Forge>, name: &str, a: &Value) -> Result<Value, String> {
    let e = |x: anyhow::Error| format!("{x:#}");
    match name {
        "forge_brief" => {
            let (route, steps) = forge
                .router
                .plan(&RouteRequest::default())
                .unwrap_or_default();
            Ok(json!({
                "brief": forge.brief().unwrap_or_else(|| "No BRIEF.md yet. Put one next to the config folder, or context_add with key 'brief'.".into()),
                "default_route": route,
                "default_route_plan": steps,
                "routes": forge.router.route_names(),
                "providers_ready": forge.router.available_providers(),
                "providers_unavailable": forge.router.unavailable_providers(),
                "templates": template::list(&forge.config.paths.templates_dir),
                "context_entries": forge.store.context_count().unwrap_or(0),
            }))
        }
        "forge_ask" => {
            let prompt = s(a, "prompt").ok_or("prompt is required")?.to_string();
            let mut messages = Vec::new();
            if let Some(sys) = s(a, "system") {
                messages.push(Message::system(sys));
            }
            let mut used = vec![];
            let mut user = prompt.clone();
            if b(a, "use_context") {
                let q = s(a, "context_query").unwrap_or(&prompt);
                let hits = forge
                    .store
                    .context_search(q, forge.config.settings.context.top_k, None)
                    .map_err(e)?;
                if !hits.is_empty() {
                    used = hits.iter().map(|h| h.key.clone()).collect();
                    user = format!(
                        "Background (from the context library):\n{}\n{}",
                        pipeline::render_context(&hits, forge.config.settings.context.max_chars),
                        prompt
                    );
                }
            }
            messages.push(Message::user(user));
            let max_tier = match s(a, "max_tier") {
                Some(t) => Some(
                    serde_json::from_value::<Tier>(json!(t))
                        .map_err(|_| format!("unknown tier {t}"))?,
                ),
                None => None,
            };
            let r = forge
                .router
                .complete(RouteRequest {
                    route: s(a, "route").map(str::to_string),
                    messages,
                    json_mode: b(a, "json"),
                    max_tokens: n(a, "max_tokens").map(|x| x as u32),
                    max_tier,
                    no_cache: b(a, "no_cache"),
                    task: "mcp:ask".into(),
                    ..Default::default()
                })
                .await
                .map_err(|f| {
                    serde_json::to_string_pretty(
                        &json!({"error": f.message, "plan": f.plan, "attempts": f.attempts}),
                    )
                    .unwrap_or(f.message)
                })?;
            Ok(json!({
                "text": r.reply.text, "route": r.route, "provider": r.provider, "model": r.model, "tier": r.tier,
                "tokens_in": r.reply.prompt_tokens, "tokens_out": r.reply.completion_tokens, "cost_usd": r.cost_usd,
                "call_id": r.call_id, "cached": r.cached, "context_used": used, "attempts": r.attempts,
            }))
        }
        "forge_route_plan" => {
            let req = RouteRequest {
                route: s(a, "route").map(str::to_string),
                messages: vec![Message::user(s(a, "prompt").unwrap_or(""))],
                ..Default::default()
            };
            let (route, steps) = forge.router.plan(&req)?;
            Ok(
                json!({"route": route, "steps": steps, "spend_today_usd": forge.store.spend_today().unwrap_or(0.0), "budget_daily_usd": forge.config.settings.budget.daily_usd}),
            )
        }
        "context_search" => {
            let hits = forge
                .store
                .context_search(
                    s(a, "query").ok_or("query is required")?,
                    n(a, "limit").unwrap_or(5).min(50) as usize,
                    s(a, "kind"),
                )
                .map_err(e)?;
            Ok(json!({"hits": hits}))
        }
        "context_add" => {
            let key = s(a, "key").ok_or("key is required")?;
            forge
                .store
                .context_put(
                    key,
                    s(a, "kind").unwrap_or("note"),
                    s(a, "title").ok_or("title is required")?,
                    s(a, "body").ok_or("body is required")?,
                    s(a, "source"),
                    s(a, "tags"),
                )
                .map_err(e)?;
            Ok(json!({"ok": true, "key": key}))
        }
        "doc_state" => {
            let id = s(a, "doc_id").ok_or("doc_id is required")?;
            Ok(json!({"state": forge.store.doc_get(id).map_err(e)?,
                "events": forge.store.doc_events(id, 20).map_err(e)?,
                "derivatives": forge.store.derivatives(id, 20).map_err(e)?,
            }))
        }
        "doc_note" => {
            forge
                .store
                .doc_note(
                    s(a, "doc_id").ok_or("doc_id is required")?,
                    s(a, "note").ok_or("note is required")?,
                )
                .map_err(e)?;
            Ok(json!({"ok": true}))
        }
        "pipeline_run" => {
            let path = PathBuf::from(s(a, "path").ok_or("path is required")?);
            let docs = pipeline::collect(&path).map_err(e)?;
            if docs.is_empty() {
                return Err(format!("no .md/.txt documents at {}", path.display()));
            }
            let dry = b(a, "dry_run");
            if docs.len() > 10 && !b(a, "confirm") && !dry {
                return Err(format!(
                    "{} documents found. More than 10 needs confirm=true (or dry_run=true first).",
                    docs.len()
                ));
            }
            let outbox = s(a, "outbox")
                .map(PathBuf::from)
                .unwrap_or_else(|| default_outbox(&path));
            let opts = RunOptions {
                job: s(a, "job").unwrap_or("default").into(),
                outbox,
                route: s(a, "route").map(str::to_string),
                redo: b(a, "redo"),
                dry_run: dry,
                workers: n(a, "workers").unwrap_or(4).clamp(1, 60) as usize,
            };
            let results = forge
                .run_many(docs, opts, Arc::new(AtomicBool::new(false)), |_| {})
                .await;
            Ok(json!({"results": results}))
        }
        "template_show" => {
            let t = template::load(
                &forge.config.paths.templates_dir,
                s(a, "name").ok_or("name is required")?,
            )
            .map_err(e)?;
            Ok(json!(t))
        }
        "canon_stage" => {
            let input: CandidateInput = serde_json::from_value(a.clone())
                .map_err(|x| format!("candidate rejected: {x}. Put extra fields inside `atom`."))?;
            let r = canon::stage(&forge.store, input).map_err(|x| x.to_string())?;
            Ok(json!(r))
        }
        "canon_search" => Ok(
            json!({"results": canon::search(&forge.store, s(a, "q").ok_or("q is required")?, n(a, "limit").unwrap_or(10).min(200) as u32).map_err(|x| x.to_string())?}),
        ),
        "canon_agenda" => Ok(
            json!({"items": canon::agenda(&forge.store, n(a, "limit").unwrap_or(50).min(500) as u32).map_err(|x| x.to_string())?}),
        ),
        "ledger" => Ok(json!({
            "recent": forge.store.recent_calls(n(a, "limit").unwrap_or(20).min(500) as u32).map_err(e)?,
            "by_model": forge.store.stats(n(a, "days").unwrap_or(1) as u32).map_err(e)?,
            "spend_today_usd": forge.store.spend_today().map_err(e)?,
        })),
        other => Err(format!("unknown tool: {other}")),
    }
}

/// `<folder>/../OUTBOX` when running on an INBOX folder, else `<folder>/OUTBOX`.
pub fn default_outbox(path: &std::path::Path) -> PathBuf {
    let dir = if path.is_file() {
        path.parent().unwrap_or(path).to_path_buf()
    } else {
        path.to_path_buf()
    };
    let dir = if dir.file_name().is_some_and(|n| n == "00_PRIORITY") {
        dir.parent().unwrap_or(&dir).to_path_buf()
    } else {
        dir
    };
    if dir
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("INBOX"))
    {
        dir.parent().unwrap_or(&dir).join("OUTBOX")
    } else {
        dir.join("OUTBOX")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::config::{Config, DEFAULT_PROVIDERS, DEFAULT_SETTINGS};

    fn forge(base: PathBuf) -> Arc<Forge> {
        let mut cfg =
            Config::from_strings(DEFAULT_PROVIDERS, DEFAULT_SETTINGS, base, None).unwrap();
        cfg.settings.default_route = Some("test".into());
        Forge::with_store(cfg, forge_core::Store::open_memory().unwrap()).unwrap()
    }

    async fn rpc(f: &Arc<Forge>, id: i64, method: &str, params: Value) -> Value {
        handle(
            f,
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn handshake_lists_tools_and_ignores_notifications() {
        let f = forge(PathBuf::from("."));
        let init = rpc(
            &f,
            1,
            "initialize",
            json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}),
        )
        .await;
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(
            &f,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .await
        .is_none());
        let list = rpc(&f, 2, "tools/list", json!({})).await;
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"forge_ask") && names.contains(&"canon_stage"));
        assert_eq!(rpc(&f, 3, "nope", json!({})).await["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn ask_with_context_and_ledger() {
        let f = forge(PathBuf::from("."));
        let add = rpc(&f, 1, "tools/call", json!({"name": "context_add", "arguments": {"key": "law9", "title": "Law 9 Moral Conservation", "body": "Weak force breaks parity; Noether conserves moral energy", "kind": "law"}})).await;
        assert_eq!(add["result"]["isError"], false);
        let ask = rpc(
            &f,
            2,
            "tools/call",
            json!({"name": "forge_ask", "arguments": {"prompt": "Explain Noether and parity", "use_context": true}}),
        )
        .await;
        let sc = &ask["result"]["structuredContent"];
        assert_eq!(sc["provider"], "mock", "{ask}");
        assert_eq!(sc["context_used"][0], "law9");
        let led = rpc(
            &f,
            3,
            "tools/call",
            json!({"name": "ledger", "arguments": {}}),
        )
        .await;
        assert_eq!(
            led["result"]["structuredContent"]["recent"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let bad = rpc(
            &f,
            4,
            "tools/call",
            json!({"name": "canon_stage", "arguments": {"atom": {}, "stray": 1}}),
        )
        .await;
        assert_eq!(bad["result"]["isError"], true);
    }

    #[tokio::test]
    async fn pipeline_runs_inbox_to_outbox_and_skips_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("INBOX");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("note one.md"), "# Grace\nGrace as a force term.").unwrap();
        let f = forge(tmp.path().to_path_buf());
        let args = json!({"name": "pipeline_run", "arguments": {"path": inbox.display().to_string(), "job": "summarize"}});
        let r = rpc(&f, 1, "tools/call", args.clone()).await;
        let res = &r["result"]["structuredContent"]["results"][0];
        assert_eq!(res["status"], "DONE", "{r}");
        let out = tmp.path().join("OUTBOX").join("note one · SUMMARY.md");
        assert!(std::fs::read_to_string(&out)
            .unwrap()
            .contains("provider: mock"));
        let again = rpc(&f, 2, "tools/call", args).await;
        assert_eq!(
            again["result"]["structuredContent"]["results"][0]["status"],
            "SKIPPED"
        );
        let redo = rpc(
            &f,
            3,
            "tools/call",
            json!({"name": "pipeline_run", "arguments": {"path": inbox.display().to_string(), "job": "summarize", "redo": true}}),
        )
        .await;
        assert_eq!(
            redo["result"]["structuredContent"]["results"][0]["status"],
            "DONE"
        );
        let older = tmp.path().join("OUTBOX").join("_older");
        assert!(
            older.is_dir(),
            "replaced output must be shelved, not deleted"
        );
        let state = rpc(
            &f,
            4,
            "tools/call",
            json!({"name": "doc_state", "arguments": {"doc_id": "note one.md"}}),
        )
        .await;
        assert_eq!(
            state["result"]["structuredContent"]["derivatives"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn note_front_matter_picks_the_route() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("hinted.md"),
            "---\npriority: draft\n---\nA draft.",
        )
        .unwrap();
        let mut cfg = Config::from_strings(
            DEFAULT_PROVIDERS,
            DEFAULT_SETTINGS,
            tmp.path().to_path_buf(),
            None,
        )
        .unwrap();
        cfg.settings.hints = vec![forge_core::config::RouteHint {
            when: "draft".into(),
            route: "test".into(),
        }];
        // The default route has no usable rung here (no keys), so only the hint can make this succeed.
        let f = Forge::with_store(cfg, forge_core::Store::open_memory().unwrap()).unwrap();
        let path = tmp.path().join("hinted.md").display().to_string();
        let r = rpc(
            &f,
            1,
            "tools/call",
            json!({"name": "pipeline_run", "arguments": {"path": path}}),
        )
        .await;
        let res = &r["result"]["structuredContent"]["results"][0];
        assert_eq!(res["status"], "DONE", "{r}");
        assert_eq!(res["route"], "test (note marked 'draft')");
    }
}
