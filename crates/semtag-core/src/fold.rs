//! Fold mode: every block can be written, or flipped later, as always collapsed or always expanded.

use serde::Deserialize;

use crate::inline::{INLINE_END, INLINE_START};
use crate::ops::Operation;
use crate::tsns::{TSNS_END, TSNS_START};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fold {
    #[default]
    Collapsed,
    Expanded,
}

impl Fold {
    /// Obsidian callout marker: `-` collapsed, `+` expanded.
    pub fn callout(self) -> char {
        match self {
            Fold::Collapsed => '-',
            Fold::Expanded => '+',
        }
    }

    /// Attribute appended inside a `<details ...>` tag.
    pub fn details_attr(self) -> &'static str {
        match self {
            Fold::Collapsed => "",
            Fold::Expanded => " open",
        }
    }
}

/// The four foldable blocks (`fold_blocks` keys in `classifiers.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    /// The outer `<details class="semantic-ai-inline-labels">`.
    InlineLabels,
    /// The `> [!abstract]` structural index callout.
    TsnsIndex,
    /// The per-node `> [!establish]` ... callouts.
    TsnsItems,
    /// Each `<details class="semantic-word ...">` chip.
    Chips,
}

impl Block {
    pub fn parse(name: &str) -> Option<Block> {
        match name {
            "inline_labels" => Some(Block::InlineLabels),
            "tsns_index" => Some(Block::TsnsIndex),
            "tsns_items" => Some(Block::TsnsItems),
            "chips" => Some(Block::Chips),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct FoldConfig {
    pub inline_labels: Fold,
    pub tsns_index: Fold,
    pub tsns_items: Fold,
    pub chips: Fold,
}

impl FoldConfig {
    pub fn all(fold: Fold) -> Self {
        FoldConfig {
            inline_labels: fold,
            tsns_index: fold,
            tsns_items: fold,
            chips: fold,
        }
    }
}

fn set_details(line: &str, fold: Fold) -> String {
    let body = line.trim_end_matches(['\r', '\n']);
    let ending = &line[body.len()..];
    let Some(inner) = body.strip_suffix('>') else {
        return line.to_string();
    };
    let inner = inner.strip_suffix(" open").unwrap_or(inner);
    format!("{inner}{}>{ending}", fold.details_attr())
}

fn set_callout(line: &str, fold: Fold) -> String {
    match line.find(']') {
        Some(i) if matches!(line.as_bytes().get(i + 1), Some(b'-' | b'+')) => {
            format!("{}{}{}", &line[..=i], fold.callout(), &line[i + 2..])
        }
        _ => line.to_string(),
    }
}

/// Rewrite fold markers inside semtag's own blocks only. `block = None` means all four.
/// No other byte of the note changes.
pub fn refold(note: &str, block: Option<Block>, fold: Fold) -> String {
    let wants = |b: Block| block.is_none_or(|x| x == b);
    let mut out = String::with_capacity(note.len() + 64);
    let (mut in_inline, mut in_tsns) = (false, false);
    for line in note.split_inclusive('\n') {
        let t = line.trim_end();
        if t == INLINE_START {
            in_inline = true;
        } else if t == INLINE_END {
            in_inline = false;
        } else if t == TSNS_START {
            in_tsns = true;
        } else if t == TSNS_END {
            in_tsns = false;
        }
        let new = if in_inline
            && t.starts_with("<details class=\"semantic-ai-inline-labels\"")
            && wants(Block::InlineLabels)
            || in_inline && t.starts_with("<details class=\"semantic-word") && wants(Block::Chips)
        {
            set_details(line, fold)
        } else if in_tsns && t.starts_with("> [!abstract]") && wants(Block::TsnsIndex)
            || in_tsns
                && Operation::ALL
                    .iter()
                    .any(|op| t.starts_with(&format!("> [!{}]", op.as_str())))
                && wants(Block::TsnsItems)
        {
            set_callout(line, fold)
        } else {
            line.to_string()
        };
        out.push_str(&new);
    }
    out
}
