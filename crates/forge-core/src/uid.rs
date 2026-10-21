//! Permanent paper ids. A content hash changes when a comma does; a `uid` doesn't. It lives in the
//! paper's front matter as `forge_uid:` and in the `papers` table, so a rename or a move keeps it.
//!
//! Resolving a paper without a front matter id, in order: same path -> that uid; else the same content
//! hash as a known version -> that uid; else mint a new one. Anything but a front matter hit is written
//! to `uid_orphans` for a human to look at. There is no fuzzy matching: a guess would merge two papers.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::store::{now, Store};

pub const KEY: &str = "forge_uid";

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A ULID: 48-bit millisecond time + 80 random bits, 26 Crockford base32 characters. Sorts by creation.
pub fn mint() -> String {
    let ms = chrono::Utc::now().timestamp_millis().max(0) as u128;
    let rand = ((fastrand::u64(..) as u128) << 16) | fastrand::u16(..) as u128;
    let mut n = (ms << 80) | rand;
    let mut out = [0u8; 26];
    for c in out.iter_mut().rev() {
        *c = CROCKFORD[(n & 31) as usize];
        n >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("ascii")
}

fn front_matter_end(text: &str) -> Option<usize> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let start = text.len() - rest.len();
    let mut pos = start;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some(pos);
        }
        pos += line.len();
    }
    None
}

