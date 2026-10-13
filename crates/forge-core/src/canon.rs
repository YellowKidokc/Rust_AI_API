//! Candidate graph: the `/candidate`, `/search`, `/agenda` contract the Canon Workbench pages
//! (reconciliation.html, the atom builder) already speak. Staging is never admission.
//!
//! Two deliberate differences from the engine those pages were written against:
//! * unknown top-level fields are **rejected** (422 with the field named), never silently dropped;
//! * staging the exact same content twice returns the first record (`deduplicated: true`) instead of
//!   minting a second id, so a retry after a timeout is safe.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::store::{now, Store};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateInput {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub atom: Option<Value>,
    #[serde(default)]
    pub atom_id: Option<String>,
    #[serde(default)]
    pub schema_version: Option<String>,
    #[serde(default)]
    pub source_uri: Option<String>,
    #[serde(default)]
    pub source_span: Option<Value>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reason {
    pub check_id: String,
    pub trigger: String,
    pub smallest_fix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub candidate_id: String,
    pub content_hash: String,
    pub status: String,
    pub verdict: String,
    pub receipt_id: String,
    pub conflicts_opened: Vec<String>,
    pub reasons: Vec<Reason>,
    pub deduplicated: bool,
    pub note: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CanonError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}

/// SHA-256 over the canonical JSON of `{text, atom}`. serde_json's default map is sorted, so key order
/// in the request doesn't change the hash. `generated_at` is excluded so a re-export of the same
/// proposal hashes the same.
pub fn content_hash(text: Option<&str>, atom: Option<&Value>) -> String {
    let mut atom = atom.cloned().unwrap_or(Value::Null);
    if let Some(o) = atom.as_object_mut() {
        o.remove("generated_at");
    }
    let canon = canonical_json(&json!({"text": text.unwrap_or(""), "atom": atom}));
    hex::encode(Sha256::digest(canon.as_bytes()))
}

