//! The clipboard librarian.
//!
//! `sort`: every new clip (a .md/.txt file in a folder, wherever the clipboard app drops them) is classified
//! by a model and filed as `<library>/<category>/<title>.md` with tags and a one-line summary. Originals
//! are never moved or deleted; the ledger remembers what was filed, so a rerun only sees new clips.
//! Clips that look like secrets (keys, passwords, private keys, card numbers) are copied to
//! `<library>/_held/` and never sent to any model.
//!
//! `review`: reads the library's index (categories, titles, summaries; not full clips) and asks a model
//! for a report: categories to merge or rename, misfiled clips, what stands out. Written to
//! `<library>/_review/<date>.md`. It changes nothing; you decide.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::Tier;
use crate::pipeline::{collect, doc_id};
use crate::router::RouteRequest;
use crate::store::now;
use crate::template;
use crate::types::Message;
use crate::Forge;

const SORT_JOB: &str = "sort-clip";
const REVIEW_JOB: &str = "review-clips";
/// The classifier sees at most this much of a clip; the filed copy keeps all of it.
const EXCERPT_CHARS: usize = 6000;

#[derive(Debug, Clone)]
pub struct SortOptions {
    pub library: PathBuf,
    pub route: Option<String>,
    pub max_tier: Option<Tier>,
    pub redo: bool,
    pub dry_run: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Filed {
    pub clip: String,
    /// FILED | HELD | SKIPPED | FAILED | DRY_RUN
    pub status: String,
    pub category: Option<String>,
    pub title: Option<String>,
    pub path: Option<String>,
    pub provider: Option<String>,
    pub cost_usd: f64,
    pub detail: Option<String>,
}

/// Why a clip must not leave the machine, if it looks like a secret.
pub fn secret_reason(text: &str) -> Option<&'static str> {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "private key"),
            (r"\bsk-(?:ant-|proj-)?[A-Za-z0-9_\-]{20,}", "API key (sk-…)"),
            (r"\bAKIA[0-9A-Z]{16}\b", "AWS access key"),
            (r"\bgh[pousr]_[A-Za-z0-9]{30,}", "GitHub token"),
            (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", "Slack token"),
            (r"\bAIza[0-9A-Za-z_\-]{30,}", "Google API key"),
            (r"(?i)\b(?:password|passwd|pwd|passcode|secret|api[_ -]?key|access[_ -]?token|auth[_ -]?token)\b\s*[:=]\s*\S{4,}", "password or key assignment"),
            (r"\b(?:\d[ -]?){15,16}\b", "card-like number"),
        ]
        .into_iter()
        .map(|(p, why)| (Regex::new(p).expect("valid secret pattern"), why))
        .collect()
    });
    rules
        .iter()
        .find(|(re, _)| re.is_match(text))
        .map(|(_, why)| *why)
}

/// A safe, readable file or folder name.
pub fn safe_name(s: &str, max: usize) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.,()&'".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    let squashed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = squashed.chars().take(max).collect();
    let trimmed = trimmed
        .trim_matches(|c: char| c == '.' || c == ' ')
        .to_string();
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "lpt1", "lpt2", "lpt3",
    ];
    if trimmed.is_empty() || RESERVED.contains(&trimmed.to_lowercase().as_str()) {
        "Untitled".into()
    } else {
        trimmed
    }
}

/// Existing category folders (not starting with `_` or `.`).
pub fn categories(library: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(library)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| !n.starts_with('_') && !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.md"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).md")))
        .find(|p| !p.exists())
        .expect("some free name")
}

fn yaml_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''").replace(['\n', '\r'], " "))
}

impl Forge {
    pub async fn sort_clips(&self, inbox: &Path, opts: &SortOptions) -> Result<Vec<Filed>> {
        let mut clips = collect(inbox)?;
        clips.retain(|p| !p.starts_with(&opts.library));
        if let Some(n) = opts.limit {
            clips.truncate(n);
        }
        let tpl = template::load(&self.config.paths.templates_dir, SORT_JOB)?;
        std::fs::create_dir_all(&opts.library)?;
        let mut out = Vec::new();
        // One at a time on purpose: each filing updates the category list the next clip sees,
        // so the library converges instead of inventing near-duplicate folders in parallel.
        for clip in clips {
            out.push(self.sort_one(&clip, &tpl, opts).await);
        }
        if !opts.dry_run {
            write_index(&opts.library)?;
        }
        Ok(out)
    }