/// The `forge_uid:` value from the front matter, if there is one.
pub fn read(text: &str) -> Option<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let end = front_matter_end(text)?;
    text[..end].lines().find_map(|l| {
        let v = l.strip_prefix(KEY)?.trim_start().strip_prefix(':')?;
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// The text with `forge_uid: <uid>` in its front matter (added, or a front matter block created).
pub fn write(text: &str, uid: &str) -> String {
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(b) => ("\u{feff}", b),
        None => ("", text),
    };
    let line = format!("{KEY}: {uid}\n");
    if let Some(end) = front_matter_end(body) {
        let fm: String = body[..end]
            .split_inclusive('\n')
            .filter(|l| !l.starts_with(KEY))
            .collect();
        return format!("{bom}{fm}{line}{}", &body[end..]);
    }
    format!("{bom}---\n{line}---\n\n{body}")
}

/// The text without its `forge_uid:` line: what content hashes are taken over, so adding the id
/// doesn't make a paper look edited.
pub fn strip(text: &str) -> String {
    let Some(end) = front_matter_end(text.strip_prefix('\u{feff}').unwrap_or(text)) else {
        return text.to_string();
    };
    let off = text.len() - text.strip_prefix('\u{feff}').unwrap_or(text).len();
    let fm: String = text[..off + end]
        .split_inclusive('\n')
        .filter(|l| !l.starts_with(KEY))
        .collect();
    let rest = &text[off + end..];
    // A block forge created only to hold the id: drop it entirely, so the hash matches the original.
    if fm[off..].trim() == "---" {
        if let Some(after) = rest
            .strip_prefix("---\n\n")
            .or_else(|| rest.strip_prefix("---\n"))
        {
            return format!("{}{after}", &fm[..off]);
        }
    }
    format!("{fm}{rest}")
}

pub fn content_sha(text: &str) -> String {
    hex::encode(Sha256::digest(strip(text).as_bytes()))
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub uid: String,
    pub path: String,
    pub sha256: String,
    /// front_matter | path | content | minted
    pub how: &'static str,
    /// The file needs `forge_uid:` written into it.
    pub needs_write: bool,
}

/// Work out a paper's uid without touching the file or the ledger.
pub fn lookup(store: &Store, path: &Path, text: &str) -> Result<Resolved> {
    let p = path.display().to_string();
    let sha = content_sha(text);
    let mk = |uid: String, how, needs_write| Resolved {
        uid,
        path: p.clone(),
        sha256: sha.clone(),
        how,
        needs_write,
    };
    if let Some(u) = read(text) {
        return Ok(mk(u, "front_matter", false));
    }
    let db = store.lock();
    if let Some(u) = db
        .query_row("SELECT uid FROM papers WHERE path = ?1", params![p], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    {
        return Ok(mk(u, "path", true));
    }
    let by_sha: Vec<String> = db
        .prepare("SELECT DISTINCT uid FROM paper_versions WHERE sha256 = ?1")?
        .query_map(params![sha], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    // Two papers with identical text: ambiguous, so it gets its own id instead of a coin flip.
    if by_sha.len() == 1 {
        return Ok(mk(by_sha[0].clone(), "content", true));
    }
    Ok(mk(mint(), "minted", true))
}

/// Record a resolved paper in the ledger (path, version, orphan report).
pub fn register(store: &Store, r: &Resolved) -> Result<()> {
    let t = now();
    let db = store.lock();
    db.execute(
        "INSERT INTO papers (uid, path, current_sha, created_at, updated_at) VALUES (?1,?2,?3,?4,?4)
         ON CONFLICT(uid) DO UPDATE SET path=excluded.path, current_sha=excluded.current_sha, updated_at=excluded.updated_at",
        params![r.uid, r.path, r.sha256, t],
    )?;
    db.execute(
        "INSERT OR IGNORE INTO paper_versions (uid, sha256, path, seen_at) VALUES (?1,?2,?3,?4)",
        params![r.uid, r.sha256, r.path, t],
    )?;
    if r.how != "front_matter" {
        let reason = match r.how {
            "path" => "no forge_uid in front matter; reattached by path",
            "content" => "no forge_uid in front matter; reattached by identical content",
            _ => "no forge_uid in front matter and path/content unknown; new uid minted",
        };
        db.execute(
            "INSERT INTO uid_orphans (at, path, sha256, uid, reason) VALUES (?1,?2,?3,?4,?5)",
            params![t, r.path, r.sha256, r.uid, reason],
        )?;
    }
    Ok(())
}

/// Resolve, record, and (if `write_back`) put the id into the file. Returns the uid.
pub fn ensure(store: &Store, path: &Path, write_back: bool) -> Result<Resolved> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let r = lookup(store, path, &text)?;
    if write_back {
        if r.needs_write {
            std::fs::write(path, write(&text, &r.uid))
                .with_context(|| format!("writing {KEY} into {}", path.display()))?;
        }
        register(store, &r)?;
    }
    Ok(r)
}

pub fn orphans(store: &Store, limit: u32) -> Result<Vec<serde_json::Value>> {
    let db = store.lock();
    let mut st = db.prepare(
        "SELECT at, path, sha256, uid, reason FROM uid_orphans ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = st.query_map(params![limit], |r| {
        Ok(serde_json::json!({"at": r.get::<_, String>(0)?, "path": r.get::<_, String>(1)?,
            "sha256": r.get::<_, String>(2)?, "uid": r.get::<_, String>(3)?, "reason": r.get::<_, String>(4)?}))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Every version a uid has had, oldest first.
pub fn versions(store: &Store, uid: &str) -> Result<Vec<serde_json::Value>> {
    let db = store.lock();
    let mut st = db.prepare(
        "SELECT sha256, path, seen_at FROM paper_versions WHERE uid = ?1 ORDER BY seen_at",
    )?;
    let rows = st.query_map(params![uid], |r| {
        Ok(serde_json::json!({"sha256": r.get::<_, String>(0)?, "path": r.get::<_, String>(1)?, "seen_at": r.get::<_, String>(2)?}))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_is_ulid_shaped_and_unique() {
        let a = mint();
        let b = mint();
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| CROCKFORD.contains(&c)));
    }

    #[test]
    fn front_matter_round_trip_keeps_other_fields_and_hash() {
        let plain = "# Paper\n\nbody";
        let w = write(plain, "01ABC");
        assert_eq!(read(&w).as_deref(), Some("01ABC"));
        assert_eq!(content_sha(&w), content_sha(&write(plain, "01XYZ")));
        assert_eq!(content_sha(&w), content_sha(plain));

        let fm = "---\ntitle: X\nforge_uid: OLD\n---\nbody\n";
        let w = write(fm, "NEW");
        assert_eq!(read(&w).as_deref(), Some("NEW"));
        assert!(w.contains("title: X") && !w.contains("OLD"));
        assert_eq!(strip(&w), "---\ntitle: X\n---\nbody\n");
        assert_eq!(read("no front matter"), None);
    }

    #[test]
    fn resolve_order_path_then_content_then_mint() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_memory().unwrap();
        let a = dir.path().join("a.md");
        std::fs::write(&a, "# A\nalpha").unwrap();
        let first = ensure(&s, &a, true).unwrap();
        assert_eq!(first.how, "minted");
        let on_disk = std::fs::read_to_string(&a).unwrap();
        assert_eq!(read(&on_disk).as_deref(), Some(first.uid.as_str()));
        // Second sight: front matter wins, no orphan row.
        assert_eq!(ensure(&s, &a, true).unwrap().how, "front_matter");

        // An AI rewrote the file and dropped the front matter: same path -> same uid.
        std::fs::write(&a, "# A\nalpha, edited").unwrap();
        let r = ensure(&s, &a, true).unwrap();
        assert_eq!((r.how, r.uid.as_str()), ("path", first.uid.as_str()));

        // Moved and stripped, content unchanged: identical content -> same uid.
        let b = dir.path().join("moved.md");
        std::fs::write(&b, "# A\nalpha, edited").unwrap();
        let r = ensure(&s, &b, true).unwrap();
        assert_eq!((r.how, r.uid.as_str()), ("content", first.uid.as_str()));

        // Unknown path and content: a new uid, never a guess.
        let c = dir.path().join("c.md");
        std::fs::write(&c, "# A\nalpha, edited twice").unwrap();
        let r = ensure(&s, &c, true).unwrap();
        assert_eq!(r.how, "minted");
        assert_ne!(r.uid, first.uid);
        assert_eq!(orphans(&s, 10).unwrap().len(), 4);
        assert!(versions(&s, &first.uid).unwrap().len() >= 2);
    }

    #[test]
    fn dry_lookup_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_memory().unwrap();
        let a = dir.path().join("a.md");
        std::fs::write(&a, "body").unwrap();
        ensure(&s, &a, false).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "body");
        assert!(orphans(&s, 10).unwrap().is_empty());
    }
}
