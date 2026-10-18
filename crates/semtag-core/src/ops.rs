//! TSNS operations, their required fields, the seven facets, and alias normalization.

use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Operation {
    Establish,
    Derive,
    Anchor,
    Bridge,
    Challenge,
    Declare,
}

/// One metadata field: canonical key, display label, and the aliases that can fill it.
#[derive(Debug)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
}

const fn f(key: &'static str, label: &'static str, aliases: &'static [&'static str]) -> Field {
    Field {
        key,
        label,
        aliases,
    }
}

const ESTABLISH: &[Field] = &[
    f(
        "deathConditionsSurvived",
        "Death conditions survived",
        &[
            "death_conditions_survived",
            "deathConditions",
            "death_conditions",
        ],
    ),
    f(
        "propagationTest",
        "Propagation test",
        &["propagation_test", "propagation"],
    ),
    f(
        "whatShouldKillIt",
        "What should kill it",
        &["what_should_kill_it", "kill_condition", "killConditions"],
    ),
];
const DERIVE: &[Field] = &[
    f("sourceNodes", "Source nodes", &["source_nodes", "sources"]),
    f("chain", "Chain", &["reasoning_chain", "derivation_chain"]),
    f("weakestLink", "Weakest link", &["weakest_link"]),
    f("ifBreaks", "If breaks", &["if_breaks", "failure_mode"]),
];
const CHALLENGE: &[Field] = &[
    f("target", "Target", &["targets"]),
    f(
        "strongestAttack",
        "Strongest attack",
        &["strongest_attack", "steelman"],
    ),
    f("survived", "Survived", &["survival", "survives"]),
    f(
        "damageReport",
        "Damage report",
        &["damage_report", "damage"],
    ),
];
const BRIDGE: &[Field] = &[
    f("domainA", "Domain A", &["domain_a"]),
    f("domainB", "Domain B", &["domain_b"]),
    f("isomorphism", "Isomorphism", &["mapping"]),
    f(
        "constrainsPredictions",
        "Constrains predictions",
        &[
            "constrains_predictions",
            "constrains_both_domains",
            "constrains",
        ],
    ),
    f(
        "whatWouldMakeThisJustAMetaphor",
        "What would make this just a metaphor",
        &["what_makes_it_just_metaphor", "metaphor_kill_condition"],
    ),
];
const ANCHOR: &[Field] = &[
    f(
        "claimBeingGrounded",
        "Claim being grounded",
        &["claim_being_grounded", "claim"],
    ),
    f("dataCited", "Data cited", &["data_cited", "evidence"]),
    f(
        "supportsFullOrWeakerVersion",
        "Supports full or weaker version",
        &["supports_full_or_weaker_version", "support_strength"],
    ),
    f(
        "whatKillsEmpirically",
        "What kills empirically",
        &["what_kills_empirically", "empirical_kill_condition"],
    ),
    f("confidence", "Confidence", &["confidence_level"]),
];
const DECLARE: &[Field] = &[
    f(
        "basisForDeclaration",
        "Basis for declaration",
        &["basis_for_declaration", "basis"],
    ),
    f("scope", "Scope", &["scopes"]),
    f("residualRisk", "Residual risk", &["residual_risk", "risk"]),
];
/// Required on every operation, after the operation's own fields.
pub const COMMON: &[Field] = &[
    f("vulnerability", "Vulnerability", &["vulnerabilities"]),
    f(
        "unlocks",
        "Unlocks",
        &["unlock", "unlocked", "implications"],
    ),
];
/// The seven facets, required on every operation in strict mode.
pub const FACETS: &[Field] = &[
    f(
        "entity",
        "Entity",
        &["facet_entity", "entityFacet", "whatIsThis", "identity"],
    ),
    f(
        "role",
        "Role",
        &["facet_role", "roleFacet", "function", "whatDoesItDo"],
    ),
    f(
        "evidence",
        "Evidence",
        &["facet_evidence", "evidenceFacet", "support", "grounding"],
    ),
    f(
        "time",
        "Time",
        &["facet_time", "timeFacet", "sequence", "temporal"],
    ),
    f(
        "ops",
        "Ops",
        &["facet_ops", "opsFacet", "workflow", "status"],
    ),
    f(
        "context",
        "Context",
        &["facet_context", "contextFacet", "lens", "domain"],
    ),
    f(
        "provenance",
        "Provenance",
        &["facet_provenance", "provenanceFacet", "origin", "audit"],
    ),
];

