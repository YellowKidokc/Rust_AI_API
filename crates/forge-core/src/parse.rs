//! Tolerant extraction of a JSON value from a model reply: fenced block, then the outermost
//! object or array, then the whole text.

use serde_json::Value;

pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = after.find("```") {
            if let Ok(v) = serde_json::from_str(after[..end].trim()) {
                return Some(v);
            }
        }
    }
    if let Ok(v) = serde_json::from_str(t) {
        return Some(v);
    }
    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(a), Some(b)) = (t.find(open), t.rfind(close)) {
            if a < b {
                if let Ok(v) = serde_json::from_str(&t[a..=b]) {
                    return Some(v);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_json_in_common_reply_shapes() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```").unwrap(), serde_json::json!({"a":1}));
        assert_eq!(extract_json("Sure! {\"a\":[1,2]} hope this helps").unwrap(), serde_json::json!({"a":[1,2]}));
        assert_eq!(extract_json("[1, 2]").unwrap(), serde_json::json!([1, 2]));
        assert!(extract_json("no json here").is_none());
    }
}
