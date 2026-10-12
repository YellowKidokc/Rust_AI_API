# Build prompt: Semantic Tagger in Rust (`semtag`)

Paste everything below the line into Codex or Claude. It is self-contained: it describes the Obsidian plugin being replaced
(`obsidian-semantic-ai` v2.0.0, "Semantic AI Tagger") and the house rules from `pipeline-workflows\API` that the new tool must follow.

---

## 0. Your job

Build **`semtag`**, a Rust command-line tool that does what the Obsidian plugin "Semantic AI Tagger" does, outside Obsidian, faster and
in parallel. It reads Markdown notes, sends them to an LLM with classification prompts, gets back structured JSON (axioms, claims,
evidence, relationships, and so on), and writes the results back onto the note in exact, stable formats, plus CSV / JSONL / SQLite
side outputs.

It must behave like a **station** in David's API workbench (`D:\GitHub\pipeline-workflows\API`, rules in section 2), so it can be
dropped in as one front folder with `1 RUN HERE.bat` / `2 RUN ON FOLDER.bat`.

Non-negotiables:

1. **Byte-compatible note output.** Notes already tagged by the plugin must parse, and notes tagged by `semtag` must still load in the
   plugin. The block formats in section 6 are a contract.
2. **Configurable concurrency.** How many API calls at once (1 to 60), parallel on or off, adaptive back-off on errors.
3. **Many providers** from one config file; keys only from environment variables.
4. **Everything is data, not code:** classification types, prompts, providers, models, and limits live in JSON/Markdown files.
5. **Fold mode:** every block written to a note can be rendered **always expanded** or **always collapsed**, and existing notes can be
   flipped between the two without any API call.
6. **Finish each note before the next result is handled.** No result waits in memory for the end of the run; a rerun skips what's done.
7. **One AI layer, many front ends.** All provider, limiter, prompt and receipt code lives in library crates. The batch CLI
   (part A, sections 1-10) and the **desktop layer** (part B, section 11: global hotkeys, act on selected text in any
   app, rewrite in place) are two thin front ends over the same core. Nothing provider-specific is written twice.

---

## 1. What the existing plugin does (the spec you're porting)

### 1.1 Classification types

Twelve built-in types plus user-defined `Custom` classifiers. Each has an editable prompt (defaults below).

| Type | Default prompt (short) | TSNS operation |
|---|---|---|
| `Axiom` | Core foundational truths that don't rely on prior proof and support other claims | `establish` |
| `Claim` | Positions the author asserts that can be supported or refuted | `derive` |
| `EvidenceBundle` | Data, quotes, citations, or logic supporting claims/axioms; label says what it supports | `anchor` |
| `ScientificProcess` | Processes, methodologies, experimental procedures | `derive` |
| `Relationship` | Explicit or implicit relations between concepts, entities, events | `bridge` |
| `InternalLink` | References to other notes/sections in the vault | none |
| `ExternalLink` | References to external sources, URLs, citations | none |
| `ProperName` | People, places, organizations, specific entities, with context | none |
| `ForwardLink` | Concepts that could be expanded in future notes | none |
| `WordOntology` | Specialized terms with category and definition | none |
| `Sentence` | Key sentences carrying important claims/evidence/concepts | none |
| `Paragraph` | Paragraphs forming logical units, with a summary label | none |
| `Custom` | `{keyword, prompt, enabled}` defined by the user; output carries `customType` | inferred from keyword |

`Custom` operation inference (substring of lowercased `customType`, first match wins):
`challenge`/`attack` -> `challenge`, `declare` -> `declare`, `bridge`/`connect` -> `bridge`, `anchor`/`evidence` -> `anchor`,
`derive`/`claim` -> `derive`, `establish`/`axiom` -> `establish`, otherwise none.

Default type set for a normal run: `Axiom, Claim, EvidenceBundle, Relationship`.
Default type set for a **structural pass**: `Claim, EvidenceBundle, Relationship, ScientificProcess` (Axiom is never in it).

### 1.2 What the model returns

A JSON array. Each item:

```json
{
  "type": "Claim",
  "label": "Renewable energy is more sustainable",
  "parentLabel": "Conservation of Energy",
  "customType": "only when type is Custom",
  "confidence": 0.85,
  "metadata": { "...operation fields...": "...", "entity": "", "role": "", "evidence": "", "time": "", "ops": "", "context": "", "provenance": "" }
}
```

`parentLabel` is resolved to the parent's UUID **within the same response** (exact label match).

### 1.3 TSNS strict mode (metadata contract)

Setting `tsns_render_mode`: `off` | `full` | `strict` (default `strict`).
In `strict`, every tag whose type maps to an operation must carry all operation fields **and** all 7 facets, or the note is **not
written** (skipped and reported).

Required operation fields (canonical key, label):

- `establish`: deathConditionsSurvived, propagationTest, whatShouldKillIt, vulnerability, unlocks
- `derive`: sourceNodes, chain, weakestLink, ifBreaks, vulnerability, unlocks
- `challenge`: target, strongestAttack, survived, damageReport, vulnerability, unlocks
- `bridge`: domainA, domainB, isomorphism, constrainsPredictions, whatWouldMakeThisJustAMetaphor, vulnerability, unlocks
- `anchor`: claimBeingGrounded, dataCited, supportsFullOrWeakerVersion, whatKillsEmpirically, confidence, vulnerability, unlocks
- `declare`: basisForDeclaration, scope, residualRisk, vulnerability, unlocks

