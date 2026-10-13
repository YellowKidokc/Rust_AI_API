//! Theory CSV export (section 7).

use std::path::{Path, PathBuf};

use crate::tag::{dedupe, Tag};

pub const HEADER: [&str; 12] = [
    "theory",
    "note_path",
    "note_name",
    "tag_index",
    "tag_type",
    "tag_type_index",
    "is_axiom",
    "axiom_number",
    "uuid",
    "label",
    "parent_uuid",
    "parent_label",
];

fn clean(s: &str) -> String {
    s.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// CSV text (RFC 4180 quoting, `\n` line ends). Indexes are 1-based.
pub fn theory_csv(note_path: &Path, tags: &[Tag]) -> String {
    let tags = dedupe(tags);
    let note_name = note_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let theory = note_name.replace('_', " ");
    let path = note_path.to_string_lossy();
    let mut w = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    let _ = w.write_record(HEADER);
    let mut per_type: Vec<(String, usize)> = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        let ty = tag.display_type();
        let n = match per_type.iter_mut().find(|(t, _)| *t == ty) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                per_type.push((ty.clone(), 1));
                1
            }
        };
        let is_axiom = tag.tag_type == "Axiom";
        let parent_label = tag
            .parent_uuid
            .as_ref()
            .and_then(|p| tags.iter().find(|t| &t.uuid == p))
            .map(|t| t.label.clone())
            .unwrap_or_default();
        let _ = w.write_record([
            clean(&theory),
            clean(&path),
            clean(&note_name),
            (i + 1).to_string(),
            clean(&ty),
            n.to_string(),
            is_axiom.to_string(),
            if is_axiom {
                n.to_string()
            } else {
                String::new()
            },
            tag.uuid.clone(),
            clean(&tag.label),
            tag.parent_uuid.clone().unwrap_or_default(),
            clean(&parent_label),
        ]);
    }
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

/// `<dir>/<stem>.theory.csv`, or `-2`, `-3`, ... if taken. Never overwrites.
pub fn free_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.theory.csv"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}.theory.csv")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}
