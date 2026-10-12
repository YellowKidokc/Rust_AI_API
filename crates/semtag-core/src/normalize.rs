//! Label normalization and sanitization.

/// The one label normalizer (the plugin had two that disagreed on `-`).
///
/// Lowercase, keep Unicode letters and digits, `_`, `-` and whitespace, then collapse whitespace.
/// Unicode-aware, so `χ` survives (JS `\w` was ASCII-only and deleted it).
pub fn normalize_label(s: &str) -> String {
    let kept: String = s
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || c.is_whitespace())
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Make a label safe for the `%%tag::...::"label"::...%%` line: `"` -> `'`, `%%` -> `%`,
/// newlines -> spaces.
pub fn sanitize_label(s: &str) -> String {
    let mut out = s.replace(['\r', '\n'], " ").replace('"', "'");
    while out.contains("%%") {
        out = out.replace("%%", "%");
    }
    out
}

/// Escape `& < > "` for the HTML blocks.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