Seven facets (all ops): `entity, role, evidence, time, ops, context, provenance`.

**Alias normalization** (fill canonical key from first non-empty alias). Port these tables exactly:

```text
establish: deathConditionsSurvived<-[death_conditions_survived,deathConditions,death_conditions]
           propagationTest<-[propagation_test,propagation]  whatShouldKillIt<-[what_should_kill_it,kill_condition,killConditions]
derive:    sourceNodes<-[source_nodes,sources] chain<-[reasoning_chain,derivation_chain] weakestLink<-[weakest_link]
           ifBreaks<-[if_breaks,failure_mode]
challenge: target<-[targets] strongestAttack<-[strongest_attack,steelman] survived<-[survival,survives] damageReport<-[damage_report,damage]
bridge:    domainA<-[domain_a] domainB<-[domain_b] isomorphism<-[mapping]
           constrainsPredictions<-[constrains_predictions,constrains_both_domains,constrains]
           whatWouldMakeThisJustAMetaphor<-[what_makes_it_just_metaphor,metaphor_kill_condition]
anchor:    claimBeingGrounded<-[claim_being_grounded,claim] dataCited<-[data_cited,evidence]
           supportsFullOrWeakerVersion<-[supports_full_or_weaker_version,support_strength]
           whatKillsEmpirically<-[what_kills_empirically,empirical_kill_condition] confidence<-[confidence_level]
declare:   basisForDeclaration<-[basis_for_declaration,basis] scope<-[scopes] residualRisk<-[residual_risk,risk]
all ops:   vulnerability<-[vulnerabilities] unlocks<-[unlock,unlocked,implications]
facets:    entity<-[facet_entity,entityFacet,whatIsThis,identity] role<-[facet_role,roleFacet,function,whatDoesItDo]
           evidence<-[facet_evidence,evidenceFacet,support,grounding] time<-[facet_time,timeFacet,sequence,temporal]
           ops<-[facet_ops,opsFacet,workflow,status] context<-[facet_context,contextFacet,lens,domain]
           provenance<-[facet_provenance,provenanceFacet,origin,audit]
```

"Non-empty" = bool always counts; finite number; non-empty array/object; string with non-whitespace.
Display formatting: bool -> `Yes`/`No`; array -> joined with `"; "`; null -> empty.

### 1.4 Context injection (locked axioms + labeled examples)

- **Canonical axioms** are read from `axiom-reference.md`, falling back to `reference/AXIOM_MATRIX_LOSSLESS.md`. Each non-blank,
  non-`#` line, strip `^\d+\.\s*` and `^-\s*`, keep text before the first `|`, drop if shorter than 8 chars, dedupe case-insensitively,
  cap 220. Cache for the run.
- **Structural pass plan** (folder-level): parse existing tags from every note in scope. If `lock_axioms`, collect all existing
  `Axiom` labels (cap `max_axioms_in_context`, 1..50, default 12). If `reuse_labeled_context`, collect up to `examples_per_type`
  (1..10, default 3) existing labels per structural type. Union canonical + found axioms (cap 220). **Skip any note that already has a
  structural tag** and report the count.
- Context renders into the prompt as:

```text
Context from already-labeled notes:
### Locked Axioms (Reference Only)
Use these as fixed context. Do not reword or relabel them unless directly quoted in the text.
- <axiom>            (max 50 lines)

### Labeled Examples: <Type>
- <label>            (max 20 per type; Axiom never listed here)

Match existing labeling style when possible.
```

### 1.5 Machine-tag refinements

`machine_tag_prompt_mode`: `off` | `assist` (default). A list of `{code, tag, description, enabled}` rows (upsert by code or tag,
case-insensitive). When on and non-empty, append (max 120 rows):

```text
Machine Tag Refinements (authoritative disambiguation hints):
- Prefer these refinements whenever a listed machine code or canonical tag appears.
- If a match is ambiguous, keep confidence <= 0.6 and avoid over-tagging.
- <code> | <tag> => <description>
```

The machine registry is a TSV: `CODE FAMILY PREFIX YAML_FIELD FULL_CONCEPT CANONICAL_TAG GRAPH_NODE_TYPE ACTIVE`
(e.g. `Pph PILLAR P domain Physics pillar/physics Domain Y`). Load it; use it for refinement lookup and as a future scanner source.

### 1.6 Prompt shapes

Three builders. Keep their wording as **template files** (`prompts/system_multi.md`, `prompts/single_type.md`,
`prompts/custom.md`, `prompts/concept_journey.md`) with placeholders, so David can edit them without recompiling. Defaults must
reproduce the plugin's text: a JSON-array output spec, the type list, a `### <Type>` definition per type, then context, strict
schema, refinements, then:

```text
Analyze the following text and identify all semantic elements according to the definitions above. Return ONLY valid JSON, no other text.

---
TEXT TO ANALYZE:
{content}
---

JSON Response:
```

