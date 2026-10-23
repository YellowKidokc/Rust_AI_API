//! The API registry: every API folder forge shows on its home page and in the tray, in one list.
//!
//! `<home>/config/apis.json` holds `{"sources": [{"name", "group", "path"}]}`. A source is whatever its path is:
//!
//! * a pipeline (`<path>/_system/engine/console.py`, or `<path>/engine/console.py`): its stations come from its own
//!   `console.py list`, unchanged (the one station feed; nothing is re-derived here)
//! * a folder of stations: every folder under it (3 levels) that holds a `.bat` or `.html` is a station, those files
//!   are its buttons (Atoms, Axioms, API 2, an old copy ...)
//! * an `.html` page (Atom Builder, Reconciliation ...): served at `/pages/{index}`
//! * an `http(s)://` link
//!
//! The first pipeline found (`PIPELINE_SYSTEM`, then forge's own base when forge was started inside `API\`, then the
//! registry in order) is the main one: its stations keep their families. Every other source's stations are grouped
//! under that source's `group`. Nothing here moves, renames or writes anything but `apis.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub name: String,
    #[serde(default)]
    pub group: String,
    pub path: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub sources: Vec<Source>,
}

pub fn registry_path(base: &Path) -> PathBuf {
    base.join("config").join("apis.json")
}

pub fn load(base: &Path) -> anyhow::Result<Registry> {
    match std::fs::read_to_string(registry_path(base)) {
        // A BOM (Windows PowerShell 5 writes one) is not JSON; skip it.
        Ok(s) => Ok(serde_json::from_str(s.trim_start_matches('\u{feff}'))
            .map_err(|e| anyhow::anyhow!("{}: {e}", registry_path(base).display()))?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn save(base: &Path, reg: &Registry) -> anyhow::Result<()> {
    let p = registry_path(base);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(reg)? + "\n")?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Pipeline,
    Folder,
    Page,
    Link,
    Missing,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Pipeline => "pipeline",
            Kind::Folder => "folder",
            Kind::Page => "page",
            Kind::Link => "link",
            Kind::Missing => "missing",
        }
    }
}

/// The `console.py` of a pipeline at `path` (the `API` folder or its `_system`).
pub fn pipeline_console(path: &Path) -> Option<PathBuf> {
    [
        path.join("_system").join("engine").join("console.py"),
        path.join("engine").join("console.py"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

pub fn kind(path: &str) -> Kind {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Kind::Link;
    }
    let p = Path::new(path);
    let lower = path.to_ascii_lowercase();
    if p.is_file() && (lower.ends_with(".html") || lower.ends_with(".htm")) {
        Kind::Page
    } else if p.is_dir() && pipeline_console(p).is_some() {
        Kind::Pipeline
    } else if p.is_dir() {
        Kind::Folder
    } else {
        Kind::Missing
    }
}

/// Station folders under `root`: any folder (root included, 3 levels down) holding a `.bat` or `.html`. Work folders
/// (INBOX, OUTBOX, BACKSIDE ...) are not searched.
pub fn scan_folder(root: &Path) -> Vec<Value> {
    const SKIP: &[&str] = &[
        "backside",
        "inbox",
        "outbox",
        "_system",
        "logs",
        "state",
        ".git",
        "node_modules",
        "target",
        "__pycache__",
        ".venv",
    ];
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut buttons = Vec::new();
        let mut subdirs = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !SKIP.contains(&name.to_ascii_lowercase().as_str()) {
                    subdirs.push(p);
                }
            } else {
                let l = name.to_ascii_lowercase();
                if l.ends_with(".bat") || l.ends_with(".html") || l.ends_with(".htm") {
                    let stem = Path::new(&name)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or(name.clone());
                    buttons.push(json!({"name": stem, "path": p.display().to_string()}));
                }
            }
        }
        if !buttons.is_empty() {
            buttons.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            let label = dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| dir.display().to_string());
            let rel = dir
                .strip_prefix(root)
                .map(|r| r.display().to_string())
                .unwrap_or_default();
            out.push(json!({"label": label, "family": "", "path": if rel.is_empty() { label.clone() } else { rel },
                            "front": dir.display().to_string(), "buttons": buttons}));
            if out.len() >= 500 {
                break;
            }
        }
        if depth < 3 {
            subdirs.sort();
            stack.extend(subdirs.into_iter().rev().map(|d| (d, depth + 1)));
        }
    }
    out.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    out
}

