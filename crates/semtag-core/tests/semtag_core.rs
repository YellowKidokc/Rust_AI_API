use std::path::Path;

use semtag_core::fold::{refold, Block};
use semtag_core::normalize::normalize_label;
use semtag_core::tag::parse_line;
use semtag_core::uri::{decode_uri_component, encode_uri_component};
use semtag_core::*;
use serde_json::{json, Map, Value};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn reply_tags(note: &str) -> Vec<Tag> {
    let items: Vec<ModelItem> = serde_json::from_str(&fixture("reply.json")).unwrap();
    let mut n = 0;
    items_to_tags(&items, &parse_tags(note), |_| {
        n += 1;
        Some(format!("11111111-2222-4333-8444-00000000000{n}"))
    })
}

fn full_meta(extra: Value) -> Map<String, Value> {
    let mut m = json!({"sourceNodes": "A1", "chain": "A1 -> C1", "ifBreaks": "x", "vulnerability": "v", "unlocks": "u",
        "entity": "e", "role": "r", "evidence": "ev", "time": "t", "ops": "o", "context": "c", "provenance": "p"});
    m.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    m.as_object().unwrap().clone()
}

// Acceptance test 2: meta= matches JS encodeURIComponent(JSON.stringify(obj)).
#[test]
fn encoding_matches_javascript() {
    let cases: Vec<Value> = serde_json::from_str(&fixture("encode_cases.json")).unwrap();
    for case in cases {
        let js_json = case["json"].as_str().unwrap();
        let js_encoded = case["encoded"].as_str().unwrap();
        let obj: Map<String, Value> = serde_json::from_str(js_json).unwrap();
        assert_eq!(
            Value::Object(obj.clone()).to_string(),
            js_json,
            "JSON.stringify parity"
        );
        assert_eq!(encode_uri_component(js_json), js_encoded);
        assert_eq!(decode_uri_component(js_encoded).unwrap(), js_json);
        let mut tag = Tag::new("Claim", "x", "u-1");
        tag.metadata = obj.clone();
        let parsed = parse_line(&tag.to_line()).unwrap();
        assert_eq!(parsed.metadata, obj);
    }
}

// Acceptance test 1: parse -> write (append, no new tags) keeps the plugin's tag lines byte for
// byte, and a semtag-written note rewrites to identical bytes.
#[test]
fn golden_round_trip() {
    let note = fixture("plugin_tagged.md");
    let tags = parse_tags(&note);
    assert_eq!(tags.len(), 3);
    assert_eq!(tags[2].display_type(), "Custom:Challenge");
    assert_eq!(tags[1].parent_uuid.as_deref(), Some(tags[0].uuid.as_str()));
    assert_eq!(tags[1].metadata["weakestLink"], "step 2");

    let first = apply(&note, &[], &WriteOptions::default());
    assert!(first.written());
    for line in note
        .lines()
        .filter(|l| l.starts_with("%%tag::") || l.starts_with("%%forward-link"))
    {
        assert!(first.text.contains(line), "tag line changed: {line}");
    }
    let second = apply(&first.text, &[], &WriteOptions::default());
    assert_eq!(second.text, first.text);
}