Strict schema lines per type, e.g.
`- Type "Claim" (operation "derive") metadata schema: {"sourceNodes":"...","chain":"...",...}`;
types with no operation get `include metadata with non-empty "vulnerability" and "unlocks"`.

**Concept journey prompt** (cross-note): given concept, aliases, and occurrences `{file, type, label}` in order, return
`{"narrative": "...", "contradictions": [], "gaps": [], "suggestions": []}`.

### 1.7 Concept registry and index

- **Registry** (`concept-registry.json`, schema `{version, lastUpdated, concepts{normalized -> {uuid,label,normalizedLabel,type,
  firstSeenFile,firstSeenDate,aliases[]}}, uuidIndex{uuid -> normalized}}`): the same label always gets the same UUID across
  the vault. `get_or_create_uuid(label, type, file)` checks the normalized label, then aliases, else mints a v4 UUID.
  Also supports add-alias, merge (keep one, redirect the other), search, stats, export/import (merge or overwrite).
- **Index** (folder or vault): per normalized label, its occurrences `{file, tag uuid, type, label, line}`, total and file
  counts, types seen, and up to 20 related concepts (co-occur in a file). File-to-file relations: shared concepts and
  `strength = shared / max(|A|, |B|)`.
- **Forward links** for a journey: append `%%forward-link::<concept>::[[<next file>]]%%` after the tag block of each occurrence's
  file, skipping files that already have one for that concept.

### 1.8 Cost estimate

`tokens ≈ ceil(chars / 4)`; output estimate = 20% of input. Price table per model (USD per 1M tokens, input/output) lives in config.
Show the estimate and ask before any batch over 10 notes (see section 2).

---

## 2. House rules from `pipeline-workflows\API` (follow them)

The working reference is `01_CKG\020_CKG`. Copy its shape, don't invent a new one.

- **A station is one front folder:** `1 RUN HERE.bat`, `2 RUN ON FOLDER.bat`, `BACKSIDE\` (binary or script, `station.json`,
  `PROMPT.md`, `FOCUS.md`, `README.md`), `INBOX\`, `OUTBOX\`. Nothing else at the top.
- **Never hard-code a folder's position.** Find `_system` by walking up from the executable's folder. Machine paths (vault, NAS)
  come from `_system\config\paths.json`.
- **Providers** come from `_system\config\providers.json`
  (`{name: {base, chat, key: ENV_VAR_NAME, default_model, openai_compatible, no_key?, alias_of?}}`), and **policy** from
  `_system\config\settings.json` (`max_concurrent_calls`, `turbo_presets {"1":10,"2":30,"3":60}`, `default_provider`,
  `models`, `fallback[]`, `allowed_providers[]`, `output_token_cap`, `retry {attempts, base_seconds, max_seconds}`,
  `request_timeout_seconds`). **Read these files; don't duplicate them.** Refuse any provider not in `allowed_providers`.
  If `_system` isn't found, fall back to `semtag`'s own `config\` copies of the same schema.
- **Run-on-folder questions, in order:** which folder (drag-in path) -> which notes (if `_PICK.md` X-list exists: "N ticked of M,
  run those?"; else "M notes, how many?" accepting a number, `all`, or `p` = write an X-list and stop; **more than 10 notes
  with no X-list never goes to the API by accident**; `INBOX\00_PRIORITY\` counts as ticked and runs first) -> output folder
  (default the station OUTBOX) -> **how many at once (1..30, or 60 with turbo)** -> up to 5 extra questions -> confirm with the cost
  estimate -> run, printing a timestamped line per step.
- **Ctrl+C never kills a run** (a dictation tool sends stray ones). Ignore SIGINT during runs; only typing `stop` (on stdin) stops,
  after in-flight calls finish.
- **A reply cut off at the cap (`finish_reason == "length"`) is an error, never a result.** Retry with a continuation, or split.
- **Receipts:** every call logs `{task, provider, model, prompt_tokens, completion_tokens, attempts, elapsed_seconds, started_at,
  finished_at, error, finish_reason, note}` as one JSON line to `_system\LOGS\` (or `OUTBOX\_receipts.jsonl`). Never log keys or
  full prompts.
- **OUTBOX layout:** `OUTBOX\<note> · SEMTAG.md` (the newest full result per note), `OUTBOX\_older\<YYYY-MM-DD>\` for anything
  replaced (**never delete**), `OUTBOX\_last_run.txt` (paths of notes just run, so a layer can follow).
- **Publishing never wipes.** If a tag already on the note is not found again, it stays (append mode is the default).
- **Read the source only:** strip our own blocks (section 6) from the note before sending it to the model.
- **The repo is public:** keys, `INBOX`, `OUTBOX`, `STATE` and `paths.json` are never committed.

---

## 3. Architecture

Rust 2021, a Cargo workspace:

```text
semtag/
  Cargo.toml                 (workspace)
  crates/
    semtag-core/             types, config, tag format parse/write, TSNS, prompts, registry, index. No I/O to the network.
    semtag-llm/              providers, request building, adaptive limiter, retries, fallback chain, receipts
    semtag-cli/              clap CLI, interactive "button" flow, progress output
    semtag-desk/             desktop layer (part B): tray app, global hotkeys, selection popup, action runner
  config/                    default providers.json, settings.json, classifiers.json, pricing.json (used when _system is absent)
  prompts/                   system_multi.md, single_type.md, custom.md, concept_journey.md, types/<Type>.md
  station/                   1 RUN HERE.bat, 2 RUN ON FOLDER.bat, station.json, README.md, FOCUS.md, PROMPT.md
  tests/fixtures/            notes tagged by the plugin (golden files)