pub fn python_command() -> (String, Vec<String>) {
    if let Some(p) = std::env::var_os("PYTHON") {
        return (p.to_string_lossy().into_owned(), vec![]);
    }
    if cfg!(windows) {
        ("py".into(), vec!["-3".into()])
    } else {
        ("python3".into(), vec![])
    }
}

/// `console.py list` of one pipeline: its `stations` rows, or the reason it could not be read.
pub async fn console_list(console: &Path) -> Result<Vec<Value>, String> {
    let (program, mut args) = python_command();
    args.push(console.display().to_string());
    args.push("list".into());
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args);
    if let Some(sys) = console.parent().and_then(|p| p.parent()) {
        cmd.current_dir(sys);
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| "console.py list took over 60 s".to_string())?
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "console.py list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let feed: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("console.py list is not JSON: {e}"))?;
    Ok(feed
        .get("stations")
        .unwrap_or(&feed)
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// One source as the home page shows it.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub index: Option<usize>,
    pub source: Source,
    pub kind: Kind,
    pub main: bool,
    pub stations: Vec<Value>,
    pub error: Option<String>,
}

impl Resolved {
    pub fn summary(&self) -> Value {
        let mut v = json!({"name": self.source.name, "group": self.source.group, "path": self.source.path,
                           "kind": self.kind.as_str(), "main": self.main, "stations": self.stations.len(),
                           "registered": self.index.is_some()});
        if let Some(i) = self.index.filter(|_| self.kind == Kind::Page) {
            v["url"] = json!(format!("/pages/{i}"));
        }
        if self.kind == Kind::Link {
            v["url"] = json!(self.source.path);
        }
        if let Some(e) = &self.error {
            v["error"] = json!(e);
        }
        v
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Every source with its stations: the implicit pipeline (PIPELINE_SYSTEM / forge's base) first, then the registry.
pub async fn resolve(base: &Path) -> (Vec<Resolved>, Option<String>) {
    let (reg, reg_error) = match load(base) {
        Ok(r) => (r, None),
        Err(e) => (Registry::default(), Some(format!("{e:#}"))),
    };
    let mut out: Vec<Resolved> = Vec::new();
    let mut consoles: Vec<PathBuf> = Vec::new();
    let implicit = std::env::var_os("PIPELINE_SYSTEM")
        .map(PathBuf::from)
        .into_iter()
        .chain(std::iter::once(base.to_path_buf()));
    for sys in implicit {
        if let Some(c) = pipeline_console(&sys) {
            if consoles.iter().any(|k| same_file(k, &c)) {
                continue;
            }
            consoles.push(c);
            out.push(Resolved {
                index: None,
                source: Source {
                    name: "Pipeline".into(),
                    group: "Pipeline".into(),
                    path: sys.display().to_string(),
                },
                kind: Kind::Pipeline,
                main: false,
                stations: vec![],
                error: None,
            });
        }
    }
    for (i, s) in reg.sources.iter().enumerate() {
        let k = kind(&s.path);
        if k == Kind::Pipeline {
            let c = pipeline_console(Path::new(&s.path)).unwrap_or_default();
            if let Some(r) = out.iter_mut().find(|r| {
                r.kind == Kind::Pipeline
                    && pipeline_console(Path::new(&r.source.path))
                        .is_some_and(|rc| same_file(&rc, &c))
            }) {
                // Registered and also found by PIPELINE_SYSTEM: one source, under its registered name.
                r.index = Some(i);
                r.source = s.clone();
                continue;
            }
            consoles.push(c);
        }
        out.push(Resolved {
            index: Some(i),
            source: s.clone(),
            kind: k,
            main: false,
            stations: vec![],
            error: (k == Kind::Missing).then(|| format!("not found: {}", s.path)),
        });
    }
    let mut main_taken = false;
    for r in out.iter_mut() {
        let group = if r.source.group.trim().is_empty() {
            r.source.name.clone()
        } else {
            r.source.group.clone()
        };
        let rows = match r.kind {
            Kind::Pipeline => {
                match console_list(&pipeline_console(Path::new(&r.source.path)).unwrap_or_default())
                    .await
                {
                    Ok(rows) => rows,
                    Err(e) => {
                        r.error = Some(e);
                        continue;
                    }
                }
            }
            Kind::Folder => scan_folder(Path::new(&r.source.path)),
            _ => continue,
        };
        let main = r.kind == Kind::Pipeline && !main_taken;
        main_taken |= main;
        r.main = main;
        r.stations = rows
            .into_iter()
            .map(|mut row| {
                row["source"] = json!(r.source.name);
                if !main {
                    row["group"] = json!(group);
                }
                row
            })
            .collect();
    }
    (out, reg_error)
}

/// Add a source (validated: it must exist). Same path again replaces the old entry's name and group.
pub fn add(base: &Path, mut s: Source) -> Result<Source, String> {
    s.path = s.path.trim().trim_matches('"').to_string();
    if s.path.is_empty() {
        return Err("path is empty".into());
    }
    if kind(&s.path) == Kind::Missing {
        return Err(format!(
            "not found (or not a folder, .html page or http link): {}",
            s.path
        ));
    }
    if s.name.trim().is_empty() {
        s.name = Path::new(&s.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| s.path.clone());
    }
    let mut reg = load(base).map_err(|e| format!("{e:#}"))?;
    match reg.sources.iter_mut().find(|x| x.path == s.path) {
        Some(x) => *x = s.clone(),
        None => reg.sources.push(s.clone()),
    }
    save(base, &reg).map_err(|e| format!("{e:#}"))?;
    Ok(s)
}

pub fn remove(base: &Path, path: &str) -> Result<bool, String> {
    let mut reg = load(base).map_err(|e| format!("{e:#}"))?;
    let before = reg.sources.len();
    reg.sources.retain(|x| x.path != path);
    let removed = reg.sources.len() != before;
    if removed {
        save(base, &reg).map_err(|e| format!("{e:#}"))?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_scan_finds_bat_and_html_and_skips_work_folders() {
        let d = tempfile::tempdir().unwrap();
        let st = d.path().join("ATOMS").join("201_ATOM_BUILDER");
        std::fs::create_dir_all(st.join("BACKSIDE")).unwrap();
        std::fs::create_dir_all(st.join("INBOX")).unwrap();
        std::fs::write(st.join("1 RUN HERE.bat"), "").unwrap();
        std::fs::write(st.join("atom-builder.html"), "").unwrap();
        std::fs::write(st.join("notes.md"), "").unwrap();
        std::fs::write(st.join("BACKSIDE").join("hidden.bat"), "").unwrap();
        std::fs::write(st.join("INBOX").join("x.bat"), "").unwrap();
        let rows = scan_folder(d.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["label"], "201_ATOM_BUILDER");
        let names: Vec<_> = rows[0]["buttons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["1 RUN HERE", "atom-builder"]);
    }

    #[test]
    fn add_validates_and_replaces_by_path() {
        let d = tempfile::tempdir().unwrap();
        let api = d.path().join("API 2");
        std::fs::create_dir_all(&api).unwrap();
        assert!(add(
            d.path(),
            Source {
                name: "".into(),
                group: "".into(),
                path: d.path().join("nope").display().to_string()
            }
        )
        .is_err());
        let s = add(
            d.path(),
            Source {
                name: "".into(),
                group: "Old APIs".into(),
                path: api.display().to_string(),
            },
        )
        .unwrap();
        assert_eq!(s.name, "API 2");
        add(
            d.path(),
            Source {
                name: "API two".into(),
                group: "Old APIs".into(),
                path: api.display().to_string(),
            },
        )
        .unwrap();
        let reg = load(d.path()).unwrap();
        assert_eq!(reg.sources.len(), 1);
        assert_eq!(reg.sources[0].name, "API two");
        assert!(remove(d.path(), &api.display().to_string()).unwrap());
        assert!(load(d.path()).unwrap().sources.is_empty());
    }

    #[test]
    fn kinds() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("API/_system/engine")).unwrap();
        std::fs::write(d.path().join("API/_system/engine/console.py"), "").unwrap();
        std::fs::write(d.path().join("page.html"), "").unwrap();
        let s = |p: &str| d.path().join(p).display().to_string();
        assert_eq!(kind(&s("API")), Kind::Pipeline);
        assert_eq!(kind(&s("API/_system")), Kind::Pipeline);
        assert_eq!(kind(&s("page.html")), Kind::Page);
        assert_eq!(kind(&s("")), Kind::Folder);
        assert_eq!(kind(&s("gone")), Kind::Missing);
        assert_eq!(kind("https://example.com"), Kind::Link);
    }
}
