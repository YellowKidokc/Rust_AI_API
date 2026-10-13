//! Routing hints from a note's own front matter, so a note can say how it should be processed:
//!
//! ```text
//! ---
//! route: reasoning          # explicit: use this route
//! priority: draft           # words matched against settings.json `hints`
//! compute: deep
//! tags: [physics, draft]
//! ---
//! ```
//!
//! Precedence in a pipeline run: `--route` on the command line > `route:` in the note > the first
//! `hints` rule whose word appears in the note's `priority` / `compute` / `mode` / `forge` / `tags`
//! > the template's `route:` > `default_route`.

use crate::config::RouteHint;

const HINT_KEYS: &[&str] = &["priority", "compute", "mode", "forge", "tags", "status"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NoteHints {
    pub route: Option<String>,
    /// Lowercased words from the hint-bearing keys.
    pub words: Vec<String>,
}

pub fn read(note: &str) -> NoteHints {
    let mut out = NoteHints::default();
    let note = note.strip_prefix('\u{feff}').unwrap_or(note);
    let Some(rest) = note.strip_prefix("---\n").or_else(|| note.strip_prefix("---\r\n")) else { return out };
    let mut current: Option<String> = None;
    for line in rest.lines() {
        if line.trim_end() == "---" {
            break;
        }
        let trimmed = line.trim();
        if let Some(item) = trimmed.strip_prefix("- ") {
            if current.as_deref().is_some_and(|k| HINT_KEYS.contains(&k)) {
                push_words(&mut out.words, item);
            }
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let key = k.trim().to_lowercase();
        let val = v.trim().trim_matches(|c| c == '"' || c == '\'');
        if key == "route" && !val.is_empty() {
            out.route = Some(val.to_string());
        } else if HINT_KEYS.contains(&key.as_str()) {
            push_words(&mut out.words, val);
        }
        current = Some(key);
    }
    out
}

fn push_words(words: &mut Vec<String>, value: &str) {
    for w in value.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).filter(|w| !w.is_empty()) {
        let w = w.to_lowercase();
        if !words.contains(&w) {
            words.push(w);
        }
    }
}

/// The route the note asks for, and why. `None` = the note expresses no preference.
pub fn resolve(h: &NoteHints, rules: &[RouteHint]) -> Option<(String, String)> {
    if let Some(r) = &h.route {
        return Some((r.clone(), "note front matter route:".into()));
    }
    rules
        .iter()
        .find(|rule| h.words.iter().any(|w| w.eq_ignore_ascii_case(&rule.when)))
        .map(|rule| (rule.route.clone(), format!("note marked '{}'", rule.when)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<RouteHint> {
        [("deep", "reasoning"), ("draft", "free")].iter().map(|(w, r)| RouteHint { when: w.to_string(), route: r.to_string() }).collect()
    }

    #[test]
    fn reads_scalars_inline_lists_and_block_lists() {
        let h = read("---\ntitle: X\npriority: Draft\ntags:\n  - physics\n  - Deep\n---\nbody priority: ignored\n");
        assert_eq!(h.words, vec!["draft", "physics", "deep"]);
        // Rule order decides: deep beats draft.
        assert_eq!(resolve(&h, &rules()).unwrap().0, "reasoning");
    }

    #[test]
    fn explicit_route_wins_and_no_front_matter_means_no_preference() {
        let h = read("---\nroute: premium\ntags: [draft]\n---\n");
        assert_eq!(resolve(&h, &rules()).unwrap().0, "premium");
        assert_eq!(resolve(&read("# just a note\npriority: draft"), &rules()), None);
    }
}