```

Crates: `tokio` (multi-thread), `reqwest` (rustls, JSON), `serde`/`serde_json`, `clap` (derive), `anyhow`/`thiserror`,
`uuid` (v4), `regex`, `percent-encoding` (must match JS `encodeURIComponent` exactly: leave `A-Z a-z 0-9 - _ . ! ~ * ' ( )`
unescaped), `rusqlite` (bundled), `csv`, `walkdir`, `indicatif` (optional), `tracing`, `chrono`.

### 3.1 Providers (`semtag-llm`)

One trait, three wire formats:

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, req: &ChatRequest) -> Result<ChatReply, LlmError>;
}
pub struct ChatReply { pub text: String, pub prompt_tokens: u32, pub completion_tokens: u32, pub finish_reason: Option<String> }
```

- **OpenAI-compatible** (`{base}/{chat}`, `Authorization: Bearer`, `messages`, `temperature`, `max_tokens`, optional
  `response_format: {"type":"json_object"}`; if JSON mode is on and no message contains "json", add a system message
  "Reply with a single JSON object."): covers deepseek, openai, openrouter, moonshot, gemini (OpenAI endpoint), groq,
  together, mistral, ollama (`/v1/chat/completions`, no key), **xAI** (`https://api.x.ai`, `v1/chat/completions`, env
  `XAI_API_KEY`), and **LM Studio** (`http://127.0.0.1:1234`, `v1/chat/completions`, no key). Add xai and lmstudio to the
  provider list if missing; Ollama and LM Studio hosts are editable settings.
- **Streaming:** support `stream: true` (SSE) for OpenAI-compatible and Anthropic, for the desktop popup. Batch runs stay non-streaming.
- **Anthropic native** (`POST https://api.anthropic.com/v1/messages`, `x-api-key`, `anthropic-version: 2023-06-01`,
  `max_tokens` required, reply `content[0].text`, `stop_reason == "max_tokens"` = truncated).
- **Custom** (POST `{prompt, model}` to a URL; read `response | content | text | output`, else the raw body).
- **Mock** (no network; deterministic fake JSON for tests).

Defaults: temperature 0.3, `max_tokens` from `output_token_cap` (8000), timeout from settings (300 s).

### 3.2 Concurrency (the part David cares most about)

Settings, CLI flags, and the button question all set the same three knobs:

| Knob | Values | Meaning |
|---|---|---|
| `workers` | 1..60 (turbo presets 1=10, 2=30, 3=60) | max API calls in flight across the whole run |
| `parallel` | `notes` (default) / `types` / `off` | `notes`: one multi-type call per note, notes run side by side. `types`: one call **per type per note**, all in parallel (more calls, smaller outputs, fewer truncations). `off`: strictly one call at a time, in order. |
| `delay_ms` | 0.. (default 0; plugin used 500 serially) | minimum gap between call starts (polite mode) |

Implement an **AdaptiveLimiter** (port of `_system\engine\llm.py`): a semaphore whose ceiling starts at `workers`; record
success/failure per call in a 60 s window; when ≥10 events and failures > 10%, halve the ceiling (min 1) and clear the window;
when no halving happened for 60 s and ceiling < max, add 1. Log every change with a timestamp.

**Retries:** `attempts` (3) with exponential back-off `base_seconds * 2^n`, capped at `max_seconds`, with jitter, on
429 / 5xx / timeouts / connection errors; honor `Retry-After`. 4xx other than 429 is not retried. Then walk the `fallback` chain
`[{provider, model}]` if configured.

**Pipeline:** a bounded producer (note reader) -> N async workers -> **one single writer task** that receives results over a channel
and writes each note to disk the moment its result arrives (tags on note, OUTBOX file, registry update, receipt). The registry is owned
by the writer, so there are no races on UUIDs. `parallel = types` merges all per-type results for a note before writing.

**Resume:** before calling, skip notes whose current source hash (SHA-256 of the stripped source + type set + prompt version)
matches `STATE\semtag_done.json`, unless `--redo`.

### 3.3 Parsing model replies

Port the plugin's tolerant parser, then make it stricter:

