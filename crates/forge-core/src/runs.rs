//! Runs that survive a restart. A run is a list of papers and a list of stages; each stage is a set of
//! jobs (stations) that run side by side on a paper, and the next stage waits for them. A failed job
//! blocks the rest of that paper's stages. Every cell (paper x job) lives in `run_cells`: that table is
//! the dashboard grid.
//!
//! Modes: `all` keeps going; `first-then-approve` stops after the first paper until approved;
//! `step` stops after every paper. Pause, resume, and cancel take effect at the next stage boundary;
//! a call in flight is never killed. State is in SQLite, so a crash loses at most the papers in
//! flight, and those re-run cheaply because unchanged finished jobs are skipped.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::pipeline::{doc_id, RunOptions};
use crate::store::{now, Store};
use crate::Forge;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    All,
    FirstThenApprove,
    Step,
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "all" => Ok(Mode::All),
            "first-then-approve" | "first" => Ok(Mode::FirstThenApprove),
            "step" => Ok(Mode::Step),
            _ => bail!("unknown mode {s:?}: want all | first-then-approve | step"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    /// Stage order; jobs inside a stage run side by side.
    pub stages: Vec<Vec<String>>,
    pub outbox: PathBuf,
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub redo: bool,
    #[serde(default = "one")]
    pub workers: usize,
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Serialize)]
pub struct RunInfo {
    pub id: String,
    pub mode: Mode,
    pub status: String,
    pub spec: RunSpec,
    pub total: usize,
    pub cursor: usize,
    pub approved: bool,
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn load(store: &Store, id: &str) -> Result<(RunInfo, Vec<PathBuf>)> {
    let row = store
        .lock()
        .query_row(
            "SELECT id, mode, status, spec, paths, cursor, approved, note, created_at, updated_at FROM runs WHERE id = ?1",
            params![id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("no run {id}"))?;
    let paths: Vec<PathBuf> = serde_json::from_str(&row.4)?;
    Ok((
        RunInfo {
            id: row.0,
            mode: row.1.parse()?,
            status: row.2,
            spec: serde_json::from_str(&row.3)?,
            total: paths.len(),
            cursor: row.5 as usize,
            approved: row.6 != 0,
            note: row.7,
            created_at: row.8,
            updated_at: row.9,
        },
        paths,
    ))
}

pub fn get(store: &Store, id: &str) -> Result<RunInfo> {
    Ok(load(store, id)?.0)
}

pub fn list(store: &Store, limit: u32) -> Result<Vec<RunInfo>> {
    let ids: Vec<String> = store
        .lock()
        .prepare("SELECT id FROM runs ORDER BY created_at DESC LIMIT ?1")?
        .query_map(params![limit], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    ids.iter().map(|i| get(store, i)).collect()
}

fn set_status(store: &Store, id: &str, status: &str, note: Option<&str>) -> Result<()> {
    store.lock().execute(
        "UPDATE runs SET status = ?2, note = COALESCE(?3, note), updated_at = ?4 WHERE id = ?1",
        params![id, status, note, now()],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn set_cell(
    store: &Store,
    run: &str,
    uid: &str,
    job: &str,
    status: &str,
    cost: f64,
    tokens: u32,
    output: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    store.lock().execute(
        "UPDATE run_cells SET status=?4, cost_usd=?5, tokens=?6, output_ref=COALESCE(?7, output_ref), error=?8, updated_at=?9
         WHERE run_id=?1 AND uid=?2 AND job=?3",
        params![run, uid, job, status, cost, tokens, output, error, now()],
    )?;
    Ok(())
}

/// Record a new run. Papers get their permanent uid here (written into the file), and every cell starts
/// `pending`. Nothing is sent until [`execute`].
pub fn create(forge: &Forge, mode: Mode, spec: RunSpec, paths: Vec<PathBuf>) -> Result<RunInfo> {
    if spec.stages.iter().all(|s| s.is_empty()) {
        bail!("a run needs at least one stage with a job");
    }
    let mut seen = std::collections::BTreeSet::new();
    for job in spec.stages.iter().flatten() {
        if !seen.insert(job) {
            bail!("job {job:?} appears twice; each job runs once per paper");
        }
        crate::template::load(&forge.config.paths.templates_dir, job)?;
    }
    let id = format!("run_{}", crate::uid::mint());
    let t = now();
    let mut cells = Vec::new();
    for p in &paths {
        let r = crate::uid::ensure(&forge.store, p, true)?;
        for (si, stage) in spec.stages.iter().enumerate() {
            for job in stage {
                cells.push((
                    r.uid.clone(),
                    doc_id(p),
                    p.display().to_string(),
                    si + 1,
                    job.clone(),
                ));
            }
        }
    }
    {
        let db = forge.store.lock();
        db.execute(
            "INSERT INTO runs (id, mode, status, spec, paths, created_at, updated_at) VALUES (?1,?2,'running',?3,?4,?5,?5)",
            params![id, serde_json::to_value(mode)?.as_str(), serde_json::to_string(&spec)?, serde_json::to_string(&paths)?, t],
        )?;
        for (uid, doc, path, stage, job) in cells {
            db.execute(
                "INSERT OR IGNORE INTO run_cells (run_id, uid, doc_id, path, stage, job, status, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,'pending',?7)",
                params![id, uid, doc, path, stage as i64, job, t],
            )?;
        }
    }
    get(&forge.store, &id)
}

fn uid_of(store: &Store, run: &str, path: &str) -> Result<String> {
    Ok(store.lock().query_row(
        "SELECT uid FROM run_cells WHERE run_id = ?1 AND path = ?2 LIMIT 1",
        params![run, path],
        |r| r.get(0),
    )?)
}

/// One paper through every stage. Returns false if the run stopped being `running` mid-paper.
async fn paper(forge: &Arc<Forge>, run: &RunInfo, path: &std::path::Path) -> Result<bool> {
    let p = path.display().to_string();
    let uid = uid_of(&forge.store, &run.id, &p)?;
    let mut blocked: Option<String> = None;
    for stage in &run.spec.stages {
        if let Some(why) = &blocked {
            for job in stage {
                set_cell(
                    &forge.store,
                    &run.id,
                    &uid,
                    job,
                    "blocked",
                    0.0,
                    0,
                    None,
                    Some(why),
                )?;
            }
            continue;
        }
        if get(&forge.store, &run.id)?.status != "running" {
            return Ok(false);
        }
        let mut set = tokio::task::JoinSet::new();
        for job in stage {
            set_cell(
                &forge.store,
                &run.id,
                &uid,
                job,
                "running",
                0.0,
                0,
                None,
                None,
            )?;
            let me = Arc::clone(forge);
            let path = path.to_path_buf();
            let opts = RunOptions {
                job: job.clone(),
                outbox: run.spec.outbox.clone(),
                route: run.spec.route.clone(),
                redo: run.spec.redo,
                dry_run: false,
                workers: 1,
            };
            set.spawn(async move { (opts.job.clone(), me.run_one(&path, &opts).await) });
        }
        while let Some(done) = set.join_next().await {
            let (job, r) = done.map_err(|e| anyhow!("station crashed: {e}"))?;
            let status = match r.status.as_str() {
                "DONE" => "ok",
                "SKIPPED" => "skipped",
                _ => "failed",
            };
            if status == "failed" {
                blocked = Some(format!("blocked: {job} failed in an earlier stage"));
            }
            set_cell(
                &forge.store,
                &run.id,
                &uid,
                &job,
                status,
                r.cost_usd,
                r.tokens,
                r.output.as_deref(),
                (status == "failed")
                    .then_some(r.detail.as_deref())
                    .flatten(),
            )?;
        }
    }
    Ok(true)
}

/// Drive a `running` run until it finishes, hits its gate, or is paused/cancelled. Safe to call again
/// after a restart: it picks up at the cursor.
pub async fn execute(forge: &Arc<Forge>, id: &str) -> Result<RunInfo> {
    loop {
        let (run, paths) = load(&forge.store, id)?;
        if run.status != "running" {
            return Ok(run);
        }
        if run.cursor >= paths.len() {
            set_status(&forge.store, id, "done", None)?;
            return get(&forge.store, id);
        }
        let gated =
            matches!(run.mode, Mode::Step) || (run.mode == Mode::FirstThenApprove && !run.approved);
        let n = if gated { 1 } else { run.spec.workers.max(1) };
        let batch: Vec<PathBuf> = paths[run.cursor..(run.cursor + n).min(paths.len())].to_vec();
        let mut set = tokio::task::JoinSet::new();
        for p in batch.clone() {
            let me = Arc::clone(forge);
            let r = run.clone();
            set.spawn(async move { paper(&me, &r, &p).await });
        }
        let mut finished = true;
        while let Some(done) = set.join_next().await {
            match done {
                Ok(Ok(true)) => {}
                Ok(Ok(false)) => finished = false,
                Ok(Err(e)) => {
                    set_status(&forge.store, id, "failed", Some(&format!("{e:#}")))?;
                    return get(&forge.store, id);
                }
                Err(e) => {
                    set_status(
                        &forge.store,
                        id,
                        "failed",
                        Some(&format!("worker crashed: {e}")),
                    )?;
                    return get(&forge.store, id);
                }
            }
        }
        if !finished {
            // Paused or cancelled mid-paper: the cursor stays, so resume re-runs that paper (finished
            // jobs are skipped as unchanged).
            return get(&forge.store, id);
        }
        let cursor = run.cursor + batch.len();
        forge.store.lock().execute(
            "UPDATE runs SET cursor = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, cursor as i64, now()],
        )?;
        if gated && cursor < paths.len() {
            let next = paths[cursor].display().to_string();
            let uid = uid_of(&forge.store, id, &next)?;
            forge.store.lock().execute(
                "UPDATE run_cells SET status='awaiting_approval', updated_at=?3 WHERE run_id=?1 AND uid=?2 AND status='pending'",
                params![id, uid, now()],
            )?;
            // Only move to the gate if nobody paused or cancelled meanwhile.
            forge.store.lock().execute(
                "UPDATE runs SET status='awaiting_approval', updated_at=?2 WHERE id=?1 AND status='running'",
                params![id, now()],
            )?;
        }
    }
}

/// Pass the gate. `first-then-approve` then runs the rest freely; `step` runs one more paper.
pub fn approve(store: &Store, id: &str) -> Result<RunInfo> {
    let run = get(store, id)?;
    if run.status != "awaiting_approval" {
        bail!("run {id} is {}, not awaiting_approval", run.status);
    }
    let db = store.lock();
    db.execute(
        "UPDATE runs SET status='running', approved=1, updated_at=?2 WHERE id=?1",
        params![id, now()],
    )?;
    db.execute(
        "UPDATE run_cells SET status='pending' WHERE run_id=?1 AND status='awaiting_approval'",
        params![id],
    )?;
    drop(db);
    get(store, id)
}

pub fn pause(store: &Store, id: &str) -> Result<RunInfo> {
    let run = get(store, id)?;
    if !matches!(run.status.as_str(), "running" | "awaiting_approval") {
        bail!(
            "run {id} is {}; only a running or waiting run can pause",
            run.status
        );
    }
    set_status(store, id, "paused", None)?;
    get(store, id)
}

/// Back to `running` from paused (or from a failed run, to retry from the cursor). A run waiting at
/// its gate needs [`approve`], not resume.
pub fn resume(store: &Store, id: &str) -> Result<RunInfo> {
    let run = get(store, id)?;
    if !matches!(run.status.as_str(), "paused" | "failed") {
        bail!(
            "run {id} is {}; resume works on paused or failed runs",
            run.status
        );
    }
    store.lock().execute(
        "UPDATE run_cells SET status='pending' WHERE run_id=?1 AND status='running'",
        params![id],
    )?;
    set_status(store, id, "running", None)?;
    get(store, id)
}

pub fn cancel(store: &Store, id: &str) -> Result<RunInfo> {
    let run = get(store, id)?;
    if matches!(run.status.as_str(), "done" | "cancelled") {
        bail!("run {id} is already {}", run.status);
    }
    set_status(store, id, "cancelled", None)?;
    get(store, id)
}

/// After a crash or restart, runs left `running` belong to no process. Park them as `paused` so money
/// is only spent again when someone says resume.
pub fn park_orphaned(store: &Store) -> Result<usize> {
    let db = store.lock();
    db.execute(
        "UPDATE run_cells SET status='pending' WHERE status='running'
         AND run_id IN (SELECT id FROM runs WHERE status='running')",
        [],
    )?;
    Ok(db.execute(
        "UPDATE runs SET status='paused', note='interrupted by a restart; resume to continue', updated_at=?1
         WHERE status='running'",
        params![now()],
    )?)
}

/// The dashboard grid: rows keyed by uid, columns by job in stage order.
pub fn grid(store: &Store, run_id: Option<&str>) -> Result<Value> {
    let id = match run_id {
        Some(i) => i.to_string(),
        None => store
            .lock()
            .query_row(
                "SELECT id FROM runs ORDER BY created_at DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("no runs yet"))?,
    };
    let (run, paths) = load(store, &id)?;
    let columns: Vec<Value> = run
        .spec
        .stages
        .iter()
        .enumerate()
        .flat_map(|(i, s)| s.iter().map(move |j| json!({"stage": i + 1, "job": j})))
        .collect();
    let db = store.lock();
    let mut st = db.prepare(
        "SELECT uid, doc_id, path, job, status, cost_usd, tokens, output_ref, error, updated_at
         FROM run_cells WHERE run_id = ?1",
    )?;
    let mut rows: std::collections::BTreeMap<String, Value> = Default::default();
    let cells = st.query_map(params![id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            json!({"status": r.get::<_, String>(4)?, "cost_usd": r.get::<_, f64>(5)?, "tokens": r.get::<_, i64>(6)?,
                   "output_ref": r.get::<_, Option<String>>(7)?, "error": r.get::<_, Option<String>>(8)?,
                   "updated_at": r.get::<_, String>(9)?}),
        ))
    })?;
    for c in cells {
        let (uid, doc, path, job, cell) = c?;
        let row = rows
            .entry(uid.clone())
            .or_insert_with(|| json!({"uid": uid, "doc_id": doc, "path": path, "cells": {}}));
        row["cells"][job] = cell;
    }
    // Keep the run's paper order.
    let order: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let mut rows: Vec<Value> = rows.into_values().collect();
    rows.sort_by_key(|r| {
        order
            .iter()
            .position(|p| r["path"].as_str() == Some(p))
            .unwrap_or(usize::MAX)
    });
    let total_cost: f64 = rows
        .iter()
        .flat_map(|r| r["cells"].as_object().into_iter().flatten())
        .filter_map(|(_, c)| c["cost_usd"].as_f64())
        .sum();
    Ok(json!({"run": run, "columns": columns, "rows": rows, "total_cost_usd": total_cost}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// A forge on the mock provider with two jobs, `a` and `b`, and `n` papers in INBOX.
    fn setup(n: usize) -> (tempfile::TempDir, Arc<Forge>, Vec<PathBuf>) {
        use crate::config::{DEFAULT_PROVIDERS, DEFAULT_SETTINGS};
        let dir = tempfile::tempdir().unwrap();
        let tpl = dir.path().join("templates");
        std::fs::create_dir_all(&tpl).unwrap();
        for j in ["a", "b"] {
            std::fs::write(
                tpl.join(format!("{j}.md")),
                format!("---\nsuffix: {j}\n---\n{{{{ document }}}}"),
            )
            .unwrap();
        }
        let mut config = Config::from_strings(
            DEFAULT_PROVIDERS,
            DEFAULT_SETTINGS,
            dir.path().to_path_buf(),
            None,
        )
        .unwrap();
        config.settings.default_route = Some("test".into());
        let forge = Forge::with_store(config, Store::open_memory().unwrap()).unwrap();
        let inbox = dir.path().join("INBOX");
        std::fs::create_dir_all(&inbox).unwrap();
        let paths = (0..n)
            .map(|i| {
                let p = inbox.join(format!("p{i}.md"));
                std::fs::write(&p, format!("# Paper {i}\ntext")).unwrap();
                p
            })
            .collect();
        (dir, forge, paths)
    }

    fn spec(dir: &tempfile::TempDir) -> RunSpec {
        RunSpec {
            stages: vec![vec!["a".into()], vec!["b".into()]],
            outbox: dir.path().join("OUTBOX"),
            route: None,
            redo: false,
            workers: 2,
        }
    }

    fn statuses(g: &Value, row: usize) -> (String, String) {
        let c = &g["rows"][row]["cells"];
        (
            c["a"]["status"].as_str().unwrap().into(),
            c["b"]["status"].as_str().unwrap().into(),
        )
    }

    #[tokio::test]
    async fn first_then_approve_stops_after_one_paper_then_finishes() {
        let (dir, forge, paths) = setup(3);
        let run = create(&forge, Mode::FirstThenApprove, spec(&dir), paths).unwrap();
        let after = execute(&forge, &run.id).await.unwrap();
        assert_eq!(
            (after.status.as_str(), after.cursor),
            ("awaiting_approval", 1)
        );
        let g = grid(&forge.store, Some(&run.id)).unwrap();
        assert_eq!(statuses(&g, 0), ("ok".into(), "ok".into()));
        assert_eq!(statuses(&g, 1).0, "awaiting_approval");
        assert_eq!(statuses(&g, 2).0, "pending");
        assert!(g["rows"][0]["uid"].as_str().unwrap().len() == 26);

        assert!(
            resume(&forge.store, &run.id).is_err(),
            "the gate needs approve, not resume"
        );
        approve(&forge.store, &run.id).unwrap();
        let done = execute(&forge, &run.id).await.unwrap();
        assert_eq!((done.status.as_str(), done.cursor), ("done", 3));
    }

    #[tokio::test]
    async fn step_mode_gates_every_paper_and_pause_holds() {
        let (dir, forge, paths) = setup(2);
        let run = create(&forge, Mode::Step, spec(&dir), paths).unwrap();
        assert_eq!(
            execute(&forge, &run.id).await.unwrap().status,
            "awaiting_approval"
        );
        pause(&forge.store, &run.id).unwrap();
        assert_eq!(
            execute(&forge, &run.id).await.unwrap().cursor,
            1,
            "paused run must not move"
        );
        resume(&forge.store, &run.id).unwrap();
        // Resumed with the gate still unpassed for step mode: runs one paper, then finishes.
        let r = execute(&forge, &run.id).await.unwrap();
        assert_eq!((r.status.as_str(), r.cursor), ("done", 2));
    }

    #[tokio::test]
    async fn failed_station_blocks_later_stages() {
        let (dir, forge, paths) = setup(1);
        let mut sp = spec(&dir);
        sp.stages = vec![vec!["a".into(), "missing".into()], vec!["b".into()]];
        assert!(
            create(&forge, Mode::All, sp.clone(), paths.clone()).is_err(),
            "unknown job rejected up front"
        );
        // Make the template exist at create time, then break it so the station fails at run time.
        let tpl = dir.path().join("templates").join("missing.md");
        std::fs::write(&tpl, "---\nsuffix: m\n---\nx").unwrap();
        let run = create(&forge, Mode::All, sp, paths).unwrap();
        std::fs::remove_file(&tpl).unwrap();
        let r = execute(&forge, &run.id).await.unwrap();
        assert_eq!(r.status, "done");
        let g = grid(&forge.store, None).unwrap();
        let c = &g["rows"][0]["cells"];
        assert_eq!(c["a"]["status"], "ok");
        assert_eq!(c["missing"]["status"], "failed");
        assert_eq!(c["b"]["status"], "blocked");
    }

    #[tokio::test]
    async fn restart_parks_running_runs_and_resume_skips_finished_work() {
        let (dir, forge, paths) = setup(2);
        let run = create(&forge, Mode::All, spec(&dir), paths).unwrap();
        // Simulate a crash: status left 'running', nothing executed.
        assert_eq!(park_orphaned(&forge.store).unwrap(), 1);
        assert_eq!(get(&forge.store, &run.id).unwrap().status, "paused");
        resume(&forge.store, &run.id).unwrap();
        assert_eq!(execute(&forge, &run.id).await.unwrap().status, "done");
        // A second run over the same papers: every cell is skipped as unchanged.
        let again = create(
            &forge,
            Mode::All,
            spec(&dir),
            vec![dir.path().join("INBOX/p0.md")],
        )
        .unwrap();
        execute(&forge, &again.id).await.unwrap();
        let g = grid(&forge.store, Some(&again.id)).unwrap();
        assert_eq!(statuses(&g, 0), ("skipped".into(), "skipped".into()));
    }
}
