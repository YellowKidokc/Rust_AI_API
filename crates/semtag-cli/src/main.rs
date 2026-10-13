//! `semtag` CLI. Milestone 1: the offline commands that need no API call.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use semtag_core::{
    apply, csv_export, fold, items_to_tags, mermaid, parse_tags, strip_blocks, Fold, FoldConfig,
    ModelItem, TsnsMode, WriteOptions,
};

#[derive(Parser)]
#[command(name = "semtag", version, about = "Semantic tagger for Markdown notes")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the tags parsed from a note's hidden tag block.
    ShowTags { note: PathBuf },
    /// Write tags from a model-reply JSON file onto a note (no API call).
    Apply {
        note: PathBuf,
        /// JSON array of {type, label, parentLabel, customType, confidence, metadata}.
        #[arg(long)]
        tags: PathBuf,
        #[arg(long)]
        replace: bool,
        #[arg(long, value_parser = ["strict", "full", "off"], default_value = "strict")]
        tsns: String,
        #[arg(long)]
        expand: bool,
        #[arg(long)]
        in_place: bool,
    },
    /// Mermaid graph of a note's tags.
    Mermaid {
        note: PathBuf,
        #[arg(long, value_parser = ["TD", "LR", "BT", "RL"], default_value = "TD")]
        direction: String,
    },
    /// Theory CSV next to the note (never overwrites).
    Csv { note: PathBuf },
    /// Flip fold markers in semtag blocks (file or folder, recursive). No API call.
    Fold {
        path: PathBuf,
        #[arg(
            long,
            conflicts_with = "collapse",
            required_unless_present = "collapse"
        )]
        expand: bool,
        #[arg(long)]
        collapse: bool,
        /// inline_labels | tsns_index | tsns_items | chips
        #[arg(long)]
        block: Option<String>,
    },
    /// Remove all semtag blocks from a note.
    Strip {
        note: PathBuf,
        #[arg(long)]
        in_place: bool,
    },
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Keep the original next to the note before overwriting it. Never deletes.
fn write_with_backup(path: &Path, text: &str) -> Result<()> {
    let backup = path.with_extension("md.semtag-bak");
    fs::copy(path, &backup).with_context(|| format!("backing up {}", path.display()))?;
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

fn output(path: &Path, text: &str, in_place: bool) -> Result<()> {
    if in_place {
        write_with_backup(path, text)
    } else {
        print!("{text}");
        Ok(())
    }
}

fn markdown_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(path).with_context(|| format!("reading {}", path.display()))? {
        let p = entry?.path();
        if p.is_dir() {
            out.extend(markdown_files(&p)?);
        } else if p.extension().is_some_and(|e| e == "md") {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::ShowTags { note } => {
            for t in parse_tags(&read(&note)?) {
                let parent = t.parent_uuid.as_deref().unwrap_or("-");
                let meta = if t.metadata.is_empty() {
                    String::new()
                } else {
                    format!("  {}", serde_json::Value::Object(t.metadata.clone()))
                };
                println!(
                    "{:<22} {}  \"{}\"  parent={}{}",
                    t.display_type(),
                    t.uuid,
                    t.label,
                    parent,
                    meta
                );
            }
        }
        Cmd::Apply {
            note,
            tags,
            replace,
            tsns,
            expand,
            in_place,
        } => {
            let text = read(&note)?;
            let items: Vec<ModelItem> =
                serde_json::from_str(&read(&tags)?).context("parsing --tags JSON")?;
            let existing = parse_tags(&text);
            let new = items_to_tags(&items, &existing, |_| None);
            let opts = WriteOptions {
                replace,
                tsns: match tsns.as_str() {
                    "full" => TsnsMode::Full,
                    "off" => TsnsMode::Off,
                    _ => TsnsMode::Strict,
                },
                fold: FoldConfig::all(if expand {
                    Fold::Expanded
                } else {
                    Fold::Collapsed
                }),
                ..WriteOptions::default()
            };
            let out = apply(&text, &new, &opts);
            if !out.written() {
                for f in &out.strict_failures {
                    eprintln!(
                        "strict: {} \"{}\" missing {}",
                        f.tag_type,
                        f.label,
                        f.missing.join(", ")
                    );
                }
                bail!("strict mode blocked {} (note not written)", note.display());
            }
            eprintln!(
                "{}: {} tag(s) added, {} total",
                note.display(),
                out.added,
                out.tags.len()
            );
            output(&note, &out.text, in_place)?;
        }
        Cmd::Mermaid { note, direction } => print!(
            "{}",
            mermaid::render(&parse_tags(&read(&note)?), &direction)
        ),
        Cmd::Csv { note } => {
            let tags = parse_tags(&read(&note)?);
            let dir = note.parent().unwrap_or(Path::new("."));
            let stem = note
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let path = csv_export::free_path(dir, &stem);
            fs::write(&path, csv_export::theory_csv(&note, &tags))?;
            eprintln!("wrote {}", path.display());
        }
        Cmd::Fold {
            path,
            expand,
            collapse: _,
            block,
        } => {
            let block = match block.as_deref() {
                Some(name) => Some(
                    fold::Block::parse(name).with_context(|| format!("unknown block {name}"))?,
                ),
                None => None,
            };
            let f = if expand {
                Fold::Expanded
            } else {
                Fold::Collapsed
            };
            for file in markdown_files(&path)? {
                let text = read(&file)?;
                let new = fold::refold(&text, block, f);
                if new != text {
                    write_with_backup(&file, &new)?;
                    eprintln!("refolded {}", file.display());
                }
            }
        }
        Cmd::Strip { note, in_place } => output(&note, &strip_blocks(&read(&note)?), in_place)?,
    }
    Ok(())
}