1. Trim; if a ```` ```json ```` fence exists take its inside.
2. Take the outermost `[ ... ]` if present.
3. `serde_json` parse; a single object becomes `[obj]`.
4. On failure, salvage every flat `{...}` that parses (the plugin's fallback).
5. Drop items missing `type` or `label`; drop unknown types (log them); coerce `confidence` to 0..1.
6. Truncated reply (step "finish_reason") -> error, never parsed.

### 3.4 Fold mode: always expanded / always collapsed

Setting `fold`: `collapsed` (default, matches the plugin) | `expanded`, overridable per block:
`fold_blocks: { "inline_labels": "...", "tsns_index": "...", "tsns_items": "...", "chips": "..." }`.

- HTML `<details>` blocks: collapsed = `<details ...>`, expanded = `<details ... open>`.
- Obsidian callouts: collapsed = `> [!type]-`, expanded = `> [!type]+`.

Command `semtag fold <path> --expand|--collapse [--block NAME]` rewrites only those markers in existing notes (all notes under a
folder, recursive), **no API call**, with an OUTBOX `_older` backup first.

---

## 4. Configuration file: `classifiers.json`

Holds everything the plugin kept in `data.json` except the key. Example:

```json
{
  "prompt_version": "2026-10-12",
  "types": {
    "Axiom": { "prompt_file": "prompts/types/Axiom.md", "enabled": true },
    "Claim": { "prompt_file": "prompts/types/Claim.md", "enabled": true }
  },
  "custom": [ { "id": "custom-1712345", "keyword": "Challenge", "prompt": "Find the strongest objections...", "enabled": true } ],
  "default_types": ["Axiom", "Claim", "EvidenceBundle", "Relationship"],
  "structural_types": ["Claim", "EvidenceBundle", "Relationship", "ScientificProcess"],
  "tsns_render_mode": "strict",
  "lock_axioms": true,
  "reuse_labeled_context": true,
  "max_axioms_in_context": 12,
  "examples_per_type": 3,
  "axiom_sources": ["axiom-reference.md", "reference/AXIOM_MATRIX_LOSSLESS.md"],
  "machine_tag_prompt_mode": "assist",
  "machine_tag_registry": "reference/THEOPHYSICS_MACHINE_TAG_REGISTRY_v1.0.tsv",
  "machine_tag_refinements": [],
  "write": { "tag_block": true, "inline_labels": true, "tsns_structure": true, "frontmatter_counts": true, "mermaid": "panel" },
  "fold": "collapsed",
  "graph_direction": "TD"
}
```

Also: `semtag import-plugin <path to data.json>` converts the plugin's `data.json` into this file and writes each prompt to
`prompts/types/<Type>.md`. **It must not copy `apiKey`;** print the env-var name the key belongs in instead.

---

## 5. CLI

```text
semtag here                         # station INBOX (button flow, like "1 RUN HERE")
semtag folder [PATH]                # button flow on any folder ("2 RUN ON FOLDER")
semtag classify <PATH...> [--types Axiom,Claim] [--custom KEYWORD] [--single-type]
                                    [--workers N | --turbo 1|2|3] [--parallel notes|types|off]
                                    [--provider P] [--model M] [--strict|--full|--no-tsns]
                                    [--replace] [--redo] [--yes] [--dry-run] [--expand|--collapse]
semtag structural <FOLDER>          # locked-axiom structural pass (skips notes that already have structural tags)
semtag estimate <PATH...>           # tokens + cost only
semtag index <FOLDER|--vault>       # build concept index -> SQLite + JSON
semtag concepts search <QUERY> | stats | export | import <FILE> [--overwrite] | merge <KEEP> <MERGE> | alias <LABEL> <ALIAS>
semtag journey <CONCEPT> [--forward-links]   # concept evolution analysis (1 call) + optional forward links
semtag csv <NOTE|FOLDER>            # theory CSV export
semtag mermaid <NOTE> [--direction TD|LR|BT|RL] [--append]
semtag fold <PATH> --expand|--collapse [--block NAME]
semtag show-tags <NOTE>             # print parsed tags (the plugin's "show hidden tags")
semtag strip <PATH>                 # remove all semtag blocks (backup to _older first)
semtag test-provider [--provider P] # sends {"test":"success"} probe
semtag import-plugin <data.json>
```

`--dry-run` prints the exact prompt for the first note and the plan, and makes no call.

---

## 6. Output formats on the note (the contract)

Write them in this order relative to the note. All four are idempotent: rerunning removes and rewrites them.

### 6.1 Hidden tag block (source of truth, end of note)

```text


%%--- SEMANTIC TAGS ---%%
%%tag::<Type or Custom:<customType>>::<uuid>::"<label>"::<parentUuid or null>[::meta=<encodeURIComponent(JSON)>]%%
...
%%--- END SEMANTIC TAGS ---%%
```

Parse with (per line, global): `%%tag::([^:]+)::([^:]+)::"([^"]+)"::([^:%]*)(?:::(?:meta=)?([^%]*))?%%`.
`meta` is omitted when metadata is empty. **Label sanitization** (bug fix, see section 8): before writing, replace `"` with `'`,
`%%` with `%`, newlines with spaces, so the regex always round-trips.

Append mode (default): parse existing tags, keep them all, add new tags whose UUID isn't present. `--replace` rewrites the block.

### 6.2 Inline labels block (after the first `# ` heading after front matter; else before the tag block; else at the end)

