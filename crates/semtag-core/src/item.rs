//! Model reply items (section 1.2) -> tags.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::normalize::normalize_label;
use crate::ops::normalize_metadata;
use crate::tag::Tag;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub label: String,
    #[serde(default)]
    pub parent_label: Option<String>,
    #[serde(default)]
    pub custom_type: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

/// Turn reply items into tags with fresh v4 UUIDs (or `uuid_for`'s, when it returns one; the
/// concept registry plugs in here). `parentLabel` resolves against the same reply first, then
/// against `existing` tags on the note. Metadata is alias-normalized.
pub fn items_to_tags(
    items: &[ModelItem],
    existing: &[Tag],
    mut uuid_for: impl FnMut(&ModelItem) -> Option<String>,
) -> Vec<Tag> {
    let mut tags: Vec<Tag> = items
        .iter()
        .filter(|i| !i.item_type.trim().is_empty() && !i.label.trim().is_empty())
        .map(|i| {
            let uuid = uuid_for(i).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let mut tag = Tag::new(i.item_type.trim(), i.label.trim(), &uuid);
            if tag.tag_type == "Custom" {
                tag.custom_type = i.custom_type.clone().filter(|c| !c.trim().is_empty());
            }
            tag.confidence = i.confidence.map(|c| c.clamp(0.0, 1.0));
            tag.metadata = normalize_metadata(tag.operation(), &i.metadata);
            tag
        })
        .collect();

    let parents: Vec<Option<String>> = items
        .iter()
        .filter(|i| !i.item_type.trim().is_empty() && !i.label.trim().is_empty())
        .map(|i| {
            let wanted = normalize_label(i.parent_label.as_deref()?);
            if wanted.is_empty() {
                return None;
            }
            tags.iter()
                .chain(existing.iter())
                .find(|t| normalize_label(&t.label) == wanted)
                .map(|t| t.uuid.clone())
        })
        .collect();
    for (tag, parent) in tags.iter_mut().zip(parents) {
        if parent.as_deref() != Some(tag.uuid.as_str()) {
            tag.parent_uuid = parent;
        }
    }
    tags
}
