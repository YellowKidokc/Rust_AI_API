# Handoff: David's AI workbench (state as of 2026-10-13)

Read this first, then `BRIEF.md` (how David works and the framework vocabulary), then `README.md` (how forge runs).
David works with several AIs at once. The biggest risk to this project is two sessions building the same piece in
two places. Section 2 says who owns what. Keep it true.

## 1. What David wants (his words, condensed)

An AI **front end** for his Windows computer (tray app, hotkeys, act on selected text, paste bar) and an AI **back
end** for everything scripted or API-driven, with local models as an option. Clipboard history lives on his Synology
NAS, reachable from anywhere through Cloudflare. AIs organize it in the background, and he looks when he wants to.
DeepSeek is the everyday model; free tiers are an option, not the default.

**Ground rule he set:** minimal builds that work, used daily for a week or two, before new layers. Don't add
architecture (event buses, WASM plugins, HUDs, graph views) until real use asks for it.

## 2. Who owns what

| Piece | Repo / branch | Owns | Must not |
|---|---|---|---|
| **forge** (Rust) | `Rust_AI_API`, branch `claude/confident-thompson-qmwyrv`, PR #2 | **Every model call**: routing, keys, retries, cost ledger, cache, context library, templates, INBOX→OUTBOX pipeline, the scheduled AI jobs, the MCP server for AI work | Store clipboard history |
| **semtag** (Rust) | same workspace: `crates/semtag-core`, `crates/semtag-cli` | Note-side tag format (byte-compatible with the Obsidian plugin), TSNS, fold, CSV, Mermaid | Call models directly (it should go through forge-core) |
| **ClipSync server** (Node/Hono + SQLite) | `CLIPBOARD-REACT`, branch `claude/synology-home` | Clip/bookmark/prompt **data** on the Synology (Docker), the PWA UI, Cloudflare Tunnel + Access, the capture script, a data-only MCP (`/api/mcp`: search/get/create clips) | Grow its own AI scheduler/organizer (that's forge's `clips` job) |
| **Desktop app** (Tauri, not built) | plan in `docs/DESKTOP_APP_NOTES.md` (+ semtag spec Part B in `docs/semtag-build-prompt.md`) | Tray square, paste bar, full window, hotkeys, selection actions; loads David's own HTML | Hold API keys; it calls forge and ClipSync |
| **api_all** (Python stations) | `api_all/` in this workspace, imported verbatim from `API_ALL` (see `api_all/FORGE.md`) | The ONE_MENU stations, their prompts, rubrics, specs and the legacy pipeline scripts | Talk to providers directly once `FORGE_URL` is set: every station that calls through ONE_MENU's `engine/llm.py` goes through forge's gateway. The legacy scripts in `api_all/FORGE.md`'s table still call providers directly until they are migrated |
| `clipsync-cloudflare-app/` | in this workspace (from the semtag branch) | An older/alternate React front end that expects an API at `:8787/api` | (**open question**: is it superseded by CLIPBOARD-REACT? see §6) |

There are about 13 other `*clip*` repos on David's account (ClipSync-CF, Synology-Clipboard, clipboard-sync-platform, …).
Treat them as history unless David says otherwise.

**The seam:** ClipSync keeps the clips; forge does the thinking. ClipSync's own `shared/ai-provider.ts` already
speaks the OpenAI chat API, so pointing its base URL at forge's gateway (`/v1`, model = a route name like `default`)
puts its chat/research calls through forge's routing and ledger without new code. That needs §5 step 4 first (gateway auth).

## 3. State of forge (this repo)

Built and tested (`cargo test`: 51 pass incl. semtag's 14; clippy `-D warnings` and fmt clean; CI green on Linux
and Windows, CI uploads `forge.exe` as artifact `forge-windows-x64`):

- **Router**: ordered ladders per route in `config/settings.json` (`default` = deepseek-chat → claude-sonnet-5-5;
  `reasoning`, `free`, `local`, `premium`, `test`). Rungs skip on missing key / 429 cooldown / daily quota / time
  window / daily budget / context window. Truncated replies and refusals cascade. Adaptive concurrency limiter.
- **Ledger** `_data/forge.sqlite`: every call with tokens, cost and latency; response cache (identical request → $0,
  `CACHED`); document state; lineage (source hash → output); FTS5 context library; candidate graph. JSONL receipts
  in `LOGS/` (no prompts).
- **Front doors**: CLI (`run ask plan context ledger clips templates doctor init`), `serve` (127.0.0.1:8790:
  OpenAI-compatible `/v1/chat/completions`, `/api/generate`, the Canon Workbench `/candidate /search /agenda`),
  `mcp` (stdio; tools listed in README).