```html
<!-- SEMANTIC INLINE LABELS START -->
<details class="semantic-ai-inline-labels">
<summary><strong>Semantic Labels</strong> (click to show/hide)</summary>

Total tags: N

**<TypeGroup> (n)**
<div class="semantic-word-grid">
<details class="semantic-word semantic-word--<axiom|claim|evidence|relationship|mastereq|generic>">
<summary><span class="semantic-word__label">LABEL</span></summary>
<div class="semantic-word__panel">
<div class="semantic-word__row"><strong>Type:</strong> TYPE</div>
<div class="semantic-word__row"><strong>Parent:</strong> PARENT</div>   (only if parent)
<div class="semantic-word__row"><strong>Entity:</strong> ...</div>      (Role, Evidence, Time, Ops, Context, Provenance: each only if non-empty)
</div>
</details>
</div>

</details>
<!-- SEMANTIC INLINE LABELS END -->
```

Group order: Axiom, Claim, EvidenceBundle, Relationship, ScientificProcess, InternalLink, ExternalLink, ProperName, ForwardLink,
WordOntology, Sentence, Paragraph, then every `Custom:*` group, then anything else. HTML-escape `& < > "`. Dedupe tags by UUID
(or `type::label::parent::customType` when UUID is missing). Chip class: establish -> axiom, anchor -> evidence,
bridge -> relationship, derive -> claim, Custom containing "master" -> mastereq, else generic. Fold mode adds `open`.

### 6.3 TSNS structure block (when `tsns_render_mode` is `full` or `strict`; same insertion point)

```text
<!-- TSNS STRUCTURE START -->
> [!abstract]- 📋 Structural Index
> 
> **Primary Operation:** <op with most items, UPPERCASE>
> **Total Structured Nodes:** N
> 
> **Axioms Established:**
> - [[#^A1]] — <label>          (or "> - (none)")
> 
  ... same for Claims Derived (derive), Evidence Anchored (anchor), Bridges Claimed (bridge),
      Challenges Addressed (challenge), Declarations (declare)

> [!establish]- **A1 — Axiom: <label>**
> **Death conditions survived:** <value or TODO>
> ...one line per required field...

^A1
<!-- TSNS STRUCTURE END -->
```

Anchor prefixes: establish A, derive C, anchor E, bridge B, challenge X, declare D (numbered per prefix). Callout titles: Axiom,
Claim, Evidence, Relationship, Challenge, Declaration. Fold mode switches `-` / `+`.

### 6.4 Front-matter counts (only if the note has front matter)

