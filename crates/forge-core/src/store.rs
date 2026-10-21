//! The SQLite ledger: every call, every document's state, the context library (FTS5 search),
//! and the canon candidate graph. One file, `_data/forge.sqlite`.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS calls (
    id                INTEGER PRIMARY KEY,
    task              TEXT NOT NULL,
    doc_id            TEXT,
    route             TEXT,
    provider          TEXT NOT NULL,
    model             TEXT NOT NULL,
    tier              TEXT,
    rung              INTEGER NOT NULL DEFAULT 0,
    attempt           INTEGER NOT NULL DEFAULT 1,
    status            TEXT NOT NULL,          -- SUCCESS | ERROR
    error_kind        TEXT,
    error             TEXT,
    prompt_tokens     INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd          REAL NOT NULL DEFAULT 0,
    latency_ms        INTEGER NOT NULL DEFAULT 0,
    finish_reason     TEXT,
    prompt_sha256     TEXT,
    prompt_text       TEXT,
    output_text       TEXT,
    started_at        TEXT NOT NULL,
    finished_at       TEXT NOT NULL,
    archived          INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS calls_day ON calls(provider, model, started_at);
CREATE INDEX IF NOT EXISTS calls_doc ON calls(doc_id);

CREATE TABLE IF NOT EXISTS documents (
    doc_id       TEXT PRIMARY KEY,
    path         TEXT,
    job          TEXT,
    source_hash  TEXT,
    status       TEXT NOT NULL,               -- NEW | DONE | FAILED
    last_call_id INTEGER,
    last_output  TEXT,
    notes        TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    archived     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS doc_events (
    id     INTEGER PRIMARY KEY,
    doc_id TEXT NOT NULL,
    at     TEXT NOT NULL,
    event  TEXT NOT NULL,
    detail TEXT
);

CREATE TABLE IF NOT EXISTS context (
    id         INTEGER PRIMARY KEY,
    key        TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL,
    title      TEXT NOT NULL,
    body       TEXT NOT NULL,
    source     TEXT,
    tags       TEXT,
    updated_at TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS context_fts USING fts5(
    title, body, tags, content='context', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS context_ai AFTER INSERT ON context BEGIN
    INSERT INTO context_fts(rowid, title, body, tags) VALUES (new.id, new.title, new.body, new.tags);
END;
CREATE TRIGGER IF NOT EXISTS context_ad AFTER DELETE ON context BEGIN
    INSERT INTO context_fts(context_fts, rowid, title, body, tags) VALUES ('delete', old.id, old.title, old.body, old.tags);
END;
CREATE TRIGGER IF NOT EXISTS context_au AFTER UPDATE ON context BEGIN
    INSERT INTO context_fts(context_fts, rowid, title, body, tags) VALUES ('delete', old.id, old.title, old.body, old.tags);
    INSERT INTO context_fts(rowid, title, body, tags) VALUES (new.id, new.title, new.body, new.tags);
END;

CREATE TABLE IF NOT EXISTS candidates (
    candidate_id   TEXT PRIMARY KEY,
    atom_id        TEXT,
    content_hash   TEXT NOT NULL,
    status         TEXT NOT NULL,
    verdict        TEXT NOT NULL,
    receipt_id     TEXT NOT NULL,
    author         TEXT,
    source_uri     TEXT,
    source_span    TEXT,
    depends_on     TEXT NOT NULL DEFAULT '[]',
    note           TEXT,
    text           TEXT,
    payload        TEXT,
    schema_version TEXT,
    reasons        TEXT NOT NULL DEFAULT '[]',
    created_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS candidates_atom ON candidates(atom_id);
CREATE INDEX IF NOT EXISTS candidates_hash ON candidates(content_hash);

CREATE TABLE IF NOT EXISTS conflicts (
    conflict_id TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    left_id     TEXT NOT NULL,
    right_id    TEXT NOT NULL,
    affected    TEXT NOT NULL DEFAULT '[]',
    reason      TEXT,
    status      TEXT NOT NULL DEFAULT 'open',
    created_at  TEXT NOT NULL
);

-- Identical request (same route, messages, mode, limits) -> stored reply. Skipped with no_cache / --redo.
CREATE TABLE IF NOT EXISTS cache (
    key               TEXT PRIMARY KEY,
    provider          TEXT NOT NULL,
    model             TEXT NOT NULL,
    tier              TEXT,
    text              TEXT NOT NULL,
    prompt_tokens     INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    finish_reason     TEXT,
    hits              INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL
);

-- Lineage: which source text (by hash) produced which output, through which job and call.
CREATE TABLE IF NOT EXISTS derivatives (
    id            INTEGER PRIMARY KEY,
    parent_doc_id TEXT NOT NULL,
    parent_sha256 TEXT NOT NULL,
    job           TEXT NOT NULL,
    output_path   TEXT NOT NULL,
    output_sha256 TEXT NOT NULL,
    call_id       INTEGER,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS derivatives_parent ON derivatives(parent_doc_id);

CREATE TABLE IF NOT EXISTS audits (
    id   INTEGER PRIMARY KEY,
    at   TEXT NOT NULL,
    body TEXT NOT NULL
);

-- Latest state per (document, job). `documents` keeps only the last job, which made multi-stage runs
-- re-run (and re-bill) earlier stages; skip-unchanged reads this table instead.
CREATE TABLE IF NOT EXISTS doc_jobs (
    doc_id      TEXT NOT NULL,
    job         TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    status      TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (doc_id, job)
);

-- Permanent paper ids. A uid survives every edit; content hashes are its versions.
CREATE TABLE IF NOT EXISTS papers (
    uid         TEXT PRIMARY KEY,
    path        TEXT NOT NULL,
    current_sha TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS papers_path ON papers(path);
CREATE TABLE IF NOT EXISTS paper_versions (
    uid     TEXT NOT NULL,
    sha256  TEXT NOT NULL,
    path    TEXT NOT NULL,
    seen_at TEXT NOT NULL,
    PRIMARY KEY (uid, sha256)
);
CREATE INDEX IF NOT EXISTS paper_versions_sha ON paper_versions(sha256);
-- Every uid forge had to mint or reattach without a frontmatter id. Reviewed by a human, never guessed.
CREATE TABLE IF NOT EXISTS uid_orphans (
    id     INTEGER PRIMARY KEY,
    at     TEXT NOT NULL,
    path   TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    uid    TEXT NOT NULL,
    reason TEXT NOT NULL
);

-- Pipeline runs that survive a restart: mode, gate state, and a cursor into `paths`.
CREATE TABLE IF NOT EXISTS runs (
    id         TEXT PRIMARY KEY,
    mode       TEXT NOT NULL,              -- all | first-then-approve | step
    status     TEXT NOT NULL,              -- running | awaiting_approval | paused | done | failed | cancelled
    spec       TEXT NOT NULL,              -- JSON: stages, outbox, route, redo, workers
    paths      TEXT NOT NULL,              -- JSON array, in run order
    cursor     INTEGER NOT NULL DEFAULT 0, -- index of the next paper to start
    approved   INTEGER NOT NULL DEFAULT 0, -- first-then-approve: gate already passed
    note       TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
-- One row per paper x stage job in a run: the dashboard grid.
CREATE TABLE IF NOT EXISTS run_cells (
    run_id     TEXT NOT NULL,
    uid        TEXT NOT NULL,
    doc_id     TEXT NOT NULL,
    path       TEXT NOT NULL,
    stage      INTEGER NOT NULL,
    job        TEXT NOT NULL,
    status     TEXT NOT NULL,              -- pending | running | ok | failed | blocked | skipped | awaiting_approval
    cost_usd   REAL NOT NULL DEFAULT 0,
    tokens     INTEGER NOT NULL DEFAULT 0,
    output_ref TEXT,
    error      TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (run_id, uid, job)
);
"#;

/// Columns added after the first release. `ALTER TABLE ADD COLUMN` only if missing, so old ledgers open.
const MIGRATIONS: &[(&str, &str, &str)] = &[
    ("calls", "uid", "TEXT"),
    ("calls", "focus_sha256", "TEXT"),
    ("documents", "uid", "TEXT"),
    ("derivatives", "uid", "TEXT"),
];

fn migrate(conn: &Connection) -> Result<()> {
    for (table, col, ty) in MIGRATIONS {
        let mut st = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let have = st
            .query_map([], |r| r.get::<_, String>(1))?
            .flatten()
            .any(|c| c == *col);
        if !have {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty}"))?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS calls_uid ON calls(uid);
         CREATE INDEX IF NOT EXISTS derivatives_uid ON derivatives(uid);",
    )?;
    Ok(())
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Thread-safe handle. Each operation takes the lock briefly; never hold it across an `.await`.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CallRecord {
    pub task: String,
    pub doc_id: Option<String>,
    pub uid: Option<String>,
    pub focus_sha256: Option<String>,
    pub route: Option<String>,
    pub provider: String,
    pub model: String,
    pub tier: String,
    pub rung: u32,
    pub attempt: u32,
    pub status: String,
    pub error_kind: Option<String>,
    pub error: Option<String>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub finish_reason: Option<String>,
    pub prompt_sha256: String,
    pub prompt_text: Option<String>,
    pub output_text: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextHit {
    pub key: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub source: Option<String>,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CachedReply {
    pub provider: String,
    pub model: String,
    pub tier: Option<String>,
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocState {
    pub doc_id: String,
    pub path: Option<String>,
    pub job: Option<String>,
    pub source_hash: Option<String>,
    pub status: String,
    pub last_call_id: Option<i64>,
    pub last_output: Option<String>,
    pub notes: Option<String>,
    pub updated_at: String,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: path.display().to_string(),
        })
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: ":memory:".into(),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------------- calls ----------------

    pub fn record_call(&self, c: &CallRecord) -> Result<i64> {
        let db = self.lock();
        db.execute(
            "INSERT INTO calls (task, doc_id, route, provider, model, tier, rung, attempt, status, error_kind, error,
                prompt_tokens, completion_tokens, cost_usd, latency_ms, finish_reason, prompt_sha256, prompt_text,
                output_text, started_at, finished_at, uid, focus_sha256)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
            params![
                c.task,
                c.doc_id,
                c.route,
                c.provider,
                c.model,
                c.tier,
                c.rung,
                c.attempt,
                c.status,
                c.error_kind,
                c.error,
                c.prompt_tokens,
                c.completion_tokens,
                c.cost_usd,
                c.latency_ms as i64,
                c.finish_reason,
                c.prompt_sha256,
                c.prompt_text,
                c.output_text,
                c.started_at,
                c.finished_at,
                c.uid,
                c.focus_sha256
            ],
        )?;
        Ok(db.last_insert_rowid())
    }

    /// Requests sent to `provider`/`model` since UTC midnight (all outcomes count against a quota).
    pub fn requests_today(&self, provider: &str, model: &str) -> Result<u32> {
        let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let n: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM calls WHERE provider = ?1 AND model = ?2 AND status <> 'CACHED' AND substr(started_at,1,10) = ?3",
            params![provider, model, day],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    pub fn spend_today(&self) -> Result<f64> {
        let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let v: f64 = self.lock().query_row(
            "SELECT COALESCE(SUM(cost_usd),0) FROM calls WHERE substr(started_at,1,10) = ?1",
            params![day],
            |r| r.get(0),
        )?;
        Ok(v)
    }

    pub fn recent_calls(&self, limit: u32) -> Result<Vec<Value>> {
        let db = self.lock();
        let mut st = db.prepare(
            "SELECT id, task, doc_id, route, provider, model, tier, rung, attempt, status, error_kind, error,
                    prompt_tokens, completion_tokens, cost_usd, latency_ms, started_at
             FROM calls WHERE archived = 0 ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = st.query_map(params![limit], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?, "task": r.get::<_, String>(1)?, "doc_id": r.get::<_, Option<String>>(2)?,
                "route": r.get::<_, Option<String>>(3)?, "provider": r.get::<_, String>(4)?, "model": r.get::<_, String>(5)?,
                "tier": r.get::<_, Option<String>>(6)?, "rung": r.get::<_, i64>(7)?, "attempt": r.get::<_, i64>(8)?,
                "status": r.get::<_, String>(9)?, "error_kind": r.get::<_, Option<String>>(10)?,
                "error": r.get::<_, Option<String>>(11)?, "prompt_tokens": r.get::<_, i64>(12)?,
                "completion_tokens": r.get::<_, i64>(13)?, "cost_usd": r.get::<_, f64>(14)?,
                "latency_ms": r.get::<_, i64>(15)?, "started_at": r.get::<_, String>(16)?,
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Per provider/model totals over the last `days` days.
    pub fn stats(&self, days: u32) -> Result<Vec<Value>> {
        let since = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        let db = self.lock();
        let mut st = db.prepare(
            "SELECT provider, model, COUNT(*), SUM(status='SUCCESS'), SUM(status<>'SUCCESS'),
                    SUM(prompt_tokens), SUM(completion_tokens), SUM(cost_usd), AVG(latency_ms)
             FROM calls WHERE started_at >= ?1 GROUP BY provider, model ORDER BY SUM(cost_usd) DESC, COUNT(*) DESC",
        )?;
        let rows = st.query_map(params![since], |r| {
            Ok(serde_json::json!({
                "provider": r.get::<_, String>(0)?, "model": r.get::<_, String>(1)?, "calls": r.get::<_, i64>(2)?,
                "ok": r.get::<_, i64>(3)?, "errors": r.get::<_, i64>(4)?, "prompt_tokens": r.get::<_, i64>(5)?,
                "completion_tokens": r.get::<_, i64>(6)?, "cost_usd": r.get::<_, f64>(7)?,
                "avg_latency_ms": r.get::<_, f64>(8)?,
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Mark calls and documents older than `days` as archived. Nothing is deleted.
    pub fn archive_older_than(&self, days: u32) -> Result<(usize, usize)> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        let db = self.lock();
        let c = db.execute(
            "UPDATE calls SET archived = 1 WHERE archived = 0 AND started_at < ?1",
            params![cutoff],
        )?;
        let d = db.execute(
            "UPDATE documents SET archived = 1 WHERE archived = 0 AND updated_at < ?1",
            params![cutoff],
        )?;
        Ok((c, d))
    }

    // ---------------- documents ----------------

    pub fn doc_get(&self, doc_id: &str) -> Result<Option<DocState>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT doc_id, path, job, source_hash, status, last_call_id, last_output, notes, updated_at
                 FROM documents WHERE doc_id = ?1",
                params![doc_id],
                |r| {
                    Ok(DocState {
                        doc_id: r.get(0)?,
                        path: r.get(1)?,
                        job: r.get(2)?,
                        source_hash: r.get(3)?,
                        status: r.get(4)?,
                        last_call_id: r.get(5)?,
                        last_output: r.get(6)?,
                        notes: r.get(7)?,
                        updated_at: r.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn doc_upsert(
        &self,
        doc_id: &str,
        path: &str,
        job: &str,
        source_hash: &str,
        status: &str,
        last_call_id: Option<i64>,
        last_output: Option<&str>,
    ) -> Result<()> {
        let t = now();
        let db = self.lock();
        db.execute(
            "INSERT INTO documents (doc_id, path, job, source_hash, status, last_call_id, last_output, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)
             ON CONFLICT(doc_id) DO UPDATE SET path=excluded.path, job=excluded.job, source_hash=excluded.source_hash,
                status=excluded.status, last_call_id=COALESCE(excluded.last_call_id, last_call_id),
                last_output=COALESCE(excluded.last_output, last_output), updated_at=excluded.updated_at, archived=0",
            params![doc_id, path, job, source_hash, status, last_call_id, last_output, t],
        )?;
        db.execute(
            "INSERT INTO doc_jobs (doc_id, job, source_hash, status, updated_at) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(doc_id, job) DO UPDATE SET source_hash=excluded.source_hash, status=excluded.status,
                updated_at=excluded.updated_at",
            params![doc_id, job, source_hash, status, t],
        )?;
        db.execute(
            "INSERT INTO doc_events (doc_id, at, event, detail) VALUES (?1, ?2, ?3, ?4)",
            params![doc_id, t, status, job],
        )?;
        Ok(())
    }

    /// This exact source already went through this exact job successfully.
    pub fn doc_job_done(&self, doc_id: &str, job: &str, source_hash: &str) -> Result<bool> {
        Ok(self
            .lock()
            .query_row(
                "SELECT 1 FROM doc_jobs WHERE doc_id = ?1 AND job = ?2 AND source_hash = ?3 AND status = 'DONE'",
                params![doc_id, job, source_hash],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn doc_note(&self, doc_id: &str, note: &str) -> Result<()> {
        let t = now();
        let db = self.lock();
        db.execute(
            "INSERT INTO documents (doc_id, status, notes, created_at, updated_at) VALUES (?1, 'NEW', ?2, ?3, ?3)
             ON CONFLICT(doc_id) DO UPDATE SET notes = COALESCE(notes || char(10), '') || ?2, updated_at = ?3",
            params![doc_id, note, t],
        )?;
        db.execute(
            "INSERT INTO doc_events (doc_id, at, event, detail) VALUES (?1, ?2, 'NOTE', ?3)",
            params![doc_id, t, note],
        )?;
        Ok(())
    }

    pub fn doc_events(&self, doc_id: &str, limit: u32) -> Result<Vec<Value>> {
        let db = self.lock();
        let mut st = db.prepare(
            "SELECT at, event, detail FROM doc_events WHERE doc_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![doc_id, limit], |r| {
            Ok(serde_json::json!({"at": r.get::<_, String>(0)?, "event": r.get::<_, String>(1)?, "detail": r.get::<_, Option<String>>(2)?}))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---------------- context ----------------

    pub fn context_put(
        &self,
        key: &str,
        kind: &str,
        title: &str,
        body: &str,
        source: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        self.lock().execute(
            "INSERT INTO context (key, kind, title, body, source, tags, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(key) DO UPDATE SET kind=excluded.kind, title=excluded.title, body=excluded.body,
                source=excluded.source, tags=excluded.tags, updated_at=excluded.updated_at",
            params![key, kind, title, body, source, tags, now()],
        )?;
        Ok(())
    }

    pub fn context_get(&self, key: &str) -> Result<Option<ContextHit>> {
        Ok(self
            .lock()
            .query_row(
                "SELECT key, kind, title, body, source FROM context WHERE key = ?1",
                params![key],
                |r| {
                    Ok(ContextHit {
                        key: r.get(0)?,
                        kind: r.get(1)?,
                        title: r.get(2)?,
                        body: r.get(3)?,
                        source: r.get(4)?,
                        score: 0.0,
                    })
                },
            )
            .optional()?)
    }

    pub fn context_count(&self) -> Result<i64> {
        Ok(self
            .lock()
            .query_row("SELECT COUNT(*) FROM context", [], |r| r.get(0))?)
    }

    /// BM25 search. Free text is turned into an OR of quoted terms, so user input can't break FTS syntax.
    pub fn context_search(
        &self,
        query: &str,
        limit: usize,
        kind: Option<&str>,
    ) -> Result<Vec<ContextHit>> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(vec![]);
        }
        let db = self.lock();
        let mut st = db.prepare(
            "SELECT c.key, c.kind, c.title, c.body, c.source, bm25(context_fts, 4.0, 1.0, 2.0) AS score
             FROM context_fts JOIN context c ON c.id = context_fts.rowid
             WHERE context_fts MATCH ?1 AND (?2 IS NULL OR c.kind = ?2)
             ORDER BY score LIMIT ?3",
        )?;
        let rows = st.query_map(params![fts, kind, limit as i64], |r| {
            Ok(ContextHit {
                key: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                body: r.get(3)?,
                source: r.get(4)?,
                score: -r.get::<_, f64>(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---------------- cache ----------------

    pub fn cache_get(&self, key: &str) -> Result<Option<CachedReply>> {
        let db = self.lock();
        let hit = db
            .query_row(
                "SELECT provider, model, tier, text, prompt_tokens, completion_tokens, finish_reason FROM cache WHERE key = ?1",
                params![key],
                |r| {
                    Ok(CachedReply {
                        provider: r.get(0)?,
                        model: r.get(1)?,
                        tier: r.get(2)?,
                        text: r.get(3)?,
                        prompt_tokens: r.get(4)?,
                        completion_tokens: r.get(5)?,
                        finish_reason: r.get(6)?,
                    })
                },
            )
            .optional()?;
        if hit.is_some() {
            db.execute(
                "UPDATE cache SET hits = hits + 1 WHERE key = ?1",
                params![key],
            )?;
        }
        Ok(hit)
    }

    pub fn cache_put(&self, key: &str, c: &CachedReply) -> Result<()> {
        self.lock().execute(
            "INSERT OR REPLACE INTO cache (key, provider, model, tier, text, prompt_tokens, completion_tokens, finish_reason, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![key, c.provider, c.model, c.tier, c.text, c.prompt_tokens, c.completion_tokens, c.finish_reason, now()],
        )?;
        Ok(())
    }

    // ---------------- lineage ----------------

    pub fn derivative_add(
        &self,
        parent_doc_id: &str,
        parent_sha256: &str,
        job: &str,
        output_path: &str,
        output_sha256: &str,
        call_id: Option<i64>,
    ) -> Result<()> {
        self.lock().execute(
            "INSERT INTO derivatives (parent_doc_id, parent_sha256, job, output_path, output_sha256, call_id, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![parent_doc_id, parent_sha256, job, output_path, output_sha256, call_id, now()],
        )?;
        Ok(())
    }

    pub fn derivatives(&self, parent_doc_id: &str, limit: u32) -> Result<Vec<Value>> {
        let db = self.lock();
        let mut st = db.prepare(
            "SELECT parent_sha256, job, output_path, output_sha256, call_id, created_at FROM derivatives
             WHERE parent_doc_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![parent_doc_id, limit], |r| {
            Ok(serde_json::json!({
                "parent_sha256": r.get::<_, String>(0)?, "job": r.get::<_, String>(1)?, "output_path": r.get::<_, String>(2)?,
                "output_sha256": r.get::<_, String>(3)?, "call_id": r.get::<_, Option<i64>>(4)?, "created_at": r.get::<_, String>(5)?,
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Stamp a document's state row and its untagged lineage rows with the paper's permanent uid.
    pub fn tag_uid(&self, doc_id: &str, uid: &str) -> Result<()> {
        let db = self.lock();
        db.execute(
            "UPDATE documents SET uid = ?2 WHERE doc_id = ?1",
            params![doc_id, uid],
        )?;
        db.execute(
            "UPDATE derivatives SET uid = ?2 WHERE parent_doc_id = ?1 AND uid IS NULL",
            params![doc_id, uid],
        )?;
        Ok(())
    }

    /// Lineage by permanent id: every output any version of this paper produced.
    pub fn derivatives_by_uid(&self, uid: &str, limit: u32) -> Result<Vec<Value>> {
        let db = self.lock();
        let mut st = db.prepare(
            "SELECT parent_doc_id, parent_sha256, job, output_path, output_sha256, call_id, created_at FROM derivatives
             WHERE uid = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![uid, limit], |r| {
            Ok(serde_json::json!({
                "doc_id": r.get::<_, String>(0)?, "parent_sha256": r.get::<_, String>(1)?, "job": r.get::<_, String>(2)?,
                "output_path": r.get::<_, String>(3)?, "output_sha256": r.get::<_, String>(4)?,
                "call_id": r.get::<_, Option<i64>>(5)?, "created_at": r.get::<_, String>(6)?,
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn audit_put(&self, body: &str) -> Result<i64> {
        let db = self.lock();
        db.execute(
            "INSERT INTO audits (at, body) VALUES (?1, ?2)",
            params![now(), body],
        )?;
        Ok(db.last_insert_rowid())
    }
}

pub fn fts_query(text: &str) -> String {
    let mut seen = std::collections::BTreeSet::new();
    let terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(|t| t.to_lowercase())
        .filter(|t| seen.insert(t.clone()))
        .take(64)
        .map(|t| format!("\"{t}\""))
        .collect();
    terms.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts5_search_ranks_and_survives_hostile_input() {
        let s = Store::open_memory().unwrap();
        s.context_put(
            "law4",
            "law",
            "Law 4 Strong Force",
            "Love as the binding potential, Yukawa V(r)",
            None,
            Some("fruits"),
        )
        .unwrap();
        s.context_put(
            "law5",
            "law",
            "Law 5 Thermodynamics",
            "Judgment and heat death; F = E - TS",
            None,
            None,
        )
        .unwrap();
        let hits = s
            .context_search("what binds? Yukawa \"OR NOT (", 5, None)
            .unwrap();
        assert_eq!(hits[0].key, "law4");
        s.context_put(
            "law4",
            "law",
            "Law 4",
            "replaced body about entropy",
            None,
            None,
        )
        .unwrap();
        assert_eq!(s.context_search("Yukawa", 5, None).unwrap().len(), 0);
        assert_eq!(
            s.context_search("entropy", 5, Some("law")).unwrap()[0].key,
            "law4"
        );
    }

    #[test]
    fn quota_and_spend_counters() {
        let s = Store::open_memory().unwrap();
        let t = now();
        for status in ["SUCCESS", "ERROR"] {
            s.record_call(&CallRecord {
                task: "t".into(),
                provider: "p".into(),
                model: "m".into(),
                status: status.into(),
                cost_usd: 0.5,
                started_at: t.clone(),
                finished_at: t.clone(),
                ..Default::default()
            })
            .unwrap();
        }
        assert_eq!(s.requests_today("p", "m").unwrap(), 2);
        assert!((s.spend_today().unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(s.stats(1).unwrap()[0]["errors"], 1);
    }
}