/// JSON with object keys sorted at every level, written by hand so the result doesn't depend on
/// whether serde_json's `preserve_order` feature is on (another crate in the workspace may enable it).
pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical_json(&o[*k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// Integrity checks that never block staging; they become reasons on the receipt.
pub fn checks(input: &CandidateInput) -> Vec<Reason> {
    let mut out = Vec::new();
    if let Some(atom) = &input.atom {
        if atom
            .get("object_type")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            out.push(Reason {
                check_id: "OBJECT_TYPE_MISSING".into(),
                trigger: "atom has no object_type".into(),
                smallest_fix: "Set exactly one primary object_type; if the content does two jobs, decompose it.".into(),
            });
        }
        walk_lean(atom, &mut out);
    }
    if input.author.as_deref().is_none_or(str::is_empty) {
        out.push(Reason {
            check_id: "AUTHOR_MISSING".into(),
            trigger: "no author on the candidate".into(),
            smallest_fix: "Set author so the ruling can be attributed.".into(),
        });
    }
    out
}

/// A build state (BUILT / ZERO_SORRY) is a receipt claim: flag any that arrives without a receipt id.
fn walk_lean(v: &Value, out: &mut Vec<Reason>) {
    match v {
        Value::Object(o) => {
            let claimed = o
                .get("lean_state")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    (o.get("axis").and_then(Value::as_str) == Some("lean_state")).then(|| {
                        o.get("proposed")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string()
                    })
                });
            if let Some(state) = claimed {
                let up = state.to_uppercase();
                let has_receipt = ["receipt_id", "lean_receipt"].iter().any(|k| {
                    o.get(*k)
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
                });
                if (up.contains("BUILT") || up.contains("ZERO_SORRY")) && !has_receipt {
                    out.push(Reason {
                        check_id: "LEAN_RECEIPT_MISSING".into(),
                        trigger: format!("lean_state '{state}' claimed without a receipt id"),
                        smallest_fix:
                            "Attach the build receipt id, or record the state as a specification."
                                .into(),
                    });
                }
            }
            o.values().for_each(|x| walk_lean(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| walk_lean(x, out)),
        _ => {}
    }
}

pub fn stage(store: &Store, input: CandidateInput) -> Result<Receipt, CanonError> {
    let has_text = input.text.as_deref().is_some_and(|t| !t.trim().is_empty());
    if !has_text && input.atom.is_none() {
        return Err(CanonError::Invalid(
            "a candidate needs `text` or `atom` (or both)".into(),
        ));
    }
    let hash = content_hash(input.text.as_deref(), input.atom.as_ref());
    let db = store.lock();

    let existing: Option<(String, String, String, String, String)> = db
        .query_row(
            "SELECT candidate_id, status, verdict, receipt_id, reasons FROM candidates WHERE content_hash = ?1 LIMIT 1",
            params![hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    if let Some((candidate_id, status, verdict, receipt_id, reasons)) = existing {
        let reasons: Vec<Value> = serde_json::from_str(&reasons).unwrap_or_default();
        return Ok(Receipt {
            candidate_id,
            content_hash: hash,
            status,
            verdict,
            receipt_id,
            conflicts_opened: vec![],
            reasons: reasons
                .into_iter()
                .map(|r| Reason {
                    check_id: r["check_id"].as_str().unwrap_or("").into(),
                    trigger: r["trigger"].as_str().unwrap_or("").into(),
                    smallest_fix: r["smallest_fix"].as_str().unwrap_or("").into(),
                })
                .collect(),
            deduplicated: true,
            note: "identical content was already staged; returning that record. Nothing new was written.".into(),
        });
    }

    let reasons = checks(&input);
    let verdict = if reasons.is_empty() {
        "PASS"
    } else {
        "NEEDS_REVIEW"
    };
    let candidate_id = format!(
        "CAN-{}-{}",
        &hash[..10],
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let receipt_id = format!("RCPT-{}", uuid::Uuid::new_v4().simple());
    let t = now();

    // Another candidate for the same atom with different content: open a conflict for a human ruling.
    let mut conflicts_opened = Vec::new();
    if let Some(atom_id) = input.atom_id.as_deref().filter(|s| !s.is_empty()) {
        let prior: Vec<String> = {
            let mut st =
                db.prepare("SELECT candidate_id FROM candidates WHERE atom_id = ?1 AND status = 'candidate' ORDER BY created_at")?;
            let rows = st.query_map(params![atom_id], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        if !prior.is_empty() {
            let conflict_id = format!("CONF-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
            let mut affected: Vec<String> = vec![atom_id.to_string()];
            affected.extend(input.depends_on.iter().cloned());
            affected.extend(prior.iter().cloned());
            affected.sort();
            affected.dedup();
            db.execute(
                "INSERT INTO conflicts (conflict_id, kind, left_id, right_id, affected, reason, status, created_at)
                 VALUES (?1, 'conflict', ?2, ?3, ?4, ?5, 'open', ?6)",
                params![
                    conflict_id,
                    atom_id,
                    candidate_id,
                    json!(affected).to_string(),
                    format!("{} open candidate(s) already exist for {atom_id}", prior.len()),
                    t
                ],
            )?;
            conflicts_opened.push(conflict_id);
        }
    }

    db.execute(
        "INSERT INTO candidates (candidate_id, atom_id, content_hash, status, verdict, receipt_id, author, source_uri,
            source_span, depends_on, note, text, payload, schema_version, reasons, created_at)
         VALUES (?1,?2,?3,'candidate',?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            candidate_id,
            input.atom_id,
            hash,
            verdict,
            receipt_id,
            input.author,
            input.source_uri,
            input.source_span.as_ref().map(Value::to_string),
            json!(input.depends_on).to_string(),
            input.note,
            input.text,
            input.atom.as_ref().map(Value::to_string),
            input.schema_version,
            serde_json::to_string(&reasons).unwrap_or_else(|_| "[]".into()),
            t
        ],
    )?;

    Ok(Receipt {
        candidate_id,
        content_hash: hash,
        status: "candidate".into(),
        verdict: verdict.into(),
        receipt_id,
        conflicts_opened,
        reasons,
        deduplicated: false,
        note: "accepted and staged. this is not canon.".into(),
    })
}

/// Exact id matches (atom or candidate) first, newest first; then substring matches in text/payload.
pub fn search(store: &Store, q: &str, limit: u32) -> Result<Vec<Value>, CanonError> {
    let db = store.lock();
    let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
    let mut st = db.prepare(
        "SELECT candidate_id, atom_id, content_hash, source_uri, status, verdict, payload, text, created_at,
                CASE WHEN atom_id = ?1 OR candidate_id = ?1 THEN 0 ELSE 1 END AS rank
         FROM candidates
         WHERE atom_id = ?1 OR candidate_id = ?1 OR text LIKE ?2 ESCAPE '\' OR payload LIKE ?2 ESCAPE '\'
         ORDER BY rank, created_at DESC LIMIT ?3",
    )?;
    let rows = st.query_map(params![q, like, limit], |r| {
        let payload: Option<String> = r.get(6)?;
        Ok(json!({
            "candidate_id": r.get::<_, String>(0)?,
            "atom_id": r.get::<_, Option<String>>(1)?,
            "content_hash": r.get::<_, String>(2)?,
            "source_uri": r.get::<_, Option<String>>(3)?,
            "status": r.get::<_, String>(4)?,
            "verdict": r.get::<_, String>(5)?,
            "payload": payload.and_then(|p| serde_json::from_str::<Value>(&p).ok()).unwrap_or(Value::Null),
            "text": r.get::<_, Option<String>>(7)?,
            "created_at": r.get::<_, String>(8)?,
        }))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What needs a human: open conflicts, then candidates whose checks raised reasons.
pub fn agenda(store: &Store, limit: u32) -> Result<Vec<Value>, CanonError> {
    let db = store.lock();
    let mut items = Vec::new();
    let mut st = db.prepare(
        "SELECT conflict_id, left_id, right_id, affected, reason, created_at FROM conflicts
         WHERE status = 'open' ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = st.query_map(params![limit], |r| {
        let affected: String = r.get(3)?;
        Ok(json!({
            "kind": "conflict", "conflict_id": r.get::<_, String>(0)?, "left": r.get::<_, String>(1)?,
            "right": r.get::<_, String>(2)?, "affected": serde_json::from_str::<Value>(&affected).unwrap_or(json!([])),
            "reason": r.get::<_, Option<String>>(4)?, "created_at": r.get::<_, String>(5)?,
        }))
    })?;
    for r in rows {
        items.push(r?);
    }
    let mut st = db.prepare(
        "SELECT candidate_id, atom_id, verdict, reasons, created_at FROM candidates
         WHERE status = 'candidate' AND verdict <> 'PASS' ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = st.query_map(params![limit], |r| {
        let reasons: String = r.get(3)?;
        Ok(json!({
            "kind": "review", "candidate_id": r.get::<_, String>(0)?, "atom_id": r.get::<_, Option<String>>(1)?,
            "verdict": r.get::<_, String>(2)?, "reasons": serde_json::from_str::<Value>(&reasons).unwrap_or(json!([])),
            "created_at": r.get::<_, String>(4)?,
        }))
    })?;
    for r in rows {
        items.push(r?);
    }
    items.truncate(limit as usize);
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(v: Value) -> CandidateInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn canonical_json_sorts_nested_keys() {
        let mut inner = serde_json::Map::new();
        inner.insert("z".into(), json!(1));
        inner.insert("a".into(), json!([{"y": 2, "b": 3}]));
        assert_eq!(
            canonical_json(&Value::Object(inner)),
            r#"{"a":[{"b":3,"y":2}],"z":1}"#
        );
    }

    #[test]
    fn unknown_top_level_fields_are_rejected_not_dropped() {
        let r: Result<CandidateInput, _> =
            serde_json::from_value(json!({"atom": {}, "reconciliation": []}));
        assert!(r.unwrap_err().to_string().contains("reconciliation"));
    }

    #[test]
    fn stage_dedupes_flags_lean_and_opens_conflicts() {
        let s = Store::open_memory().unwrap();
        let a = json!({"atom_id":"A042","author":"david","atom":{"object_type":"CLAIM","lean_state":"BUILT","generated_at":"t1"}});
        let r1 = stage(&s, input(a.clone())).unwrap();
        assert_eq!(r1.verdict, "NEEDS_REVIEW");
        assert_eq!(r1.reasons[0].check_id, "LEAN_RECEIPT_MISSING");

        let mut again = a.clone();
        again["atom"]["generated_at"] = json!("t2");
        let r2 = stage(&s, input(again)).unwrap();
        assert!(r2.deduplicated);
        assert_eq!(r2.candidate_id, r1.candidate_id);

        let b = json!({"atom_id":"A042","author":"david","atom":{"object_type":"CLAIM","lean_state":"BUILT","receipt_id":"R1"}});
        let r3 = stage(&s, input(b)).unwrap();
        assert_eq!(r3.verdict, "PASS");
        assert_eq!(r3.conflicts_opened.len(), 1);

        let hits = search(&s, "A042", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["payload"]["object_type"], "CLAIM");
        let ag = agenda(&s, 50).unwrap();
        assert_eq!(ag[0]["kind"], "conflict");
        assert!(ag[0]["affected"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "A042"));
    }

    #[test]
    fn reconciliation_rows_accepting_a_build_without_receipt_are_flagged() {
        let atom = json!({"object_type":"PROCESS","reconciliation":[{"axis":"lean_state","proposed":"ZERO_SORRY","resolution":"Accept proposed"}]});
        let reasons = checks(&input(json!({"atom": atom, "author": "d"})));
        assert_eq!(reasons.len(), 1);
    }
}
