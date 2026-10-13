//! Job templates: Markdown files with a small front-matter header, rendered with minijinja.
//! Editing a template never needs a recompile. Undefined variables are an error, not a blank.
//!
//! ```text
//! ---
//! route: default
//! json: false
//! system: You are a careful editor.
//! context_k: 5
//! suffix: SUMMARY
//! ---
//! Summarize {{ name }}.
//! {% if context %}Background:
//! {{ context }}{% endif %}
//! {{ document }}
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

pub const BUILTIN: &[(&str, &str)] = &[
    ("default", include_str!("../../../templates/default.md")),
    ("summarize", include_str!("../../../templates/summarize.md")),
    ("sort-clip", include_str!("../../../templates/sort-clip.md")),
    (
        "review-clips",
        include_str!("../../../templates/review-clips.md"),
    ),
    (
        "extract-claims",
        include_str!("../../../templates/extract-claims.md"),
    ),
];

#[derive(Debug, Clone, Serialize)]
pub struct Template {
    pub name: String,
    pub route: Option<String>,
    pub json: bool,
    pub system: Option<String>,
    pub context_k: Option<usize>,
    pub suffix: String,
    pub body: String,
}

impl Template {
    pub fn parse(name: &str, text: &str) -> Result<Self> {
        let text = text
            .strip_prefix('\u{feff}')
            .unwrap_or(text)
            .replace("\r\n", "\n");
        let mut meta = BTreeMap::new();
        let body = if let Some(rest) = text.strip_prefix("---\n") {
            let (head, body) = rest
                .split_once("\n---\n")
                .ok_or_else(|| anyhow!("template {name}: front matter not closed"))?;
            for line in head
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            {
                let (k, v) = line
                    .split_once(':')
                    .ok_or_else(|| anyhow!("template {name}: bad header line '{line}'"))?;
                meta.insert(k.trim().to_string(), v.trim().to_string());
            }
            body.to_string()
        } else {
            text
        };
        let t = Self {
            name: name.into(),
            route: meta.get("route").filter(|v| !v.is_empty()).cloned(),
            json: meta.get("json").is_some_and(|v| v == "true"),
            system: meta.get("system").filter(|v| !v.is_empty()).cloned(),
            context_k: meta.get("context_k").and_then(|v| v.parse().ok()),
            suffix: meta
                .get("suffix")
                .cloned()
                .unwrap_or_else(|| name.to_uppercase()),
            body,
        };
        // Compile once now so syntax errors surface at load time.
        env()
            .template_from_str(&t.body)
            .with_context(|| format!("template {name}"))?;
        Ok(t)
    }

    pub fn render<S: Serialize>(&self, vars: S) -> Result<String> {
        let env = env();
        let tpl = env.template_from_str(&self.body)?;
        tpl.render(vars)
            .with_context(|| format!("rendering template {}", self.name))
    }
}

fn env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env.set_keep_trailing_newline(true);
    env
}

/// Load `<dir>/<name>.md`, falling back to the built-in template of that name.
pub fn load(dir: &Path, name: &str) -> Result<Template> {
    let file = dir.join(format!("{name}.md"));
    if file.is_file() {
        return Template::parse(name, &std::fs::read_to_string(&file)?);
    }
    match BUILTIN.iter().find(|(n, _)| *n == name) {
        Some((_, text)) => Template::parse(name, text),
        None => Err(anyhow!(
            "no template '{name}' in {} and no built-in of that name",
            dir.display()
        )),
    }
}

/// Every template name available: files in `dir` plus built-ins.
pub fn list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = BUILTIN.iter().map(|(n, _)| n.to_string()).collect();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "md") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        for (n, t) in BUILTIN {
            Template::parse(n, t).unwrap();
        }
    }

    #[test]
    fn front_matter_and_strict_undefined() {
        let t = Template::parse(
            "x",
            "---\n# comment\nroute: free\njson: true\nsuffix: X\n---\nHi {{ name }}",
        )
        .unwrap();
        assert_eq!(t.route.as_deref(), Some("free"));
        assert!(t.json);
        assert_eq!(t.render(serde_json::json!({"name": "D"})).unwrap(), "Hi D");
        assert!(t.render(serde_json::json!({})).is_err());
    }
}
