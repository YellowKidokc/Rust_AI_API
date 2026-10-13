//! Putting it together: write all blocks onto a note, idempotently, and strip them again.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

use crate::fold::FoldConfig;
use crate::frontmatter::{self, body_range};
use crate::inline::{self, INLINE_END, INLINE_START};
use crate::ops::missing_fields;
use crate::tag::{block_range, parse_tags, render_block, Tag};
use crate::tsns::{self, TSNS_END, TSNS_START};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TsnsMode {
    Off,
    Full,
    #[default]
    Strict,
}

#[derive(Debug, Clone)]
pub struct WriteOptions {
    pub tag_block: bool,
    pub inline_labels: bool,
    pub tsns: TsnsMode,
    pub frontmatter_counts: bool,
    pub fold: FoldConfig,
    /// Rewrite the tag block from `new` only. Default is append: every existing tag stays.
    pub replace: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions {
            tag_block: true,
            inline_labels: true,
            tsns: TsnsMode::Strict,
            frontmatter_counts: true,
            fold: FoldConfig::default(),
            replace: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrictFailure {
    pub tag_type: String,
    pub label: String,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WriteOutcome {
    /// The new note text, or the original unchanged when strict mode blocked it.
    pub text: String,
    /// Every tag now on the note.
    pub tags: Vec<Tag>,
    pub added: usize,
    /// Non-empty means the note was not written.
    pub strict_failures: Vec<StrictFailure>,
}

impl WriteOutcome {
    pub fn written(&self) -> bool {
        self.strict_failures.is_empty()
    }
}

fn block_re(start: &str, end: &str) -> Regex {
    Regex::new(&format!(
        r"(?s)\n?{}.*?{}\n?",
        regex::escape(start),
        regex::escape(end)
    ))
    .unwrap()
}

fn inline_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| block_re(INLINE_START, INLINE_END))
}

fn tsns_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| block_re(TSNS_START, TSNS_END))
}

fn forward_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^%%forward-link::.*%%[ \t]*\n?").unwrap())
}

fn remove_display_blocks(text: &str) -> String {
    let text = inline_re().replace_all(text, "");
    tsns_re().replace_all(&text, "").into_owned()
}

/// Remove every semtag block (inline labels, TSNS, tag block, forward links). Front-matter counts stay.
pub fn strip_blocks(note: &str) -> String {
    let lf = note.replace("\r\n", "\n");
    let mut text = remove_display_blocks(&lf);
    if let Some((start, end)) = block_range(&text) {
        let after = text[end..]
            .strip_prefix('\n')
            .unwrap_or(&text[end..])
            .to_string();
        text = format!("{}\n{}", text[..start].trim_end(), after);
    }
    let text = forward_link_re().replace_all(&text, "").into_owned();
    if note.contains("\r\n") {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

/// What the model sees: the note with all of semtag's own blocks removed.
pub fn strip_for_model(note: &str) -> String {
    strip_blocks(note).trim_end().to_string()
}

/// Byte offset just after the first `# ` heading line that follows the front matter, skipping
/// fenced code.
fn heading_insert_point(text: &str) -> Option<usize> {
    let mut pos = body_range(text).map_or(0, |(_, end)| {
        let close = &text[end..];
        end + close.find('\n').map_or(close.len(), |i| i + 1)
    });
    let mut fence: Option<&str> = None;
    for line in text[pos..].split_inclusive('\n') {
        let t = line.trim_start();
        match fence {
            Some(f) if t.starts_with(f) => fence = None,
            Some(_) => {}
            None if t.starts_with("```") => fence = Some("```"),
            None if t.starts_with("~~~") => fence = Some("~~~"),
            None if line.starts_with("# ") => return Some(pos + line.len()),
            None => {}
        }
        pos += line.len();
    }
    None
}

fn insert_display_blocks(text: &str, blocks: &[String]) -> String {
    if blocks.is_empty() {
        return text.to_string();
    }
    let joined = blocks.join("\n\n");
    match heading_insert_point(text) {
        Some(at) if text[..at].ends_with('\n') => {
            format!("{}\n{}\n{}", &text[..at], joined, &text[at..])
        }
        Some(at) => format!("{}\n\n{}\n{}", &text[..at], joined, &text[at..]),
        None => format!("{}\n\n{}\n", text.trim_end(), joined),
    }
}

/// Write `new` tags onto `note`: tag block (append by default), front-matter counts, inline
/// labels and TSNS blocks. Rerunning with the same input gives the same bytes.
pub fn apply(note: &str, new: &[Tag], opts: &WriteOptions) -> WriteOutcome {
    let crlf = note.contains("\r\n");
    let work = note.replace("\r\n", "\n");
    let existing = parse_tags(&work);

    let new: Vec<Tag> = if opts.replace {
        new.to_vec()
    } else {
        new.iter()
            .filter(|t| !existing.iter().any(|e| e.uuid == t.uuid))
            .cloned()
            .collect()
    };

    if opts.tsns == TsnsMode::Strict {
        let failures: Vec<StrictFailure> = new
            .iter()
            .filter_map(|t| {
                let missing = missing_fields(t.operation()?, &t.metadata);
                (!missing.is_empty()).then(|| StrictFailure {
                    tag_type: t.display_type(),
                    label: t.label.clone(),
                    missing: missing.into_iter().map(str::to_string).collect(),
                })
            })
            .collect();
        if !failures.is_empty() {
            return WriteOutcome {
                text: note.to_string(),
                tags: existing,
                added: 0,
                strict_failures: failures,
            };
        }
    }

    let added = new.len();
    let merged: Vec<Tag> = if opts.replace {
        new
    } else {
        existing.into_iter().chain(new).collect()
    };

    // Split off the tag block; anything after it (forward links) is kept as is.
    let (main, old_block, suffix) = match block_range(&work) {
        Some((start, end)) => (&work[..start], Some(&work[start..end]), &work[end..]),
        None => (work.as_str(), None, "\n"),
    };
    let mut main = remove_display_blocks(main);
    if opts.frontmatter_counts {
        main = frontmatter::update_counts(&main, &merged);
    }
    let mut blocks = Vec::new();
    if opts.inline_labels && !merged.is_empty() {
        blocks.push(inline::render(
            &merged,
            opts.fold.inline_labels,
            opts.fold.chips,
        ));
    }
    if opts.tsns != TsnsMode::Off {
        blocks.extend(tsns::render(
            &merged,
            opts.fold.tsns_index,
            opts.fold.tsns_items,
        ));
    }
    let main = insert_display_blocks(&main, &blocks);

    let block = if opts.tag_block && !merged.is_empty() {
        Some(render_block(&merged))
    } else {
        old_block.map(str::to_string)
    };
    let mut text = match block {
        Some(block) => format!("{}\n\n\n{}{}", main.trim_end(), block, suffix),
        None => main,
    };
    if crlf {
        text = text.replace('\n', "\r\n");
    }
    WriteOutcome {
        text,
        tags: merged,
        added,
        strict_failures: Vec::new(),
    }
}