- **Note route hints**: front matter `route:` or words like `draft`/`deep`/`private` pick the route.
- **Clipboard librarian** `forge clips sort|review|index`: currently **folder-based** (one file per clip). Secrets
  (keys, passwords, private keys, card numbers) are held back and never sent; review writes a report, moves nothing.
  `station/CLIPS SETUP.bat` schedules it on Windows.

Not built: streaming, provider batch APIs, embedding search, gateway auth / LAN bind, the desktop app,
semtag-cli calling models through forge-core.

Not verified: any real provider call (no keys were available where it was built). First real run is David's.

## 4. Decisions already made (don't relitigate without a reason)

- **One router.** No second `llm-router` crate; extend `forge-core`.
- **rusqlite, not sqlx.** Single static exe. sqlx doesn't reach Cloudflare D1 anyway (D1 is a Worker binding / HTTP API).
- **SQLite FTS5 before vectors.** Hundreds of axioms don't need LanceDB. Add embeddings behind `context_search` only if recall fails in use.
- **Routing words live in front matter**, not in semtag tags (semtag tags are content types: Axiom, Claim, …).
- **Port 8790**, not 8765: forge must not silently become a second canon engine next to the existing one.
- **Candidate staging**: unknown top-level fields → 422 (never a silent 201); identical content dedupes.
- **serde_json `preserve_order` is on** (semtag needs it); forge hashes through `canon::canonical_json`.
- **Keys only in env vars.** Never in JSON, never in localStorage, never in prompts or logs.
- **Nothing is deleted.** Replaced outputs go to `_older/<date>/`; ledger archive only flags rows.

## 5. Next steps, in order (each small, each usable)

1. **David runs forge on Windows**: download the CI artifact, `forge init C:\Forge`, `setx DEEPSEEK_API_KEY …`,
   `forge doctor`, `forge ask "…"`, then a real `forge run` on 5 notes. Fix whatever that surfaces first.
2. **ClipSync live on the Synology** (CLIPBOARD-REACT `claude/synology-home` README): Docker, API key, Tunnel,
   **Cloudflare Access on**, import the old D1 history.
3. **`forge clips sort --from-clipsync <URL>`**: replaces the folder source and the server-side "organizer" in
   ClipSync's NOTES.md:
   - `GET /api/clips?limit=N` with `X-API-Key` (env `CLIPSYNC_API_KEY`); candidates are clips whose `category` is
     still the default `clipboard` and that aren't in forge's ledger at that `updatedAt`.
   - Long clips come back as previews (`truncated: true`); fetch `GET /api/clips/:id` for the full text.
   - Same secret hold-back (`clips::secret_reason`): held clips get tag `held` and are never sent.
   - Write back with `PATCH /api/clips/:id` `{category, tags}` (keep the user's existing tags, add the model's).
   - `review` reads titles/categories/tags from the API and writes the report as a ClipSync **note** (or to a
     folder), never moving or deleting anything.
   - Test against a fake server (axum in the test) that serves fixture clips and records the PATCHes.
4. **Gateway auth + LAN bind** for forge: `FORGE_TOKEN` env; when set, require `Authorization: Bearer` on
   everything except `/health`; allow `--host 0.0.0.0` only when a token is set. Then point ClipSync's AI base URL at forge.
5. **Run forge on the Synology** too (Linux build or Docker) so the scheduled clip jobs run while the PC is off.
6. **Desktop app**, per `docs/DESKTOP_APP_NOTES.md` order: taskbar square loading David's HTML first.

## 6. Open questions for David

- Is `clipsync-cloudflare-app/` (in this repo) still wanted, or is CLIPBOARD-REACT's UI the one? If superseded,
  remove it (and its committed `dist/`) in a separate PR.
- ~~Which Synology model?~~ **DS923+, 32 GB RAM** (AMD Ryzen R1600, x86_64): forge builds as a plain
  `x86_64-unknown-linux-musl` binary or a Docker image; no ARM cross-compile needed.
- How many clips should the hourly sort take per run on the free tier (quotas are per day)?

## 7. Working with David

- Read `BRIEF.md`. Short version: challenge beats agreement, no performance, say it when he's wrong, name
  recurring blind spots, abort dead threads plainly, verify before dismissing.
- He talks fast, voice to text, several threads at once; track the thread under the tangent.
- When another AI's spec arrives pasted in, check it against the actual code before building it. Specs from
  other sessions have named modules that didn't exist and stale model names.
- Small validated pushes; tests with every change; say plainly what was and wasn't verified.
