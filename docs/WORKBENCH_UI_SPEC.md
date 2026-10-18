# Workbench UI spec (draft v0.1, 2026-10-15)

David Lowe + Claude. The HTML front end for the API workbench (`pipeline-workflows\API`). It follows
`PAPER_LIFECYCLE_CHECKLIST_v0.1` and imports the stations David already has; it does not replace them.

## 1. Four sections

| Section | Job | Lifecycle checklist |
|---|---|---|
| **Intake** (own page, `intake.html`) | Identity, frozen original, duplicate check, the ten intake questions, routing | §1, §24 |
| **API Calls** | Station groups (CKG, YouTube, Evidence, Academia, Lean, Atoms, Arguments, API Deep); each opens its backside | §2–§12 |
| **Outputs** | One folder per paper: atom record, station results, thread, private audit | §13–§18 |
| **Settings** | Route (DeepSeek default / free), calls at once, budget, key status, paths | §22 |

## 2. Intake

- Drop a paper (or a folder of papers). Python/forge mints the UUID, computes SHA-256, stores the frozen original.
- Duplicate check: exact hash, then near-duplicate.
- Title, series, authors (including the co-writing AI), source format, lane (priority / series / general).
- **Ten intake questions** (ID spec §24): 1–5 build the ID (topic, origin, type, version-of, duplicate);
  6–10 route the paper (claims → CKG; math → Lean; Master Equation → ME station; isomorphism → bridge; declaration vs canon).
- Output: the intake YAML header (ID spec) and a proposed workflow (section 4) the user can accept or edit.

## 3. API Calls: the station backside

Every station card opens four tabs. They read what already sits in each `BACKSIDE\` folder.

| Tab | Source | Editable |
|---|---|---|
| **Code** | `station.json` + the station script: what runs, flags, `uses_api`, kind (legacy / bundle / chain) | No (maintained by the builder AI) |
| **Template** | `BACKSIDE\templates\*.md` (e.g. `CKG_ATOM_RECORD_V3.2_TEMPLATE.md`) | Swap from a dropdown |
| **Ask** | `PROMPT.md` + `FOCUS.md`: what the model is asked to do with the paper | Yes |
| **Runs** | Receipts: date, time, model, tokens, cost, result | No |

The page reads the whole `API` folder through the browser's folder access (Edge/Chrome). Running Python stations
goes through forge, because a page cannot launch `.bat` files.

**Known gap (verified 2026-10-15):** `020_CKG` does not read its backside template. `run_ckg.py` uses
`_system\vendor\ckg\templates\CKG_ATOM_MASTER_TEMPLATE.md`, a different file from
`BACKSIDE\templates\CKG_ATOM_RECORD_V3.2_TEMPLATE.md`, and its `PROMPT.md` only points into the script.
For the Template tab to really swap templates, `station.json` needs a `"template"` field and the runner must honor it.

## 4. Running: the run dialog

Pressing Run on a station, a group, or a selection of papers asks, in order (same order as `button.py`):

1. **Which papers?** Selected papers, a folder, or "all in this group's INBOX". More than 10 without a tick-list asks first.
2. **Put the output in a folder?** Yes/no. If yes: **folder name** (default: the paper's readable ID or title) and location
   (default: the station OUTBOX).
3. **Run in parallel?** How many calls at once (1–30; turbo 60). Default from Settings.
4. **Just this station, or a workflow?**
   - *Just this station*: e.g. run every paper through CKG and stop.
   - *A workflow*: this station plus the next ones (e.g. CKG → Claims → Proofs → API Deep).
5. **Up to 5 extra questions** for this run (answered in a "Your questions" section).
6. **Confirm** with the cost estimate and the first model the route will use. Then a timestamped line per step.

## 5. Workflows

- A **workflow** is a named, ordered list of **stages**; each stage lists one or more stations.
  Stations in a stage run side by side; the next stage waits for all of them. A station that needs an earlier
  result is **blocked** (not run) if that result failed.
- Saved workflows extend the existing `routines.json` (G = full paper audit, P = paper complete, Y = YouTube),
  adding the stage grouping. The `"kind": "chain"` stations (063_PAPER_AUDIT) are workflows already.
- **Batch vs one paper:** run *all papers* through one station (or one workflow), or send *one paper* down its
  own workflow. Intake proposes a workflow per paper from questions 6–10; the user can edit it before running.
- Example: `CKG` → stage 2 `Claims ‖ Proofs ‖ Evidence` → stage 3 `API Deep` → stage 4 `Report`.

## 6. Outputs

One folder per paper (name from the run dialog):
`00 ORIGINAL.md` (frozen, hashed) · the CKG atom record · `NN <station>.md` per result · `THREAD.md`
(every run: station, model, cost, time, result) · `manifest.json` (hashes, template and prompt versions) ·
private audit (checklist §17, never published). Replaced files move to `_older\<date>\`; nothing is deleted.

## 7. Built so far / open

- Built: `ui/workbench.html` (pack-driven stations, stages, barrier, thread, output folder) and forge (routing,
  ledger, cache). Its sample stations are placeholders, not David's stations.
- Open decisions: (a) build `intake.html` first? (b) may the builder change `pipeline-workflows` so stations
  declare their template and the runner honors it?