#[test]
fn appended_note_is_idempotent_and_strips_back() {
    let note = fixture("plugin_tagged.md");
    let new = reply_tags(&note);
    assert_eq!(
        new[0].parent_uuid.as_deref(),
        Some("0b6f9c2e-1111-4a2b-9c3d-000000000002"),
        "parent from existing tags"
    );
    assert_eq!(new[1].confidence, Some(1.0));
    let out = apply(&note, &new, &WriteOptions::default());
    assert!(out.written(), "{:?}", out.strict_failures);
    assert_eq!(out.added, 2);
    assert_eq!(out.tags.len(), 5);
    assert!(out.text.contains("bridge_count: 1"));
    assert!(out.text.contains("claim_count: 1"));
    assert!(out.text.contains("evidence_count: 0\n"));
    assert!(out.text.contains("> **Primary Operation:** ESTABLISH"));
    assert!(out
        .text
        .contains("> [!bridge]- **B1 — Relationship: Grace ↔ entropy bridge**"));
    assert!(out.text.contains("> **Constrains predictions:** Yes"));
    assert!(out
        .text
        .contains("%%--- END SEMANTIC TAGS ---%%\n%%forward-link::coherence::[[Next Note]]%%\n"));

    // Same reply again: nothing new, identical bytes.
    let again = apply(&out.text, &new, &WriteOptions::default());
    assert_eq!(again.added, 0);
    assert_eq!(again.text, out.text);

    // Display blocks sit right after the H1 and strip back out cleanly.
    let h1 = out
        .text
        .find("# Coherence and Grace\n\n<!-- SEMANTIC INLINE LABELS START -->");
    assert!(h1.is_some());
    let stripped = strip_blocks(&out.text);
    assert!(!stripped.contains("SEMANTIC"));
    assert!(!stripped.contains("TSNS"));
    assert!(stripped.contains("# Coherence and Grace\n\nGrace acts"));
}

// Acceptance test 3: labels with `"` and `:` survive write -> parse.
#[test]
fn label_with_quotes_and_colons_round_trips() {
    let mut tag = Tag::new(
        "Claim",
        "He said \"F = ma\": force::mass %% 50%\nnext",
        "u-9",
    );
    tag.metadata = json!({"note": "x"}).as_object().unwrap().clone();
    let parsed = parse_line(&tag.to_line()).unwrap();
    assert_eq!(parsed.label, "He said 'F = ma': force::mass % 50% next");
    assert_eq!(parsed.uuid, "u-9");
    assert_eq!(parsed.metadata["note"], "x");
    assert_eq!(parse_line(&parsed.to_line()).unwrap().label, parsed.label);
}

// Acceptance test 4: strict mode.
#[test]
fn strict_mode_blocks_missing_field_and_accepts_alias() {
    let note = "# T\n\nbody\n";
    let mut claim = Tag::new("Claim", "c", "u-1");
    claim.metadata = full_meta(json!({}));
    let out = apply(note, &[claim.clone()], &WriteOptions::default());
    assert!(!out.written());
    assert_eq!(out.text, note);
    assert_eq!(out.strict_failures[0].missing, vec!["weakestLink"]);

    claim.metadata = full_meta(json!({"weakest_link": "step 3"}));
    let out = apply(note, &[claim.clone()], &WriteOptions::default());
    assert!(out.written());

    let full = WriteOptions {
        tsns: TsnsMode::Full,
        ..WriteOptions::default()
    };
    claim.metadata = Map::new();
    let out = apply(note, &[claim], &full);
    assert!(out.written());
    assert!(out.text.contains("> **Weakest link:** TODO"));
}

// Acceptance test 8: fold --expand then --collapse returns the original bytes.
#[test]
fn fold_round_trip() {
    let note = fixture("plugin_tagged.md");
    let out = apply(&note, &reply_tags(&note), &WriteOptions::default()).text;
    let expanded = refold(&out, None, Fold::Expanded);
    assert_ne!(expanded, out);
    assert!(expanded.contains("<details class=\"semantic-ai-inline-labels\" open>"));
    assert!(expanded.contains("> [!abstract]+ 📋 Structural Index"));
    assert!(expanded.contains("> [!bridge]+ **B1"));
    assert_eq!(refold(&expanded, None, Fold::Collapsed), out);

    let only_chips = refold(&out, Some(Block::Chips), Fold::Expanded);
    assert!(only_chips.contains("semantic-word--relationship\" open>"));
    assert!(only_chips.contains("<details class=\"semantic-ai-inline-labels\">"));

    // Written expanded == written collapsed then refolded.
    let opts = WriteOptions {
        fold: FoldConfig::all(Fold::Expanded),
        ..WriteOptions::default()
    };
    assert_eq!(apply(&note, &reply_tags(&note), &opts).text, expanded);
}

