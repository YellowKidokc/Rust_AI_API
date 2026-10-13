# 92_SITE

`ONE_MENU.bat 92` writes `index.html` into the papers folder, every series/group folder and every paper folder.
No model calls; paper folders are only read. `--order 49,40,41` sets which layers stack first on each paper page.
- `index.html`: dashboard (Gated, Unscored, Scored weakest first, Scoring-dependent, Machine-only, Negative), every
  series and group, validation results, and every paper.
- `<paper>\index.html`: the stacked page, each station's latest layer on top, the original document below.
- `_SITE\index.tsv`: MASTER_INDEX v0.3 (43 columns, its source map). `_SITE\validation.tsv`: V1-V11 results (V12 not checked yet).
