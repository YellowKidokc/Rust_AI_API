# ONE_MENU

Portable orchestration for the gathered API pipelines. `ONE_MENU.bat` is the only operational front door; `RELOCATE.bat` is the one-time path repair utility after a move.

## First use

1. Copy `config/paths.example.json` to the ignored `config/paths.json` (or run `RELOCATE.bat`) and enter external locations. Blank entries are reported, never guessed.
2. Set `DEEPSEEK_API_KEY` and/or `OPENAI_API_KEY` in the environment. Never put keys in configuration files.
3. Run `ONE_MENU.bat`, or use it non-interactively:

```bat
ONE_MENU.bat 30 --limit 1 --focus "entropy"
ONE_MENU.bat 47 --item paper.md
ONE_MENU.bat 40 --item PAPER_ID_slug --limit 1
ONE_MENU.bat Y --limit 1
ONE_MENU.bat find resurrection --min 5
```

On macOS/Linux, the equivalent is `python API_HOME/engine/menu.py ...`.

## Inbox, priority, series and groups

Set `inbox` in `config/paths.json`, then drop files in:

```
INBOX\00_PRIORITY\              runs first, whatever else is waiting
INBOX\SERIES\God Is\01 ....md   a series: kept together, in file-number order, gets a series brief (station 48)
INBOX\GROUPS\One Pages\...      a group: kept together, no order
INBOX\...                        loose papers
```

`ONE_MENU.bat 47` makes a paper folder for each (or `47 --item D:\Some\Folder` to pull from a folder in place, with the same
subfolder rules). Originals are never moved; the same file (by SHA-256) never gets a second folder. Paper folders land in
`papers_root\SERIES\<name>\`, `papers_root\GROUPS\<name>\` or `papers_root\`. Point any station at a collection to run all of
its papers: `ONE_MENU.bat 40 --item "<papers_root>\SERIES\God Is"`. With no `--item`, every paper runs, priority first and each
series in order. For papers in a series or group, a station's result is also copied to `<collection>\_BY_STATION\<station>\`
(the latest run per paper), so one station's output for the whole series sits in one folder. `ONE_MENU.bat 48` writes the
series brief to `<collection>\_SERIES\<date>\`: its prompts are `stations\48_SERIES_BRIEF\PAPER_PROMPT.md` and `SERIES_PROMPT.md`.

## Output templates

A station's prompt says what to look for; a template in `templates/OUTPUT/` says what shape the answer comes back in.
`ONE_MENU.bat 49 --item <paper>` fills the Claims Layer v0.3 template (its default); `--template MATH_LAYER_TEMPLATE_v0.3`
or `--template D:\path\my_format.md` swaps the shape for that run. The filled result is `<station>.md`. Paper id, source
SHA-256, model, timestamp and token counts are written by the code, not the model. See `templates/OUTPUT/README.md`.

A third part, the **vocabulary** (`--vocab NAME`, files in `templates/VOCAB/`), is the full list of every type there is:
sent as reference, never returned. The model reports only what is present, tagged with codes from it; unknown codes are
flagged in the receipt. See `templates/VOCAB/README.md`.

## Portability and outputs

All internal paths derive from `engine/paths.py`. Every external root is a named entry in `config/paths.json`; environment variable `ONE_MENU_<KEY>` can override one. Numbered adapters invoke legacy programs only through these roots. New paper runs write dated bundles under `<paper>/02_RUNS/NN_STATION/`, never overwriting an earlier run. Station 46 builds `03_REPORT/report.html` and `report.xlsx`.

Every station has `PROMPT.md`, editable `FOCUS.md`, `station.json`, `README.md`, and a matching `NN_name.py`. Adding a station requires a folder and registry entry, never another batch file.

## Concurrency and recovery

The default call ceiling is 30. `engine/llm.py` owns API access, retry/backoff, the process-wide adaptive limiter, and token receipts. A routine executes stations in dependency order so separate station pools cannot multiply the ceiling. Each station parallelizes independent items/calls and saves an item immediately. Retry keys are defined by source, prompt, model, and focus hashes in receipts.

## Intentionally unresolved owner decisions

ONE_MENU does not silently decide these:

- YouTube canonical download root: `subtitles` or the former channel folder.
- Which 12 real metrics become the statistics headline strip.
- Canonical axiom registry.
- Fruits scale (`-2..+2` is marked provisional in its plug-in rubric) versus `0..4`.
- Which of the 23 OpenAI raw stations receive permanent individual numbers; they remain grouped at 60.
- Whether YouTube transcripts automatically run analytical arms.

Until David answers, configuration and metadata keep these choices explicit and reversible.
