# 48_SERIES_BRIEF

`ONE_MENU.bat 48 --item "<papers_root>\SERIES\God Is"` (or a group folder; nothing = every series).
Stage 1 sends each paper whole, in parallel, with `PAPER_PROMPT.md`. Stage 2 sends all the stage-1 briefs, in order,
with `SERIES_PROMPT.md`. Output: `<collection>\_SERIES\<date>\48_SERIES_BRIEF.md` (+ `.json`, `.run.json`).
A rerun with the same papers (same source hashes) and prompts is skipped unless `--redo`.
Accepted options: workers, provider, model, focus, redo.