#[test]
fn crlf_notes_stay_crlf() {
    let note = fixture("plugin_tagged.md").replace('\n', "\r\n");
    let out = apply(&note, &reply_tags(&note), &WriteOptions::default()).text;
    assert!(!out.replace("\r\n", "").contains('\n'));
    assert_eq!(apply(&out, &[], &WriteOptions::default()).text, out);
}

#[test]
fn no_heading_no_front_matter() {
    let note = "just text\n";
    let mut t = Tag::new("Sentence", "just text", "u-1");
    t.confidence = Some(0.5);
    let out = apply(note, &[t], &WriteOptions::default()).text;
    assert!(out.starts_with("just text\n\n<!-- SEMANTIC INLINE LABELS START -->"));
    assert!(!out.contains("TSNS"), "no structured nodes, no TSNS block");
    assert!(out.ends_with("%%--- END SEMANTIC TAGS ---%%\n"));
    assert_eq!(apply(&out, &[], &WriteOptions::default()).text, out);
}

#[test]
fn heading_inside_code_fence_is_skipped() {
    let note = "```\n# not a heading\n```\n\n# Real\n\ntext\n";
    let out = apply(
        note,
        &[Tag::new("Sentence", "s", "u-1")],
        &WriteOptions::default(),
    )
    .text;
    assert!(out.contains("# Real\n\n<!-- SEMANTIC INLINE LABELS START -->"));
}

#[test]
fn normalizer_is_unicode_aware() {
    assert_eq!(
        normalize_label("  χ-Field:  Coherence!! "),
        "χ-field coherence"
    );
    assert_eq!(normalize_label("Λόγος_1"), "λόγος_1");
}

#[test]
fn custom_operation_inference() {
    use semtag_core::ops::infer_custom;
    assert_eq!(infer_custom("Strongest Attack"), Some(Operation::Challenge));
    assert_eq!(infer_custom("Connect domains"), Some(Operation::Bridge));
    assert_eq!(infer_custom("MasterEquation"), None);
    let mut t = Tag::new("Custom", "m", "u");
    t.custom_type = Some("MasterEquation".into());
    assert_eq!(semtag_core::inline::chip_class(&t), "mastereq");
}

#[test]
fn mermaid_and_csv() {
    let tags = parse_tags(&fixture("plugin_tagged.md"));
    let m = mermaid::render(&tags, "LR");
    assert_eq!(
        m,
        "graph LR\n  n0[\"Axiom: Coherence is conserved\"]\n  n1[\"Claim: Grace lowers moral entropy\"]\n  n2[\"Custom:Challenge: Entropy objection\"]\n  n0 --> n1\n"
    );
    let csv = csv_export::theory_csv(Path::new("vault/Coherence_and_Grace.md"), &tags);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], csv_export::HEADER.join(","));
    assert!(lines[1].starts_with(
        "Coherence and Grace,vault/Coherence_and_Grace.md,Coherence_and_Grace,1,Axiom,1,true,1,"
    ));
    assert!(lines[2].ends_with(
        ",Grace lowers moral entropy,0b6f9c2e-1111-4a2b-9c3d-000000000001,Coherence is conserved"
    ));
}

#[test]
fn strip_for_model_removes_everything_of_ours() {
    let note = fixture("plugin_tagged.md");
    let out = apply(&note, &reply_tags(&note), &WriteOptions::default()).text;
    let src = strip_for_model(&out);
    assert!(!src.contains("%%"));
    assert!(!src.contains("<!--"));
    assert!(src.ends_with("Some more text with a χ symbol."));
}

#[test]
fn tsns_callouts_follow_index_order() {
    let note = fixture("plugin_tagged.md");
    let out = apply(&note, &reply_tags(&note), &WriteOptions::default()).text;
    let pos = |s: &str| out.find(s).unwrap();
    assert!(pos("^A1") < pos("^C1") && pos("^C1") < pos("^B1") && pos("^B1") < pos("^X1"));
}
