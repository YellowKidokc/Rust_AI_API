//! INBOX -> OUTBOX. For each document: hash it, skip it if this exact source already went through this
//! exact job, pull the most relevant context snippets from the library, render the job template, send it
//! through the router, write `OUTBOX/<name> · <SUFFIX>.md` (the previous one moves to `_older/<date>/`,
//! never deleted), and record the state in SQLite. Each document is finished before its slot is reused.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::hints;
use crate::router::RouteRequest;
use crate::store::{now, ContextHit};
use crate::template::{self, Template};
use crate::types::Message;
use crate::Forge;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub job: String,
    pub outbox: PathBuf,
    pub route: Option<String>,
    pub redo: bool,
    pub dry_run: bool,
    /// Max documents in flight. The shared limiter still caps API calls.
    pub workers: usize,
}

/// A document's prompt, ready to send.
pub struct Prepared {
    pub template: Template,
    pub messages: Vec<Message>,
    /// SHA-256 of source text + template: decides whether the document needs another run.
    pub source_hash: String,
    /// SHA-256 of the source text alone: the parent side of the lineage record.
    pub text_sha256: String,
    /// What the note's own front matter asked for (route, reason).
    pub note_route: Option<(String, String)>,
    /// The paper's permanent id, if it already has one (`forge_uid:`).
    pub uid: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocResult {
    pub doc_id: String,
    /// Permanent paper id (empty only if the file couldn't be read).
    pub uid: String,
    pub path: String,
    pub status: String, // DONE | SKIPPED | FAILED | DRY_RUN | STOPPED
    pub output: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cost_usd: f64,
    pub tokens: u32,
    /// Route used, and why it was chosen.
    pub route: Option<String>,
    pub detail: Option<String>,
}

/// Documents a folder run would process: `00_PRIORITY/` first, then the folder itself (not recursive).
pub fn collect(folder: &Path) -> Result<Vec<PathBuf>> {
    if folder.is_file() {
        return Ok(vec![folder.to_path_buf()]);
    }
    let mut out = Vec::new();
    for dir in [folder.join("00_PRIORITY"), folder.to_path_buf()] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut batch: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("md") || x.eq_ignore_ascii_case("txt"))
            })
            .filter(|p| {
                !p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('_') || n.starts_with('.'))
            })
            .collect();
        batch.sort();
        out.extend(batch);
    }
    Ok(out)
}

/// Stable id: the file name. Two files with the same name in different folders share history on purpose
/// (a note moved from INBOX to 00_PRIORITY is still the same note).
pub fn doc_id(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed")
        .to_string()
}

fn source_hash(text: &str, tpl: &Template) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    h.update(b"\0");
    h.update(tpl.name.as_bytes());
    h.update(b"\0");
    h.update(tpl.body.as_bytes());
    hex::encode(h.finalize())
}

