//! The inline labels block (section 6.2).

use std::fmt::Write;

use crate::fold::Fold;
use crate::normalize::html_escape;
use crate::ops::{display_value, normalize_metadata, FACETS};
use crate::tag::{dedupe, Tag};

pub const INLINE_START: &str = "<!-- SEMANTIC INLINE LABELS START -->";
pub const INLINE_END: &str = "<!-- SEMANTIC INLINE LABELS END -->";

const GROUP_ORDER: &[&str] = &[
    "Axiom",
    "Claim",
    "EvidenceBundle",
    "Relationship",
    "ScientificProcess",
    "InternalLink",
    "ExternalLink",
    "ProperName",
    "ForwardLink",
    "WordOntology",
    "Sentence",
    "Paragraph",
];

/// Chip CSS modifier: by operation first, then `Custom` containing "master", else generic.
pub fn chip_class(tag: &Tag) -> &'static str {
    use crate::ops::Operation::*;
    match tag.operation() {
        Some(Establish) => "axiom",
        Some(Anchor) => "evidence",
        Some(Bridge) => "relationship",
        Some(Derive) => "claim",
        _ if tag.tag_type == "Custom"
            && tag
                .custom_type
                .as_deref()
                .is_some_and(|c| c.to_lowercase().contains("master")) =>
        {
            "mastereq"
        }
        _ => "generic",
    }
}

/// Group tags: fixed order, then `Custom:*` groups, then anything else (first appearance).
fn groups<'a>(tags: &[&'a Tag]) -> Vec<(String, Vec<&'a Tag>)> {
    let mut groups: Vec<(String, Vec<&Tag>)> = Vec::new();
    for tag in tags {
        let key = tag.display_type();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(tag),
            None => groups.push((key, vec![tag])),
        }
    }
    let rank = |k: &str| match GROUP_ORDER.iter().position(|g| *g == k) {
        Some(i) => i,
        None if k.starts_with("Custom") => GROUP_ORDER.len(),
        None => GROUP_ORDER.len() + 1,
    };
    groups.sort_by_key(|(k, _)| rank(k)); // stable: first appearance within a rank
    groups
}

pub fn render(tags: &[Tag], outer: Fold, chips: Fold) -> String {
    let tags = dedupe(tags);
    let mut s = String::new();
    let _ = writeln!(s, "{INLINE_START}");
    let _ = writeln!(
        s,
        "<details class=\"semantic-ai-inline-labels\"{}>",
        outer.details_attr()
    );
    s.push_str("<summary><strong>Semantic Labels</strong> (click to show/hide)</summary>\n\n");
    let _ = writeln!(s, "Total tags: {}\n", tags.len());
    for (group, members) in groups(&tags) {
        let _ = writeln!(s, "**{} ({})**", html_escape(&group), members.len());
        s.push_str("<div class=\"semantic-word-grid\">\n");
        for tag in members {
            let _ = writeln!(
                s,
                "<details class=\"semantic-word semantic-word--{}\"{}>",
                chip_class(tag),
                chips.details_attr()
            );
            let _ = writeln!(
                s,
                "<summary><span class=\"semantic-word__label\">{}</span></summary>",
                html_escape(&tag.label)
            );
            s.push_str("<div class=\"semantic-word__panel\">\n");
            row(&mut s, "Type", &tag.display_type());
            if let Some(p) = &tag.parent_uuid {
                let parent = tags
                    .iter()
                    .find(|t| &t.uuid == p)
                    .map_or(p.as_str(), |t| t.label.as_str());
                row(&mut s, "Parent", parent);
            }
            let meta = normalize_metadata(tag.operation(), &tag.metadata);
            for facet in FACETS {
                let value = meta.get(facet.key).map(display_value).unwrap_or_default();
                if !value.trim().is_empty() {
                    row(&mut s, facet.label, &value);
                }
            }
            s.push_str("</div>\n</details>\n");
        }
        s.push_str("</div>\n\n");
    }
    s.push_str("</details>\n");
    s.push_str(INLINE_END);
    s
}

fn row(s: &mut String, name: &str, value: &str) {
    let value = value.replace(['\r', '\n'], " ");
    let _ = writeln!(
        s,
        "<div class=\"semantic-word__row\"><strong>{name}:</strong> {}</div>",
        html_escape(&value)
    );
}
