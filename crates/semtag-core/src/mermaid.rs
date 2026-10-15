//! Mermaid graph of tags, parent -> child (section 6.5).

use std::fmt::Write;

use crate::tag::{dedupe, Tag};

fn node_text(tag: &Tag) -> String {
    let label: String = tag
        .label
        .replace('"', "'")
        .replace('[', "(")
        .replace(']', ")")
        .chars()
        .take(50)
        .collect();
    format!(
        "{}: {}",
        tag.display_type().replace('"', "'"),
        label.replace(['\r', '\n'], " ")
    )
}

/// `direction` is one of `TD`, `LR`, `BT`, `RL`.
pub fn render(tags: &[Tag], direction: &str) -> String {
    let tags = dedupe(tags);
    let mut s = format!("graph {direction}\n");
    for (i, tag) in tags.iter().enumerate() {
        let _ = writeln!(s, "  n{i}[\"{}\"]", node_text(tag));
    }
    for (child, tag) in tags.iter().enumerate() {
        let Some(parent) = &tag.parent_uuid else {
            continue;
        };
        if let Some(p) = tags.iter().position(|t| &t.uuid == parent) {
            let _ = writeln!(s, "  n{p} --> n{child}");
        }
    }
    s
}
