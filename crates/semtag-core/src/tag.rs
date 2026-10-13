//! The hidden tag block: the source of truth at the end of every tagged note.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::normalize::sanitize_label;
use crate::ops::{operation_for, Operation};
use crate::uri::{decode_uri_component, encode_uri_component};

pub const TAG_START: &str = "%%--- SEMANTIC TAGS ---%%";
pub const TAG_END: &str = "%%--- END SEMANTIC TAGS ---%%";

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// `Axiom`, `Claim`, ... or `Custom`.
    pub tag_type: String,
    /// Set when `tag_type` is `Custom` (written as `Custom:<customType>`).
    pub custom_type: Option<String>,
    pub uuid: String,
    pub label: String,
    pub parent_uuid: Option<String>,
    pub metadata: Map<String, Value>,
    /// From the model reply; not part of the tag line.
    pub confidence: Option<f64>,
    /// Exact text between the parent field and the closing `%%` as read from a note
    /// (e.g. `::meta=%7B...%7D`). Re-emitted verbatim so untouched tags round-trip byte for byte.
    /// Clear it whenever `metadata` is changed.
    pub raw_meta: Option<String>,
}

impl Tag {
    pub fn new(tag_type: &str, label: &str, uuid: &str) -> Self {
        Tag {
            tag_type: tag_type.to_string(),
            custom_type: None,
            uuid: uuid.to_string(),
            label: label.to_string(),
            parent_uuid: None,
            metadata: Map::new(),
            confidence: None,
            raw_meta: None,
        }
    }

    /// `Claim`, or `Custom:<customType>`.
    pub fn display_type(&self) -> String {
        match (&self.custom_type, self.tag_type.as_str()) {
            (Some(c), "Custom") => format!("Custom:{c}"),
            _ => self.tag_type.clone(),
        }
    }

    pub fn operation(&self) -> Option<Operation> {
        operation_for(&self.tag_type, self.custom_type.as_deref())
    }

    /// UUID, or `type::label::parent::customType` when the UUID is missing.
    pub fn dedupe_key(&self) -> String {
        if !self.uuid.is_empty() {
            return self.uuid.clone();
        }
        format!(
            "{}::{}::{}::{}",
            self.tag_type,
            self.label,
            self.parent_uuid.as_deref().unwrap_or(""),
            self.custom_type.as_deref().unwrap_or("")
        )
    }

    /// One `%%tag::...%%` line (no newline).
    pub fn to_line(&self) -> String {
        let meta = match &self.raw_meta {
            Some(raw) => raw.clone(),
            None if self.metadata.is_empty() => String::new(),
            None => {
                let json = Value::Object(self.metadata.clone()).to_string();
                format!("::meta={}", encode_uri_component(&json))
            }
        };
        format!(
            "%%tag::{}::{}::\"{}\"::{}{}%%",
            self.display_type(),
            self.uuid,
            sanitize_label(&self.label),
            self.parent_uuid.as_deref().unwrap_or("null"),
            meta
        )
    }
}

fn line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Looser than the plugin's regex on purpose: the type may be `Custom:<x>`, and `meta` is
    // percent-encoded so it contains `%`. Encoded meta never contains `"` or `:`, so a greedy
    // label still stops at the last `"::`.
    RE.get_or_init(|| {
        Regex::new(r#"^%%tag::(.+?)::([^:"]+)::"(.*)"::([^:%"]*)((?:::(?:meta=)?[^"]*?)?)%%\s*$"#)
            .unwrap()
    })
}

/// Parse one tag line. Returns `None` if the line isn't a tag.
pub fn parse_line(line: &str) -> Option<Tag> {
    let caps = line_re().captures(line.trim_end_matches('\r'))?;
    let full_type = &caps[1];
    let (tag_type, custom_type) = match full_type.strip_prefix("Custom:") {
        Some(c) => ("Custom".to_string(), Some(c.to_string())),
        None => (full_type.to_string(), None),
    };
    let parent = &caps[4];
    let raw = &caps[5];
    let metadata = raw
        .strip_prefix("::")
        .map(|m| m.strip_prefix("meta=").unwrap_or(m))
        .and_then(decode_uri_component)
        .and_then(|json| serde_json::from_str::<Map<String, Value>>(&json).ok())
        .unwrap_or_default();
    Some(Tag {
        tag_type,
        custom_type,
        uuid: caps[2].to_string(),
        label: caps[3].to_string(),
        parent_uuid: if parent.is_empty() || parent == "null" {
            None
        } else {
            Some(parent.to_string())
        },
        metadata,
        confidence: None,
        raw_meta: if raw.is_empty() {
            None
        } else {
            Some(raw.to_string())
        },
    })
}

/// Every tag line anywhere in the note, in order.
pub fn parse_tags(note: &str) -> Vec<Tag> {
    note.lines().filter_map(parse_line).collect()
}

/// `START`, one line per tag, `END` (no trailing newline).
pub fn render_block(tags: &[Tag]) -> String {
    let mut out = String::from(TAG_START);
    out.push('\n');
    for tag in tags {
        out.push_str(&tag.to_line());
        out.push('\n');
    }
    out.push_str(TAG_END);
    out
}

/// First occurrence of each tag by [`Tag::dedupe_key`].
pub fn dedupe(tags: &[Tag]) -> Vec<&Tag> {
    let mut seen = HashSet::new();
    tags.iter()
        .filter(|t| seen.insert(t.dedupe_key()))
        .collect()
}

/// Byte range of the tag block, from the start of `TAG_START` to the end of `TAG_END`.
pub fn block_range(note: &str) -> Option<(usize, usize)> {
    let start = note.find(TAG_START)?;
    let end = note[start..].find(TAG_END)? + start + TAG_END.len();
    Some((start, end))
}
