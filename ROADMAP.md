# semtag / Rust AI: roadmap and decisions

The full build spec is `README.md`. This file tracks what is built, the calls made where the spec was
ambiguous, and where the desktop layer is headed.

## Status

| Milestone | What | State |
|---|---|---|
| 1 | `semtag-core`: tag block, inline labels, TSNS, front-matter counts, Mermaid, CSV, fold, golden tests | done |
| 2 | `semtag-llm`: providers, adaptive limiter, retries, receipts, mock → **provided by `forge-core`** (router, limiter, ledger, receipts, mock). Remaining: `test-provider`, `estimate`, `classify` in semtag-cli on top of forge-core | next |
| 3 | Batch + structural pass + resume + single writer; button flow | |
| 4 | Registry + index + SQLite + journey / forward links | |
| 5 | `import-plugin`, station folder and `.bat` launchers (fold and strip already exist in M1) | |
| 6 | Desktop core: tray, palette hotkey, selection capture/replace, streaming popup, **extension host** | |
| 7 | Desktop rest: per-action hotkeys, recognitions, compare mode, mouse button, import/export | |

Run it now (no API call needed):

```text
cargo test
cargo run -p semtag-cli -- apply  <note.md> --tags reply.json [--in-place] [--expand] [--tsns strict|full|off]
cargo run -p semtag-cli -- show-tags | mermaid | csv | strip <note.md>
cargo run -p semtag-cli -- fold <file-or-folder> --expand|--collapse [--block chips]
```

## Spec gaps found while building M1 (decisions made; check against the real plugin)

1. **The plugin regex in 6.1 can't parse its own output.** `([^%]*)` for `meta` stops at the first
   `%`, and percent-encoded JSON always contains `%7B%22`. `([^:]+)` for the type can't match
   `Custom:<x>` either. semtag parses line by line with a looser regex that accepts both. Either the
   regex in the spec was transcribed differently from the plugin's code, or the plugin silently drops
   every tag that has metadata on reread (that would be bug #12). **Real plugin-tagged notes settle it.**
2. **Golden fixtures are hand-made** from section 6, not from the plugin. Byte compatibility with the
   plugin is unproven until 3-5 real tagged notes go into `crates/semtag-core/tests/fixtures/`.
   Untouched tag lines are re-emitted verbatim (raw `meta` kept), so existing tags can't drift.
3. Blank-line layout: display blocks go right after the H1 as `\n<block>\n\n<block>\n`; tag block is
   `trimmed body + "\n\n\n" + block + "\n"`. Anything after the END marker (forward links) is kept.
4. TSNS callouts list the operation fields plus `vulnerability` / `unlocks`; the 7 facets show in the
   inline chips, not in the callouts. The block is skipped when no tag has an operation.
5. TSNS primary-operation ties go to the earlier operation (establish, derive, anchor, bridge, challenge, declare).
6. CSV: indexes are 1-based, `is_axiom` is `true`/`false`, `axiom_number` is blank for non-axioms, `\n` line ends.
7. `parentLabel` matches on the normalized label (not exact), in the same reply first, then tags already on the note.
8. A model item's top-level `confidence` is clamped to 0..1 and kept for JSONL/SQLite; it isn't written into the tag line.
9. `evidence_count` and `anchor_count` are the same number (both count `anchor` tags), kept for compatibility.
10. CRLF notes are processed as LF and written back as CRLF.

## Desktop layer: extensions (direction for M6-M7)

The desktop app is meant to be the go-to AI for everything outside other programs, so actions are
**extensions** loaded from folders, not a fixed list of kinds. Build the extension slot first in M6,
then ship the current kinds (`ai`, `script`, `search`, `semtag`, `chain`) as built-in extensions.

Each extension is a folder with `extension.json`:

```jsonc
{
  "id": "web-research",
  "name": "Research a topic",
  "entry": { "kind": "script", "command": "py -3 research.py" },   // or "js", "wasm", "ai", "mcp"
  "capabilities": ["network", "llm"],                               // declared up front, shown at install
  "inputs": ["selection", "clipboard", "screenshot", "input"],
  "outputs": ["popup", "replace", "new_note", "station_inbox"]
}
```

Capabilities, from least to most trust:

| Capability | Example extensions | Rule |
|---|---|---|
| `llm` | rewrite, summarize, classify | goes through `semtag-llm` (limiter, receipts) |
| `network` | web search, research a topic (search -> fetch -> summarize -> cite) | URL allow-list per extension |
| `files` | send to station, write note | limited to paths in `paths.json` |
| `screen` | share screen / region / window as an image to a vision model | capture only on a hotkey press; preview before send |
| `control` | operate the computer (click, type, open apps) | off by default, asks every time, visible "AI is driving" banner, `Esc` kill switch, every step logged |

Scripts can be Python or JavaScript (the reference tool's script actions use `function process(data)`
with `data.selection` / `data.clipboard`); semtag passes the same `data` object to both.
MCP servers can also be loaded as extensions, so existing connectors plug in without new code.