Replace or add: `axiom_count, claim_count, evidence_count, bridge_count, anchor_count, challenge_count, declare_count`.
Keep every other front-matter line untouched (don't reserialize the YAML; edit lines).

### 6.5 Mermaid

```text
graph TD
  n0["<Type>: <label: " -> ', [ -> (, ] -> ), max 50 chars>"]
  n0 --> n1        (parent -> child)
```

Written to `OUTBOX\<note> · GRAPH.md` by default; `--append` puts it in a ```` ```mermaid ```` block on the note.

---

## 7. Side outputs

- **Theory CSV** (`<csv folder or note folder>\<note>.theory.csv`, never overwrite: add `-2`, `-3`):
  `theory,note_path,note_name,tag_index,tag_type,tag_type_index,is_axiom,axiom_number,uuid,label,parent_uuid,parent_label`
  (`theory` = basename with `_` -> space; RFC 4180 quoting; newlines -> space).
- **JSONL** per run: one line per tag `{note, type, customType, uuid, label, parentUuid, confidence, operation, metadata}`.
- **SQLite** (`_data\semtag.sqlite`, or `OUTBOX\semtag.sqlite`): tables `concepts(uuid PK, label, normalized, type, first_file,
  first_date)`, `aliases(alias PK, uuid)`, `tags(uuid, note, type, custom_type, label, parent_uuid, confidence, operation,
  metadata_json, line, run_id)`, `relations(note_a, note_b, shared, strength)`, `runs(run_id, started, provider, model, workers,
  parallel, notes, calls, tokens_in, tokens_out, est_cost, errors)`. `concept-registry.json` is still written for the plugin.
- **OUTBOX** `<note> · SEMTAG.md`: the note's title, a link back to it, the counts, the inline labels block, the TSNS block, the Mermaid
  graph, and any strict-mode failures.

---

## 8. Plugin bugs to fix (don't port them)

1. Batch runs were **strictly serial with a 500 ms sleep**: replace with section 3.2.
2. **No retry/back-off and no truncation check** (`max_tokens` 4096, cut-off JSON was half-salvaged as a result).
3. **Anthropic endpoint and the concept-journey call were duplicated** in a second code path that ignored `custom`: one provider layer.
4. **Labels containing `"` or `:` break the tag regex** (silently lost on reread): sanitize on write, and test the round trip.
5. **Two different label normalizers**: the registry keeps `-` (`[^\w\s-]` removed) and the index strips it (`[^\w\s]`). Use one:
   lowercase, trim, drop chars other than letters, digits, `_`, whitespace, `-`, collapse whitespace. Make this Unicode-aware
   (`\w` in JS was ASCII-only, so Greek letters like χ were deleted; keep them).
6. `parentLabel` only resolved within one call: also resolve against existing tags on the note and against the registry.
7. `evidence_count` and `anchor_count` are the same number: keep both keys for compatibility, document it.
8. File-to-file relations are O(n²) over all notes: build an inverted index (concept -> notes) and only pair notes sharing a concept.
9. Strict mode blocked the whole note silently in batch: write the failures to the OUTBOX report and receipts, and offer
   `--strict-retry` (re-ask the model once for only the missing fields of the failing tags).
10. Folder filter used `path.startsWith(folder)` so `Notes2` matched `Notes`: compare path components.
11. API key stored in plain `data.json`: env vars only.

---

## 9. Acceptance tests (must pass)

1. **Golden round trip:** for fixture notes tagged by the plugin, `parse -> write (append, no new tags)` produces identical bytes.
2. **Encoding:** `meta=` produced by Rust equals JS `encodeURIComponent(JSON.stringify(obj))` for fixtures containing quotes,
   `%`, `&`, `χ`, emoji, and newlines.
3. **Label with `"` and `:`** survives write -> parse.
4. **Strict mode:** a Claim missing `weakestLink` blocks the note; the alias `weakest_link` satisfies it.
5. **Concurrency:** with the mock provider set to 20% failures and `workers = 30`, the limiter halves, the run completes, every
   note is written exactly once, and the receipt count equals calls made.
6. **Truncation:** a mock reply with `finish_reason: "length"` is reported as an error and never written.
7. **Resume:** kill mid-run (simulate), rerun: finished notes are skipped by hash; unfinished ones run.
8. **Fold:** `fold --expand` then `fold --collapse` returns the original bytes.
9. **More than 10 notes with no X-list** never calls the API without explicit confirmation.
10. **Ctrl+C** during a run doesn't stop it; typing `stop` drains in-flight calls and exits cleanly.
11. `cargo clippy -- -D warnings` and `cargo test` clean; builds a single `semtag.exe` on Windows (MSVC) with no runtime dependencies.

---

## 10. Milestones (deliver in this order, each one runnable)

1. `semtag-core`: types, tag block parse/write, inline labels, TSNS block, front-matter counts, Mermaid, CSV, golden tests.
2. `semtag-llm`: providers, limiter, retries, receipts, mock; `test-provider`, `estimate`, `classify` on one note.
3. Batch + structural pass + resume + single writer; button flow (`here`, `folder`) with X-lists and the confirm step.
4. Registry + index + SQLite + concept search/journey/forward links.
5. Fold command, `import-plugin`, `strip`, station folder with the two `.bat` launchers (walk-up block like the existing ones).
6. Desktop layer, core: tray app, palette hotkey, selection capture and replace, `ai` actions with streaming popup, history.
7. Desktop layer, rest: per-action hotkeys, recognitions, compare mode, `script` / `search` / `semtag` actions, selection
   popup, exclusions, action import/export.

At each milestone, show the `cargo test` output and one real run on two small notes with the mock provider.

---

# Part B: the desktop layer (`semtag-desk`)

## 11. What it is

A small Windows tray app that puts every connected AI on a shortcut **anywhere on the desktop**: select text in any program
(browser, Word, Obsidian, email, a PDF), press a hotkey, and an action runs on it: rewrite, summarize, explain, translate, tag,
or send it to a station. The result can **replace the selection in place**, appear in a popup, go to the clipboard, or land in a
note. Model it on the selection-action tools David already uses (see the feature list below), but driven by the same
`semtag-llm` core, `providers.json` and receipts as part A.

Use **Tauri 2** for the tray, the popup and the settings window (plugins: `global-shortcut`, `clipboard-manager`,
`single-instance`, `autostart`), with `enigo` (or `windows` crate `SendInput`) for simulated keystrokes and `keyring` for keys.
One process, starts with Windows (optional), single instance.

### 11.1 Settings window (same sections as the reference tool)

- **Custom recognitions**
  - **Classification model:** an optional cheap model (local Ollama/LM Studio by default) that labels a selection
    (`url, code, scripture_ref, equation, email, prose, list, table, foreign_language, ...`) so the matching actions are listed first.
    Off by default; never blocks the popup (show rules-based ordering at once, reorder when the label arrives).
  - **Regular expressions:** `{name, pattern, label}` rules that run first, locally, e.g. a Bible reference
    (`\b(?:[1-3]\s?)?[A-Z][a-z]+\.?\s\d+:\d+(?:-\d+)?\b`) -> `scripture_ref`, a URL -> `url`, a `χ`/LaTeX block -> `equation`.
- **Custom actions**
  - **AI conversation:** prompt templates (below).
  - **Script execution:** run a local command with the selection on stdin or in `{{selection_file}}`; output handled like an AI
    action. Default to David's Python (`py -3`), with `pipeline-workflows\API\_ACTIONS\actions\*.py` listed as ready-made scripts.
  - **Web search:** open a URL template (`https://www.google.com/search?q={{selection_urlencoded}}`, Bible Gateway, Wikipedia, arXiv,
    Google Scholar). No API call.
  - **Semtag:** run part A on the selection or the current note (section 11.4).
- **Mouse settings:** show a small floating button after a mouse text selection (on/off, delay ms, minimum characters,
  hold-modifier-to-show, excluded apps).
- **Model provider options:** Ollama host, LM Studio host, and keys for OpenRouter, OpenAI, Anthropic, Gemini, xAI, DeepSeek,
  Moonshot, Groq, Together, Mistral. **Keys go to Windows Credential Manager (`keyring`) or env vars, never into a JSON file.**
  A "Test" button per provider. Model list fetched from `/v1/models` where the provider has it, free text otherwise.
- **General:** start with Windows, palette hotkey, theme, history on/off, default output mode, max calls at once (shared limiter).

### 11.2 Action definition (`actions.json`, import/export as one file)

```jsonc
{
  "id": "rewrite-plain",
  "name": "Rewrite plainly",
  "icon": "pencil",
  "kind": "ai",                        // ai | script | search | semtag | chain
  "provider": "deepseek",              // any provider from providers.json, or "compare"
  "model": "deepseek-chat",
  "compare": [ {"provider":"openai","model":"gpt-4o-mini"}, {"provider":"anthropic","model":"claude-sonnet-5"} ],
  "system": "You are David's editor. Keep his voice.",
  "prompt": "Rewrite this so a high-school reader follows it. Keep every claim.\n\n{{selection}}",
  "output": "popup",                   // replace | insert_below | copy | popup | append_to_note | new_note | station_inbox
  "hotkey": "Ctrl+Alt+R",
  "show_for": ["prose"],               // recognition labels; empty = always
  "temperature": 0.3,
  "max_tokens": 2000,
  "stream": true,
  "json_mode": false
}
```

Template variables: `{{selection}}`, `{{clipboard}}`, `{{selection_urlencoded}}`, `{{selection_file}}`, `{{app}}` (foreground
process name), `{{window_title}}`, `{{date}}`, `{{note_path}}` (when the foreground window is Obsidian: resolve the vault from
`paths.json` and the note from the window title), `{{input}}` (asks a one-line question first), and `{{file:relative/path.md}}`
(inline a reference file, e.g. `axiom-reference.md` or a voice guide). Unknown variables are an error at save time, not at run time.

A `chain` action runs a list of action ids in order, each getting the previous output as `{{selection}}`.

Ship these defaults: Rewrite plainly, Tighten, Fix grammar only, Explain, Summarize in 3 bullets, Translate to English,
Steelman objection, Find the weakest link, Classify (semtag types, popup), Tag this note (semtag), Send to station INBOX,
Look up scripture (search), Search Scholar (search).

### 11.3 How it runs

1. **Trigger:** an action's own hotkey, the **palette hotkey** (default `Ctrl+Alt+Space`: a searchable list of actions, recognized
   ones on top, number keys to pick), or the floating mouse button.
