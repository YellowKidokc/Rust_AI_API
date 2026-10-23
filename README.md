# forge

One Rust core for every AI call you make: a router over DeepSeek, Claude, OpenRouter, Gemini, Ollama, LM Studio and any
OpenAI-compatible API; one SQLite ledger; one context library; Markdown job templates. Three front doors sit on that core:

```
            HTML pages            other AIs              you
   (Atom Builder, Reconciliation) (Claude, Cursor…)   (terminal, .bat)
                 │                      │                   │
         forge serve (HTTP)        forge mcp (stdio)    forge run / ask
                 └──────────────┬───────┴───────────────────┘
                          forge-core (library)
     router · cascade · adaptive limiter · ledger · context (FTS5) · templates · pipeline · candidates
                 ┌──────────────┼──────────────┬──────────────┐
             DeepSeek       Anthropic      OpenRouter / Gemini   Ollama / LM Studio
```

Front ends are swappable. The HTML pages, a future desktop tray app (`semtag-desk`), and the `semtag` tagger
(`docs/semtag-build-prompt.md`) all call the same core, so keys, retries, logging and cost live in one place.

## Quick start (Windows)

```bat
cargo build --release                       :: or download forge.exe from the CI artifact "forge-windows-x64"
target\release\forge.exe init C:\Forge      :: writes config\, templates\, INBOX\, OUTBOX\ (never overwrites)
setx DEEPSEEK_API_KEY "sk-..."              :: keys are env vars only; open a new terminal after setx
cd C:\Forge
forge doctor                                :: what was found, which providers are ready
forge ask "Steelman the strongest objection to Law 9's Noether argument"
forge run INBOX --job summarize --dry-run   :: see the exact prompt, call nothing
forge run INBOX --job summarize
```

