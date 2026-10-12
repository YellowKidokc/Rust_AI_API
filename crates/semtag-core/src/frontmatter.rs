//! Front-matter counts (section 6.4). Lines are edited in place; the YAML is never reserialized.

use crate::ops::Operation;
use crate::tag::{dedupe, Tag};

/// Byte range of the front-matter body: from after the opening `---\n` to the start of the
/// closing `---` line. `None` if the note has no front matter.
pub fn body_range(note: &str) -> Option<(usize, usize)> {
    let rest = note.strip_prefix("---\n")?;
    let start = note.len() - rest.len();
    let mut pos = start;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((start, pos));
        }
        pos += line.len();
    }
    None
}

/// `(key, count)` in write order. `evidence_count` and `anchor_count` are the same number: both
/// count `anchor` tags; the two keys are kept for compatibility with the plugin.
pub fn counts(tags: &[Tag]) -> Vec<(&'static str, usize)> {
    let tags = dedupe(tags);
    let n = |op: Operation| tags.iter().filter(|t| t.operation() == Some(op)).count();
    vec![
        ("axiom_count", n(Operation::Establish)),
        ("claim_count", n(Operation::Derive)),
        ("evidence_count", n(Operation::Anchor)),
        ("bridge_count", n(Operation::Bridge)),
        ("anchor_count", n(Operation::Anchor)),
        ("challenge_count", n(Operation::Challenge)),
        ("declare_count", n(Operation::Declare)),
    ]
}

/// Replace or append each count line. Notes without front matter are returned unchanged.
pub fn update_counts(note: &str, tags: &[Tag]) -> String {
    let Some((start, end)) = body_range(note) else {
        return note.to_string();
    };
    let mut lines: Vec<String> = note[start..end].lines().map(str::to_string).collect();
    for (key, n) in counts(tags) {
        let prefix = format!("{key}:");
        let line = format!("{key}: {n}");
        match lines.iter_mut().find(|l| l.starts_with(&prefix)) {
            Some(existing) => *existing = line,
            None => lines.push(line),
        }
    }
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    format!("{}{}{}", &note[..start], body, &note[end..])
}
