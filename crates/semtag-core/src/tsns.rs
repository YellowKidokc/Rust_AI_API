//! The TSNS structure block (section 6.3).

use std::fmt::Write;

use crate::fold::Fold;
use crate::ops::{display_value, normalize_metadata, Operation};
use crate::tag::{dedupe, Tag};

pub const TSNS_START: &str = "<!-- TSNS STRUCTURE START -->";
pub const TSNS_END: &str = "<!-- TSNS STRUCTURE END -->";

/// `None` when no tag maps to an operation.
pub fn render(tags: &[Tag], index: Fold, items: Fold) -> Option<String> {
    let mut nodes: Vec<(Operation, String, &Tag)> = Vec::new();
    let mut counters = [0usize; 6];
    for tag in dedupe(tags) {
        if let Some(op) = tag.operation() {
            let i = Operation::ALL.iter().position(|o| *o == op).unwrap_or(0);
            counters[i] += 1;
            nodes.push((op, format!("{}{}", op.prefix(), counters[i]), tag));
        }
    }
    if nodes.is_empty() {
        return None;
    }
    // Callouts follow the index: grouped by operation, tag order within each (stable sort).
    nodes.sort_by_key(|(op, _, _)| *op);
    // Most items wins; ties go to the earlier operation in display order.
    let mut primary = 0;
    for i in 1..counters.len() {
        if counters[i] > counters[primary] {
            primary = i;
        }
    }

    let mut s = String::new();
    let _ = writeln!(s, "{TSNS_START}");
    let _ = writeln!(s, "> [!abstract]{} 📋 Structural Index", index.callout());
    s.push_str("> \n");
    let _ = writeln!(
        s,
        "> **Primary Operation:** {}",
        Operation::ALL[primary].as_str().to_uppercase()
    );
    let _ = writeln!(s, "> **Total Structured Nodes:** {}", nodes.len());
    for op in Operation::ALL {
        s.push_str("> \n");
        let _ = writeln!(s, "> **{}:**", op.index_heading());
        let mut any = false;
        for (_, id, tag) in nodes.iter().filter(|(o, _, _)| *o == op) {
            let _ = writeln!(s, "> - [[#^{id}]] — {}", one_line(&tag.label));
            any = true;
        }
        if !any {
            s.push_str("> - (none)\n");
        }
    }

    for (op, id, tag) in &nodes {
        s.push('\n');
        let _ = writeln!(
            s,
            "> [!{}]{} **{id} — {}: {}**",
            op.as_str(),
            items.callout(),
            op.callout_title(),
            one_line(&tag.label)
        );
        let meta = normalize_metadata(Some(*op), &tag.metadata);
        for field in op.required_fields() {
            let value = meta.get(field.key).map(display_value).unwrap_or_default();
            let value = if value.trim().is_empty() {
                "TODO".to_string()
            } else {
                one_line(&value)
            };
            let _ = writeln!(s, "> **{}:** {value}", field.label);
        }
        let _ = writeln!(s, "\n^{id}");
    }
    s.push_str(TSNS_END);
    Some(s)
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}