Config lookup: `--home <dir>`, else `FORGE_HOME`, else walk up from the current folder and then from `forge.exe`'s folder
looking for `config\providers.json` or `_system\config\providers.json` (the API workbench's existing files work as-is),
else built-in defaults.

## Taskbar menu: `forge-tray`

`forge-tray.exe` (built with forge; in the `forge-windows-x64` CI artifact) puts the pipeline in the Windows taskbar.
Its menu is built from forge's `GET /api/stations`, which runs the pipeline's own `_system\engine\console.py list`
(the one station feed: nothing is typed into forge or the tray):

```
Open workspace                      forge's page (ui/api-workspace.html)
01 · CKG  >  020_CKG  >  1 RUN HERE / 2 RUN ON FOLDER / Open folder
...
10 · FIS  >  101_FIS_INVENTORY  >  1 RUN HERE / 2 RUN ON FOLDER / Open folder
Front stations  >  000_QUICK_CALL ...
Refresh · Quit
```

A click runs exactly that station's `.bat`, in its own console, the same as double-clicking it. The OpenAI stations are
left out (as on the page). Setup: start `forge serve` with `PIPELINE_SYSTEM` set to the pipeline's `API\_system` (or
start forge from inside `API\`, which it then finds by itself), then run `forge-tray.exe`. `FORGE_URL` points the tray at
another port. `forge-tray --print` prints the same menu as text.

## Routes: DeepSeek by default, free when you ask

A route is an ordered ladder in `config/settings.json`. forge tries each rung in order and moves down on a 429, 5xx,
timeout, refusal, or a reply cut off at the token cap (a truncated reply is an error, never a result).

| route | ladder | use |
|---|---|---|
| `default` | deepseek-chat → claude-sonnet-5-5 | everyday work |
| `reasoning` | deepseek-reasoner → claude-opus-5-5 | hard derivations |
| `free` | OpenRouter `:free` → Gemini free tier → Ollama → deepseek-chat | when you'd rather not pay |
| `local` | Ollama → LM Studio | offline / private |
| `premium` | claude-opus-5-5 → deepseek-chat | final passes |
| `test` | mock | no network, no cost |

`forge ask --route free "..."`, `forge run INBOX --route free`, or `"route": "free"` from the pages / MCP.
`--max-tier free` refuses anything that costs money.

**A note can pick its own route** in its front matter: `route: reasoning`, or words in `priority`, `compute`, `mode`,
`status` or `tags` matched against the `hints` list in `settings.json` (first rule wins). Defaults: `deep` → reasoning,
`premium`/`final` → premium, `private` → local, `background`/`draft`/`low` → free. Precedence: `--route` > the note's
front matter > the template's `route:` > `default_route`. `--dry-run` prints which route each note will take and why.

**Repeat requests are free.** An identical request (same route, messages, mode and limits) is answered from the ledger's
cache and logged as `CACHED` at $0; cached answers don't count against free-tier quotas. `--redo`, `--no-cache`, or
`no_cache: true` forces a fresh call; `cache.enabled: false` turns it off.

A rung is skipped (and `forge plan --route X` says why) when: its key is missing, it's not in `allowed_providers`, it got a
429 and is cooling down, its `daily_requests` quota is used (counted from the ledger), it's outside its `window_utc`, the
day's spend hit `budget.daily_usd`, or the prompt won't fit its `context_window`. Prices in `settings.json` are USD per 1M
tokens; check them against each provider's current price list, the ledger's cost figures are only as good as those numbers.

## The ledger

`_data/forge.sqlite` (next to `config/`) holds every call (provider, model, rung, attempt, status, tokens, cost, latency,
prompt SHA-256, output), the response cache, lineage (which version of a source, by SHA-256, produced which output
through which job and call), each document's state (source hash, last job, last output, notes, history), the context library,
and the candidate graph. `LOGS/forge_receipts.jsonl` gets one line per call without prompt text. Full prompts are stored
only if `ledger.store_prompts` is true.

```
forge ledger recent | forge ledger stats --days 7 | forge ledger archive --days 30   (archive marks, never deletes)
```

## Context library (what replaces feeding 40 files to every call)

```
forge context import axiom-reference.md --kind axiom --lines   :: one snippet per line
forge context import D:\Vault\00_Canonical --kind canon          :: one snippet per heading, recursive
forge context search noether parity
```

Each pipeline run searches the library with the document's key terms and injects the top `context_k` matches into the
template's `{{ context }}`. Search is SQLite FTS5 (BM25): no extra service, exact-term friendly, good for named laws and
axioms. Embedding search can be added behind the same `context_search` call later if keyword recall proves too narrow.

## Templates

`templates/<job>.md`: front matter + a minijinja body. Variables: `document`, `name`, `path`, `context`, `history`
(the previous result for this document and job), `date`. Unknown variables are an error. Ships with `default`,
`summarize`, `extract-claims` (JSON mode). Add a file, run `--job <name>`; no recompile.

## INBOX → OUTBOX

`forge run <folder|file> --job <name>`: `00_PRIORITY\` first; unchanged documents are skipped (same text + same template =
same hash) unless `--redo`; more than 10 documents or an estimate over $0.50 asks first; output is
`OUTBOX\<name> · <SUFFIX>.md` with provenance front matter; a replaced output moves to `OUTBOX\_older\<date>\`. Ctrl+C
is ignored during a run; type `stop` to let in-flight documents finish and exit. `station\` has the `.bat` launchers.

## Clipboard librarian

Point it at the folder your clipboard app drops clips into (one `.txt`/`.md` per clip, local or on the NAS):

```
forge clips sort \\NAS\clips\inbox --library \\NAS\clips\Library --max-tier free   :: hourly: file new clips
forge clips review \\NAS\clips\Library --route default                            :: every 8h: DeepSeek report
```

- Each new clip becomes `Library\<category>\<title>.md` with kind, tags and a one-line summary. Categories are the
  model's call; it sees the existing ones and reuses them, so the library settles into your own shape over time.
- Anything that looks like a password, API key, private key or card number goes to `Library\_held\` and is
  **never sent to a model**.
- Originals are never moved or deleted; already-filed clips are skipped by hash.
- `INDEX.md` lists every category with titles and summaries. `review` writes `Library\_review\<date>.md`
  (merge/rename suggestions, misfiled clips, what stands out, likely junk) and changes nothing.
- `station\CLIPS SETUP.bat` sets the two folders, shows a dry run, and registers both as Windows scheduled tasks.

## Permanent paper ids, focus, staged runs

**Permanent ids.** Every paper gets `forge_uid: <ULID>` in its front matter the first time a run (or
`forge uid backfill FOLDER --apply`) sees it. The id survives edits, renames and moves; content hashes
stay underneath as versions, and hashes ignore the `forge_uid:` line, so stamping a paper doesn't
make it look edited. With no front matter id, forge matches by the same path, then by identical
content, and otherwise mints a new id. It never matches on "looks similar". Every case except a front matter hit is
listed by `forge uid orphans` for review. `forge uid show <uid>` lists every version and output.
Calls, document state and lineage rows all carry the uid.

**GLOBAL_FOCUS.md.** If `GLOBAL_FOCUS.md` sits next to `BRIEF.md` (or `settings.focus_file` names
another file), it is prepended as a system block to **every** call: pipeline, `ask`, HTTP, MCP.
It is read on every call, so an edit applies to the next call. The focus is part of the cache key, so changing it
never serves a stale $0 reply. Each ledger row records `focus_sha256` (`off` with `forge ask --no-focus`).

**Staged runs.** `forge runs start INBOX --stage summarize,extract-claims --stage review --mode first-then-approve`.
Jobs inside a stage run side by side; the next stage waits; a failed job marks that paper's later
stages `blocked`. Modes: `all`, `first-then-approve` (stop after paper 1 until `forge runs approve <id>`),
`step` (approve every paper). `pause`, `resume` and `cancel` act at the next stage boundary and never kill a
call in flight. Run state lives in SQLite. If `forge serve` restarts, it parks runs that were left
`running` as `paused`, so nothing spends money again until you resume. The confirmation above 10 docs or $0.50 still
applies. `forge runs grid [id]` prints the per-paper status grid. Every stage reads the original paper;
stage outputs don't feed the next stage yet.

**The workspace page.** `forge serve` serves `ui/api-workspace.html` (Kimi's design) at
`http://127.0.0.1:8790/`, live-wired: Dashboard = `GET /api/grid` (rows by uid, columns by job),
Quick Run / Workflows start runs (`POST /api/runs`) and drive the gate, Pause, Resume and Stop
(`POST /api/runs/{id}/{approve|pause|resume|cancel}`). Click a paper for its history (`GET /api/uid/{uid}`).
Opened as a plain file, it's still the sample prototype. A copy at `<home>/ui/api-workspace.html` overrides
the built-in one, so you can edit it without rebuilding.

## The workbench page: `ui/workbench.html`

One HTML file that runs papers through API stations, driven entirely by an **API pack** (JSON you can export, edit, and
import into another copy of the page). Open it straight from disk with `forge serve` running.

- **Pages** (Intake, CKG, Theology, YouTube, Academia…) hold up to 20 station cards. `+ Station` and `Duplicate` make the next
  one from the same pattern. Each page can set its own route; each station can override it.
- **Stations** are a prompt template plus a **stage** and the **kinds** of paper they apply to (`paper`, `theology`, `ckg`,
  `youtube`, or `*`). Templates use `{{document}} {{title}} {{kind}} {{related}} {{prior.ID}}`; anything else is flagged.
- **Pipeline**: each paper runs its stages in order. Stations inside a stage run side by side (shared "at once" limit), and
  the next stage waits. A station that reads `{{prior.X}}` is **blocked** if X failed, never run on a missing input.
- **Papers**: drop files or pick a folder. Kind comes from `kind:` front matter (or is guessed); `related:` front matter
  feeds `{{related}}`. Originals are never changed; each is fingerprinted with SHA-256.
- **Thread**: every station run with time, stage, route, model, cost, ms and status. Unchanged work is skipped; repeats
  that reach forge come back `CACHED` at $0.
- **Save output folder** (Edge/Chrome) writes one folder per paper: `00 ORIGINAL.md`, `NN <station>.md` per result,
  `THREAD.md`, `manifest.json` (hashes, models, costs). Other browsers download one bundle file per paper.

## HTTP gateway: `forge serve` (127.0.0.1:8790)

| endpoint | for |
|---|---|
| `POST /v1/chat/completions`, `GET /v1/models` | any OpenAI-style client. `model` = a route name, `auto`, or a model listed in a route |
| `POST /api/generate`, `POST /api/atom-builder/raw` | the Atom Builder's `{prompt, system}` calls; returns `{response, output}` |
| `POST /api/audit` | the Atom Builder's audit records |
| `GET /health`, `GET /search?q=`, `GET /agenda`, `POST /candidate` | the Canon Workbench contract the reconciliation page uses |
| `GET /route/plan`, `/ledger/recent`, `/ledger/stats`, `/context/search` | inspection |

Point the Atom Builder's Worker URL at `http://127.0.0.1:8790` and its calls go through the router; the OpenRouter key
no longer has to sit in the page's localStorage. The reconciliation page talks to port 8765; run `forge serve --port 8765`
only once forge's candidate graph should replace the existing canon engine, otherwise you'll have two sources of truth.

Candidate staging differs from that engine on purpose: unknown top-level fields get a 422 naming the field (instead of a
clean 201 that stored nothing), and re-staging identical content returns the original record (so a retry after a timeout
can't mint a duplicate). Build-state claims (`BUILT`, `ZERO_SORRY`) without a receipt id come back as `NEEDS_REVIEW`.

Browsers are allowed only from `file://` pages and localhost (plus `server.cors_origins`); requests must be JSON, so a
website you visit cannot make the gateway spend your credits.

## MCP: `forge mcp`

```
claude mcp add forge -- C:\Forge\forge.exe mcp
```
or in `claude_desktop_config.json`:
```json
{ "mcpServers": { "forge": { "command": "C:\\Forge\\forge.exe", "args": ["mcp", "--home", "C:\\Forge"] } } }
```

Tools: `forge_brief` (read first: `BRIEF.md`, routes, providers, templates), `forge_ask`, `forge_route_plan`,
`context_search`, `context_add`, `doc_state`, `doc_note`, `pipeline_run`, `template_show`, `canon_stage`, `canon_search`,
`canon_agenda`, `ledger`. Edit `BRIEF.md` to change what every connected AI learns about the project before it starts.

## Layout

```
crates/forge-core   library: config, provider, router, limiter, store, template, pipeline, hints, clips, canon, parse
crates/forge        binary: main.rs (CLI), server.rs (HTTP), mcp.rs (MCP)
crates/semtag-core  note-side tagging: tag block, inline labels, TSNS, fold, CSV, Mermaid (no network)
crates/semtag-cli   offline semtag commands (apply, show-tags, fold, strip, ...)
clipsync-cloudflare-app/   the React clipboard / research front end (Cloudflare Pages)
config/             default providers.json, settings.json (compiled in as fallbacks)
templates/          default job templates (compiled in as fallbacks)
station/            .bat launchers and station layout
docs/               semtag-build-prompt.md (the tagger spec that builds on this core)
```

`cargo test` (51 tests: forge's routing cascade, quotas, budget, truncation, cache, route hints, clip filing and
secret hold-back, limiter, FTS, canon contract, CORS, MCP, INBOX→OUTBOX, lineage; plus semtag-core's suite) · `cargo clippy --all-targets -- -D warnings` · CI builds `forge.exe` on Windows.

## Not built yet

`forge clips pull` from the clipsync backend (waiting on where that backend lives); semtag-cli calling models through
forge-core; streaming responses; provider batch APIs (the 50%-off overnight queues are a separate async flow); embedding search;
the desktop tray layer and `semtag` from `docs/`; admission/rulings (those stay with the Canon Workbench).