/// Most significant terms of a document, used as the context-library query.
pub fn key_terms(text: &str, n: usize) -> String {
    const STOP: &[&str] = &[
        "this", "that", "with", "from", "have", "were", "which", "their", "there", "these",
        "those", "would", "could", "should", "about", "into", "than", "then", "them", "they",
        "what", "when", "where", "while", "will", "shall", "been", "being", "does", "also", "only",
        "more", "most", "some", "such", "each", "other", "because", "between",
    ];
    let mut freq: std::collections::HashMap<String, usize> = Default::default();
    for w in text.split(|c: char| !c.is_alphanumeric()) {
        if w.chars().count() < 4 {
            continue;
        }
        let w = w.to_lowercase();
        if STOP.contains(&w.as_str()) {
            continue;
        }
        *freq.entry(w).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = freq.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter()
        .take(n)
        .map(|(w, _)| w)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render hits as a compact block, capped at `max_chars`.
pub fn render_context(hits: &[ContextHit], max_chars: usize) -> String {
    let mut out = String::new();
    for h in hits {
        let piece = format!(
            "- [{}] {}: {}\n",
            h.kind,
            h.title,
            h.body.trim().replace('\n', " ")
        );
        if out.len() + piece.len() > max_chars {
            let room = max_chars.saturating_sub(out.len());
            if room > 80 {
                out.extend(piece.chars().take(room - 2));
                out.push_str("…\n");
            }
            break;
        }
        out.push_str(&piece);
    }
    out
}

fn outbox_name(path: &Path, suffix: &str) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("doc");
    format!("{stem} · {suffix}.md")
}

/// Move an existing output to `_older/<date>/` before it is replaced. Never deletes.
fn shelve(target: &Path, outbox: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let day = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dir = outbox.join("_older").join(day);
    std::fs::create_dir_all(&dir)?;
    let name = target.file_name().unwrap();
    let mut dest = dir.join(name);
    if dest.exists() {
        let stamp = chrono::Local::now().format("%H%M%S%.3f").to_string();
        dest = dir.join(format!("{stamp} {}", name.to_string_lossy()));
    }
    std::fs::rename(target, &dest)
        .or_else(|_| std::fs::copy(target, &dest).and_then(|_| std::fs::remove_file(target)))?;
    Ok(())
}

impl Forge {
    /// Build the prompt for one document.
    pub fn prepare(&self, path: &Path, job: &str) -> Result<Prepared> {
        let tpl = template::load(&self.config.paths.templates_dir, job)?;
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let text = raw.strip_prefix('\u{feff}').unwrap_or(&raw).to_string();
        let id = doc_id(path);
        let k = tpl.context_k.unwrap_or(self.config.settings.context.top_k);
        let hits = if k > 0 {
            self.store.context_search(&key_terms(&text, 24), k, None)?
        } else {
            vec![]
        };
        let context = render_context(&hits, self.config.settings.context.max_chars);
        let history = self
            .store
            .doc_get(&id)?
            .filter(|d| d.job.as_deref() == Some(job))
            .and_then(|d| d.last_output)
            .map(|o| o.chars().take(4000).collect::<String>())
            .unwrap_or_default();
        let user = tpl.render(serde_json::json!({
            "document": text,
            "name": path.file_stem().and_then(|s| s.to_str()).unwrap_or(""),
            "path": path.display().to_string(),
            "context": context,
            "history": history,
            "date": chrono::Local::now().format("%Y-%m-%d").to_string(),
        }))?;
        let mut messages = Vec::new();
        if let Some(sys) = &tpl.system {
            messages.push(Message::system(sys.clone()));
        }
        messages.push(Message::user(user));
        let note_route = hints::resolve(&hints::read(&text), &self.config.settings.hints);
        // Hashes ignore the `forge_uid:` line: stamping the id must not make a paper look edited.
        let hashed = crate::uid::strip(&text);
        Ok(Prepared {
            uid: crate::uid::read(&text),
            source_hash: source_hash(&hashed, &tpl),
            text_sha256: hex::encode(Sha256::digest(hashed.as_bytes())),
            template: tpl,
            messages,
            note_route,
        })
    }

    pub async fn run_one(&self, path: &Path, opts: &RunOptions) -> DocResult {
        let id = doc_id(path);
        let mut res = DocResult {
            doc_id: id.clone(),
            uid: String::new(),
            path: path.display().to_string(),
            status: "FAILED".into(),
            output: None,
            provider: None,
            model: None,
            cost_usd: 0.0,
            tokens: 0,
            route: None,
            detail: None,
        };
        let Prepared {
            template: tpl,
            messages,
            source_hash: hash,
            text_sha256,
            note_route,
            uid: _,
        } = match crate::uid::ensure(&self.store, path, !opts.dry_run).and_then(|u| {
            res.uid = u.uid;
            self.prepare(path, &opts.job)
        }) {
            Ok(x) => x,
            Err(e) => {
                res.detail = Some(format!("{e:#}"));
                return res;
            }
        };
        let uid = res.uid.clone();
        if !opts.redo {
            let legacy = self.store.doc_get(&id).ok().flatten().is_some_and(|d| {
                d.status == "DONE"
                    && d.source_hash.as_deref() == Some(&hash)
                    && d.job.as_deref() == Some(&opts.job)
            });
            {
                if legacy
                    || self
                        .store
                        .doc_job_done(&id, &opts.job, &hash)
                        .unwrap_or(false)
                {
                    res.status = "SKIPPED".into();
                    res.detail = Some("unchanged since last run (use --redo to run again)".into());
                    return res;
                }
            }
        }
        // --route > the note's own front matter > the template > default_route.
        let (route, why) = match (&opts.route, &note_route, &tpl.route) {
            (Some(r), _, _) => (Some(r.clone()), "--route".to_string()),
            (None, Some((r, why)), _) => (Some(r.clone()), why.clone()),
            (None, None, Some(r)) => (Some(r.clone()), format!("template {}", tpl.name)),
            (None, None, None) => (None, "default_route".to_string()),
        };
        res.route = Some(format!(
            "{} ({why})",
            route
                .clone()
                .unwrap_or_else(|| self.config.settings.default_route_name())
        ));
        if opts.dry_run {
            res.status = "DRY_RUN".into();
            res.detail = Some(
                messages
                    .iter()
                    .map(|m| format!("[{}]\n{}", m.role, m.content))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
            return res;
        }
        let req = RouteRequest {
            route,
            messages,
            json_mode: tpl.json,
            task: format!("run:{}", opts.job),
            doc_id: Some(id.clone()),
            uid: Some(uid.clone()),
            no_cache: opts.redo,
            ..Default::default()
        };
        let path_s = path.display().to_string();
        match self.router.complete(req).await {
            Ok(r) => {
                let mut body = r.reply.text.clone();
                if tpl.json {
                    match crate::parse::extract_json(&body) {
                        Some(v) => {
                            body = format!(
                                "```json\n{}\n```\n",
                                serde_json::to_string_pretty(&v).unwrap_or(body)
                            )
                        }
                        None => {
                            let _ = self.store.doc_upsert(
                                &id, &path_s, &opts.job, &hash, "FAILED", r.call_id, None,
                            );
                            res.detail = Some(
                                "job expects JSON and the reply had none; nothing written".into(),
                            );
                            return res;
                        }
                    }
                }
                let out_path = opts.outbox.join(outbox_name(path, &tpl.suffix));
                let out_sha = hex::encode(Sha256::digest(body.as_bytes()));
                let doc = format!(
                    "---\nsource: '{}'\nsource_uid: {}\njob: {}\nroute: {}\nprovider: {}\nmodel: {}\ntokens_in: {}\ntokens_out: {}\ncost_usd: {:.6}\ncall_id: {}\ncreated: {}\n---\n\n{}\n",
                    path_s.replace('\'', "''"), uid, opts.job, r.route, r.provider, r.model, r.reply.prompt_tokens,
                    r.reply.completion_tokens, r.cost_usd, r.call_id.unwrap_or(0), now(), body.trim_end()
                );
                let write = std::fs::create_dir_all(&opts.outbox)
                    .map_err(anyhow::Error::from)
                    .and_then(|_| shelve(&out_path, &opts.outbox))
                    .and_then(|_| std::fs::write(&out_path, doc).map_err(anyhow::Error::from));
                if let Err(e) = write {
                    let _ = self.store.doc_upsert(
                        &id,
                        &path_s,
                        &opts.job,
                        &hash,
                        "FAILED",
                        r.call_id,
                        Some(&r.reply.text),
                    );
                    res.detail = Some(format!(
                        "model answered but writing the OUTBOX file failed: {e:#}"
                    ));
                    return res;
                }
                let _ = self.store.doc_upsert(
                    &id,
                    &path_s,
                    &opts.job,
                    &hash,
                    "DONE",
                    r.call_id,
                    Some(&r.reply.text),
                );
                let _ = self.store.derivative_add(
                    &id,
                    &text_sha256,
                    &opts.job,
                    &out_path.display().to_string(),
                    &out_sha,
                    r.call_id,
                );
                let _ = self.store.tag_uid(&id, &uid);
                res.status = "DONE".into();
                res.output = Some(out_path.display().to_string());
                res.provider = Some(r.provider);
                res.model = Some(r.model);
                res.cost_usd = r.cost_usd;
                res.tokens = r.reply.prompt_tokens + r.reply.completion_tokens;
                res
            }
            Err(f) => {
                let _ = self
                    .store
                    .doc_upsert(&id, &path_s, &opts.job, &hash, "FAILED", None, None);
                let _ = self.store.tag_uid(&id, &uid);
                res.detail = Some(format!(
                    "{}{}",
                    f.message,
                    f.attempts
                        .last()
                        .and_then(|a| a.error.as_ref())
                        .map(|e| format!(" (last error: {e})"))
                        .unwrap_or_default()
                ));
                res
            }
        }
    }

    /// Run many documents. `stop` is checked before each document starts; in-flight ones finish.
    pub async fn run_many(
        self: &Arc<Self>,
        paths: Vec<PathBuf>,
        opts: RunOptions,
        stop: Arc<AtomicBool>,
        on_done: impl Fn(&DocResult) + Send + Sync + 'static,
    ) -> Vec<DocResult> {
        let on_done = Arc::new(on_done);
        let sem = Arc::new(tokio::sync::Semaphore::new(opts.workers.max(1)));
        let mut set = tokio::task::JoinSet::new();
        let mut results = Vec::new();
        for path in paths {
            let permit = sem.clone().acquire_owned().await.expect("semaphore closed");
            if stop.load(Ordering::SeqCst) {
                results.push(DocResult {
                    doc_id: doc_id(&path),
                    uid: String::new(),
                    path: path.display().to_string(),
                    status: "STOPPED".into(),
                    output: None,
                    provider: None,
                    model: None,
                    cost_usd: 0.0,
                    tokens: 0,
                    route: None,
                    detail: Some("not started: stop requested".into()),
                });
                continue;
            }
            let me = Arc::clone(self);
            let opts = opts.clone();
            let cb = on_done.clone();
            set.spawn(async move {
                let r = me.run_one(&path, &opts).await;
                cb(&r);
                drop(permit);
                r
            });
            while let Some(done) = set.try_join_next() {
                results.push(done.unwrap_or_else(panic_result));
            }
        }
        while let Some(done) = set.join_next().await {
            results.push(done.unwrap_or_else(panic_result));
        }
        if !opts.dry_run {
            let list: String = results
                .iter()
                .filter_map(|r| r.output.clone())
                .map(|p| p + "\n")
                .collect();
            let _ = std::fs::create_dir_all(&opts.outbox);
            let _ = std::fs::write(opts.outbox.join("_last_run.txt"), list);
        }
        results
    }
}

fn panic_result(e: tokio::task::JoinError) -> DocResult {
    DocResult {
        doc_id: "?".into(),
        uid: String::new(),
        path: "?".into(),
        status: "FAILED".into(),
        output: None,
        provider: None,
        model: None,
        cost_usd: 0.0,
        tokens: 0,
        route: None,
        detail: Some(format!("worker crashed: {e}")),
    }
}

/// Import Markdown into the context library. Each `#`/`##`/`###` section becomes one snippet; with
/// `lines`, each non-blank line does (for axiom lists). JSON arrays of `{key,title,body,kind}` also work.
pub fn import_context(forge: &Forge, path: &Path, kind: &str, lines: bool) -> Result<usize> {
    let mut files = Vec::new();
    if path.is_dir() {
        walk(path, &mut files)?;
    } else {
        files.push(path.to_path_buf());
    }
    let mut n = 0;
    for f in files {
        let text = std::fs::read_to_string(&f)?;
        let src = f.display().to_string();
        let stem = f
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("doc")
            .to_string();
        if f.extension().is_some_and(|x| x == "json") {
            let arr: Vec<serde_json::Value> =
                serde_json::from_str(&text).with_context(|| format!("{src}: want a JSON array"))?;
            for (i, v) in arr.iter().enumerate() {
                let body = v["body"]
                    .as_str()
                    .or(v["text"].as_str())
                    .ok_or_else(|| anyhow!("{src}[{i}]: no body/text"))?;
                let key = v["key"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{stem}#{i}"));
                let title = v["title"].as_str().unwrap_or(&key).to_string();
                forge.store.context_put(
                    &key,
                    v["kind"].as_str().unwrap_or(kind),
                    &title,
                    body,
                    Some(&src),
                    v["tags"].as_str(),
                )?;
                n += 1;
            }
            continue;
        }
        if lines {
            for (i, line) in text.lines().enumerate() {
                let l = line
                    .trim()
                    .trim_start_matches(|c: char| {
                        c.is_ascii_digit() || c == '.' || c == '-' || c == '*'
                    })
                    .trim();
                if l.chars().count() < 8 || l.starts_with('#') {
                    continue;
                }
                let title: String = l.split('|').next().unwrap_or(l).chars().take(80).collect();
                forge.store.context_put(
                    &format!("{stem}:L{}", i + 1),
                    kind,
                    title.trim(),
                    l,
                    Some(&src),
                    None,
                )?;
                n += 1;
            }
            continue;
        }
        let mut title = stem.clone();
        let mut body = String::new();
        let mut idx = 0;
        let flush = |title: &str, body: &str, idx: usize| -> Result<usize> {
            if body.trim().is_empty() {
                return Ok(0);
            }
            forge.store.context_put(
                &format!("{stem}#{idx}"),
                kind,
                title,
                body.trim(),
                Some(&src),
                None,
            )?;
            Ok(1)
        };
        for line in text.lines() {
            let t = line.trim_start();
            if t.starts_with("# ") || t.starts_with("## ") || t.starts_with("### ") {
                n += flush(&title, &body, idx)?;
                idx += 1;
                title = t.trim_start_matches('#').trim().to_string();
                body.clear();
            } else {
                body.push_str(line);
                body.push('\n');
            }
        }
        n += flush(&title, &body, idx)?;
    }
    Ok(n)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                walk(&p, out)?;
            }
        } else if p.extension().is_some_and(|x| x == "md" || x == "json") {
            out.push(p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_terms_prefers_repeated_content_words() {
        let t = key_terms(
            "Grace grace GRACE entropy entropy with with with the and coherence",
            3,
        );
        assert_eq!(t, "grace entropy coherence");
    }

    #[test]
    fn context_render_caps_length() {
        let h = ContextHit {
            key: "k".into(),
            kind: "law".into(),
            title: "T".into(),
            body: "x".repeat(500),
            source: None,
            score: 1.0,
        };
        let s = render_context(&[h.clone(), h], 300);
        assert!(s.chars().count() <= 300);
    }
}