2. **Capture:** remember the foreground window; save the clipboard (all formats where possible); send `Ctrl+C`; wait for the
   clipboard sequence number to change (timeout 400 ms); read text; **restore the original clipboard**. If nothing was selected,
   offer the clipboard text instead.
3. **Run** through `semtag-llm` (same limiter, retries, truncation rule, receipts as part A, task id `DESK-<action id>`).
   Stream tokens into the popup.
4. **Popup:** shows the result as it streams, with **Replace**, **Insert below**, **Copy**, **Retry**, **Switch model**
   (dropdown, reruns), **Edit prompt** (one-off), and **Esc** to close. In compare mode, one column per model, all calls in
   parallel, with tokens, time and cost under each; pick one to Replace.
5. **Replace / insert:** refocus the remembered window, put the result on the clipboard, send `Ctrl+V` (insert below = `End`,
   `Enter`, then paste), then restore the clipboard. If the window is gone or refuses focus, fall back to Copy and say so.
6. **History:** every run appended to `history.jsonl` (action, app, model, tokens, cost, time, first 200 chars of input and
   output; full text only if "keep full history" is on). A History tab can rerun any entry.

### 11.4 Bridges to part A and the API workbench

- **Classify selection:** runs the part-A classifier on the selected text, shows the tags as chips (same classes as 6.2).
- **Tag this note:** when Obsidian is in front, runs `semtag classify` on `{{note_path}}` and writes the blocks (section 6).
- **Send to station:** saves the selection as `INBOX\00_PRIORITY\<date> <first words>.md` in a station chosen from
  `_system\config\stations.json`, so the next `1 RUN HERE` picks it up first.
- **Run an `_ACTIONS` job:** list `_ACTIONS\actions\*.py` (read each one's `ABOUT`) as script actions.

### 11.5 Safety rules

- Nothing is ever sent unless David triggers an action. No background reading of selections or the clipboard.
- **Excluded apps** (default: password managers like KeePass/1Password/Bitwarden, `mstsc`, `credentialuiBroker`, Windows
  Security): hotkeys and the mouse button do nothing there.
- A per-action **local only** flag restricts it to Ollama/LM Studio.
- Show the provider and model on every popup so it's always clear where the text went.
- Elevated (admin) windows can't receive simulated keys from a normal process: detect it and fall back to Copy with a notice.

### 11.6 Desktop acceptance tests

1. Select text in Notepad, run "Fix grammar only" with the mock provider and Replace: the text is replaced, and the clipboard holds
   what it held before.
2. No selection: the action offers the clipboard text instead of sending an empty prompt.
3. Compare mode with three models runs three calls at once, and the limiter counts them.
4. An action in an excluded app does nothing and logs why.
5. Hotkey conflicts are detected at save time and named.
6. Killing a provider mid-stream shows the error in the popup; nothing is pasted.