    async fn sort_one(&self, clip: &Path, tpl: &template::Template, opts: &SortOptions) -> Filed {
        let id = format!("clip:{}", doc_id(clip));
        let mut f = Filed {
            clip: clip.display().to_string(),
            status: "FAILED".into(),
            category: None,
            title: None,
            path: None,
            provider: None,
            cost_usd: 0.0,
            detail: None,
        };
        let text = match std::fs::read_to_string(clip) {
            Ok(t) => t.trim_start_matches('\u{feff}').to_string(),
            Err(e) => {
                f.detail = Some(format!("unreadable: {e}"));
                return f;
            }
        };
        let hash = hex::encode(Sha256::digest(text.as_bytes()));
        if !opts.redo {
            if let Ok(Some(d)) = self.store.doc_get(&id) {
                if d.source_hash.as_deref() == Some(hash.as_str())
                    && (d.status == "DONE" || d.status == "HELD")
                {
                    f.status = "SKIPPED".into();
                    return f;
                }
            }
        }
        if text.trim().is_empty() {
            f.status = "SKIPPED".into();
            f.detail = Some("empty".into());
            return f;
        }
        let path_s = clip.display().to_string();

        if let Some(why) = secret_reason(&text) {
            f.status = "HELD".into();
            f.detail = Some(format!("looks like a {why}; not sent to any model"));
            if opts.dry_run {
                return f;
            }
            let held = opts.library.join("_held");
            let stem = clip.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
            let dest = unique_path(&held, &safe_name(stem, 80));
            let r = std::fs::create_dir_all(&held).and_then(|_| std::fs::copy(clip, &dest));
            if let Err(e) = r {
                f.status = "FAILED".into();
                f.detail = Some(format!("could not copy to _held: {e}"));
                return f;
            }
            f.path = Some(dest.display().to_string());
            let _ = self
                .store
                .doc_upsert(&id, &path_s, SORT_JOB, &hash, "HELD", None, Some(why));
            return f;
        }

        let excerpt: String = text.chars().take(EXCERPT_CHARS).collect();
        let cats = categories(&opts.library);
        let user = match tpl.render(serde_json::json!({
            "document": excerpt,
            "name": doc_id(clip),
            "categories": cats.join(", "),
            "truncated": text.chars().count() > EXCERPT_CHARS,
        })) {
            Ok(u) => u,
            Err(e) => {
                f.detail = Some(format!("{e:#}"));
                return f;
            }
        };
        let mut messages = Vec::new();
        if let Some(s) = &tpl.system {
            messages.push(Message::system(s.clone()));
        }
        messages.push(Message::user(user));
        if opts.dry_run {
            f.status = "DRY_RUN".into();
            f.detail = Some(
                messages
                    .iter()
                    .map(|m| format!("[{}]\n{}", m.role, m.content))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
            return f;
        }

        let req = RouteRequest {
            route: opts.route.clone().or(tpl.route.clone()),
            messages,
            json_mode: true,
            max_tier: opts.max_tier,
            max_tokens: Some(600),
            task: "clips:sort".into(),
            doc_id: Some(id.clone()),
            no_cache: opts.redo,
            ..Default::default()
        };
        let r = match self.router.complete(req).await {
            Ok(r) => r,
            Err(e) => {
                f.detail = Some(e.message);
                let _ = self
                    .store
                    .doc_upsert(&id, &path_s, SORT_JOB, &hash, "FAILED", None, None);
                return f;
            }
        };
        let Some(v) = crate::parse::extract_json(&r.reply.text) else {
            f.detail = Some("model reply had no JSON".into());
            let _ = self
                .store
                .doc_upsert(&id, &path_s, SORT_JOB, &hash, "FAILED", r.call_id, None);
            return f;
        };
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let category = safe_name(&s("category"), 60);
        let category = if category == "Untitled" {
            "Unsorted".to_string()
        } else {
            category
        };
        let title = safe_name(&s("title"), 90);
        let tags: Vec<String> = v
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|x| x.trim().to_lowercase().replace(' ', "-"))
                    .filter(|x| !x.is_empty())
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();

        let dir = opts.library.join(&category);
        let dest = unique_path(&dir, &title);
        let doc = format!(
            "---\ntitle: {}\ncategory: {}\nkind: {}\ntags: [{}]\nsummary: {}\nsource: {}\nsorted_at: {}\nsorted_by: {} / {}\n---\n\n{}\n",
            yaml_str(&s("title")),
            yaml_str(&category),
            yaml_str(&s("kind")),
            tags.join(", "),
            yaml_str(&s("summary")),
            yaml_str(&path_s),
            now(),
            r.provider,
            r.model,
            text.trim_end()
        );
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&dest, &doc)) {
            f.detail = Some(format!("could not write {}: {e}", dest.display()));
            return f;
        }
        let dest_s = dest.display().to_string();
        let _ = self.store.doc_upsert(
            &id,
            &path_s,
            SORT_JOB,
            &hash,
            "DONE",
            r.call_id,
            Some(&dest_s),
        );
        let _ = self.store.derivative_add(
            &id,
            &hash,
            SORT_JOB,
            &dest_s,
            &hex::encode(Sha256::digest(doc.as_bytes())),
            r.call_id,
        );
        f.status = "FILED".into();
        f.category = Some(category);
        f.title = Some(title);
        f.path = Some(dest_s);
        f.provider = Some(format!("{} / {}", r.provider, r.model));
        f.cost_usd = r.cost_usd;
        f
    }

    /// Ask a model to look over the whole library and write a report. Moves nothing.
    pub async fn review_clips(&self, library: &Path, route: Option<String>) -> Result<PathBuf> {
        let index = write_index(library)?;
        let tpl = template::load(&self.config.paths.templates_dir, REVIEW_JOB)?;
        let user = tpl.render(serde_json::json!({ "document": index, "name": "clip library" }))?;
        let mut messages = Vec::new();
        if let Some(s) = &tpl.system {
            messages.push(Message::system(s.clone()));
        }
        messages.push(Message::user(user));
        let r = self
            .router
            .complete(RouteRequest {
                route: route.or(tpl.route.clone()),
                messages,
                task: "clips:review".into(),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        let dir = library.join("_review");
        std::fs::create_dir_all(&dir)?;
        let path = unique_path(
            &dir,
            &chrono::Local::now().format("%Y-%m-%d %H%M").to_string(),
        );
        std::fs::write(
            &path,
            format!(
                "---\nreviewed_at: {}\nreviewed_by: {} / {}\ncost_usd: {:.5}\n---\n\n{}\n",
                now(),
                r.provider,
                r.model,
                r.cost_usd,
                r.reply.text.trim_end()
            ),
        )?;
        Ok(path)
    }
}

/// `INDEX.md`: every category with each clip's title and summary, rebuilt from the files (no model).
pub fn write_index(library: &Path) -> Result<String> {
    let mut out = format!(
        "# Clip library\n\nRebuilt {}.\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    );
    let mut total = 0;
    for cat in categories(library) {
        let mut items: Vec<(String, String)> = Vec::new();
        for e in std::fs::read_dir(library.join(&cat))?.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "md") {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let summary = text
                    .lines()
                    .find_map(|l| l.strip_prefix("summary: "))
                    .map(|s| s.trim_matches('\'').replace("''", "'"))
                    .unwrap_or_default();
                let title = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                items.push((title, summary));
            }
        }
        items.sort();
        total += items.len();
        out.push_str(&format!("\n## {cat} ({})\n", items.len()));
        for (t, s) in items {
            out.push_str(&format!(
                "- [[{cat}/{t}|{t}]]{}\n",
                if s.is_empty() {
                    String::new()
                } else {
                    format!(" — {s}")
                }
            ));
        }
    }
    let held = std::fs::read_dir(library.join("_held"))
        .map(|rd| rd.count())
        .unwrap_or(0);
    out.push_str(&format!("\n---\n{total} clip(s) filed; {held} held back as possibly sensitive (in `_held/`, never sent to a model).\n"));
    std::fs::write(library.join("INDEX.md"), &out).context("writing INDEX.md")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_caught_and_ordinary_text_is_not() {
        for s in [
            "my key sk-proj-abcdefghijklmnopqrstuvwxyz123456",
            "password: hunter22",
            "API_KEY=abc123def",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "card 4111 1111 1111 1111",
        ] {
            assert!(secret_reason(s).is_some(), "{s}");
        }
        for s in [
            "Grace as a force term, Law 2",
            "call me at 555-123-4567",
            "the password field should be hidden",
            "https://example.com/a?b=c",
        ] {
            assert!(secret_reason(s).is_none(), "{s}");
        }
    }

    #[test]
    fn safe_names() {
        assert_eq!(
            safe_name("  Physics / Theology: notes?  ", 60),
            "Physics Theology notes"
        );
        assert_eq!(safe_name("CON", 10), "Untitled");
        assert_eq!(safe_name("...", 10), "Untitled");
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use crate::config::{Config, DEFAULT_PROVIDERS, DEFAULT_SETTINGS};
    use crate::provider::MockProvider;
    use crate::{Router, Store};
    use std::sync::Arc;

    #[tokio::test]
    async fn files_clips_holds_secrets_skips_repeats_and_reviews() {
        let tmp = tempfile::tempdir().unwrap();
        let inbox = tmp.path().join("clips");
        let library = tmp.path().join("Library");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(
            inbox.join("c1.txt"),
            "Noether: time-translation symmetry conserves energy",
        )
        .unwrap();
        std::fs::write(inbox.join("c2.txt"), "password: correct-horse-battery").unwrap();

        let cfg = Config::from_strings(
            DEFAULT_PROVIDERS,
            DEFAULT_SETTINGS,
            tmp.path().to_path_buf(),
            None,
        )
        .unwrap();
        let store = Store::open_memory().unwrap();
        let mut router = Router::new(cfg.clone(), store.clone()).unwrap();
        let reply = r#"{"category":"Physics / Symmetry","title":"Noether on energy","kind":"reference","tags":["noether","Symmetry"],"summary":"Noether's theorem for energy."}"#;
        let mock = Arc::new(MockProvider::scripted(
            "mock",
            vec![
                MockProvider::ok(reply),
                MockProvider::ok("## Merge or rename\nNothing yet."),
            ],
        ));
        router.insert_provider("mock", mock.clone());
        let forge = Forge {
            config: cfg,
            store,
            router,
        };
        let opts = SortOptions {
            library: library.clone(),
            route: Some("test".into()),
            max_tier: None,
            redo: false,
            dry_run: false,
            limit: None,
        };

        let out = forge.sort_clips(&inbox, &opts).await.unwrap();
        let by = |n: &str| out.iter().find(|f| f.clip.ends_with(n)).unwrap().clone();
        assert_eq!(by("c1.txt").status, "FILED", "{out:?}");
        assert_eq!(by("c1.txt").category.as_deref(), Some("Physics Symmetry"));
        let filed = std::fs::read_to_string(
            library
                .join("Physics Symmetry")
                .join("Noether on energy.md"),
        )
        .unwrap();
        assert!(filed.contains("tags: [noether, symmetry]") && filed.contains("conserves energy"));
        assert_eq!(by("c2.txt").status, "HELD");
        assert!(library.join("_held").join("c2.md").exists());
        assert_eq!(
            mock.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the secret must never reach the model"
        );
        assert!(std::fs::read_to_string(library.join("INDEX.md"))
            .unwrap()
            .contains("Noether's theorem for energy."));

        let again = forge.sort_clips(&inbox, &opts).await.unwrap();
        assert!(again.iter().all(|f| f.status == "SKIPPED"), "{again:?}");

        let report = forge
            .review_clips(&library, Some("test".into()))
            .await
            .unwrap();
        assert!(std::fs::read_to_string(report)
            .unwrap()
            .contains("Merge or rename"));
        assert!(
            library
                .join("Physics Symmetry")
                .join("Noether on energy.md")
                .exists(),
            "review must not move anything"
        );
    }
}