impl Operation {
    /// Display and index order.
    pub const ALL: [Operation; 6] = [
        Operation::Establish,
        Operation::Derive,
        Operation::Anchor,
        Operation::Bridge,
        Operation::Challenge,
        Operation::Declare,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Operation::Establish => "establish",
            Operation::Derive => "derive",
            Operation::Anchor => "anchor",
            Operation::Bridge => "bridge",
            Operation::Challenge => "challenge",
            Operation::Declare => "declare",
        }
    }

    /// Anchor prefix in the TSNS block (`^A1`, `^C2`, ...).
    pub fn prefix(self) -> char {
        match self {
            Operation::Establish => 'A',
            Operation::Derive => 'C',
            Operation::Anchor => 'E',
            Operation::Bridge => 'B',
            Operation::Challenge => 'X',
            Operation::Declare => 'D',
        }
    }

    pub fn callout_title(self) -> &'static str {
        match self {
            Operation::Establish => "Axiom",
            Operation::Derive => "Claim",
            Operation::Anchor => "Evidence",
            Operation::Bridge => "Relationship",
            Operation::Challenge => "Challenge",
            Operation::Declare => "Declaration",
        }
    }

    pub fn index_heading(self) -> &'static str {
        match self {
            Operation::Establish => "Axioms Established",
            Operation::Derive => "Claims Derived",
            Operation::Anchor => "Evidence Anchored",
            Operation::Bridge => "Bridges Claimed",
            Operation::Challenge => "Challenges Addressed",
            Operation::Declare => "Declarations",
        }
    }

    /// The operation's own fields (without the common `vulnerability` / `unlocks`).
    pub fn own_fields(self) -> &'static [Field] {
        match self {
            Operation::Establish => ESTABLISH,
            Operation::Derive => DERIVE,
            Operation::Anchor => ANCHOR,
            Operation::Bridge => BRIDGE,
            Operation::Challenge => CHALLENGE,
            Operation::Declare => DECLARE,
        }
    }

    /// Own fields followed by `vulnerability` and `unlocks`.
    pub fn required_fields(self) -> impl Iterator<Item = &'static Field> {
        self.own_fields().iter().chain(COMMON.iter())
    }
}

/// Map a tag type (and custom keyword) to its operation.
pub fn operation_for(tag_type: &str, custom_type: Option<&str>) -> Option<Operation> {
    match tag_type {
        "Axiom" => Some(Operation::Establish),
        "Claim" | "ScientificProcess" => Some(Operation::Derive),
        "EvidenceBundle" => Some(Operation::Anchor),
        "Relationship" => Some(Operation::Bridge),
        "Custom" => infer_custom(custom_type.unwrap_or("")),
        _ => None,
    }
}

/// Custom operation inference: substring of the lowercased keyword, first match wins.
pub fn infer_custom(custom_type: &str) -> Option<Operation> {
    const RULES: &[(&[&str], Operation)] = &[
        (&["challenge", "attack"], Operation::Challenge),
        (&["declare"], Operation::Declare),
        (&["bridge", "connect"], Operation::Bridge),
        (&["anchor", "evidence"], Operation::Anchor),
        (&["derive", "claim"], Operation::Derive),
        (&["establish", "axiom"], Operation::Establish),
    ];
    let lower = custom_type.to_lowercase();
    RULES
        .iter()
        .find(|(words, _)| words.iter().any(|w| lower.contains(w)))
        .map(|(_, op)| *op)
}

/// bool always counts; numbers count; arrays/objects when non-empty; strings with non-whitespace.
pub fn is_non_empty(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// bool -> Yes/No, array -> joined with `"; "`, null -> empty.
pub fn display_value(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(true) => "Yes".into(),
        Value::Bool(false) => "No".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(display_value).collect::<Vec<_>>().join("; "),
        Value::Object(_) => v.to_string(),
    }
}

fn fill_from_aliases(meta: &mut Map<String, Value>, field: &Field) {
    if meta.get(field.key).is_some_and(is_non_empty) {
        return;
    }
    let found = field
        .aliases
        .iter()
        .find_map(|a| meta.get(*a).filter(|v| is_non_empty(v)).cloned());
    if let Some(v) = found {
        meta.insert(field.key.to_string(), v);
    }
}

/// Copy of `meta` with every canonical key filled from its first non-empty alias.
/// Alias keys are kept; nothing is removed.
pub fn normalize_metadata(op: Option<Operation>, meta: &Map<String, Value>) -> Map<String, Value> {
    let mut out = meta.clone();
    if let Some(op) = op {
        for field in op.required_fields() {
            fill_from_aliases(&mut out, field);
        }
    }
    for field in FACETS {
        fill_from_aliases(&mut out, field);
    }
    out
}

/// Strict-mode check: canonical keys (operation fields, then facets) still empty after normalization.
pub fn missing_fields(op: Operation, meta: &Map<String, Value>) -> Vec<&'static str> {
    let normalized = normalize_metadata(Some(op), meta);
    op.required_fields()
        .chain(FACETS.iter())
        .filter(|field| !normalized.get(field.key).is_some_and(is_non_empty))
        .map(|field| field.key)
        .collect()
}
