# Live pipeline station workspace

Forge serves API Home at `/` and `ui/api-workspace.html` at `/workspace`. The root `api-workspace_1.html`
is the older seven-group prototype. The pipeline repo's `API/api-workspace.html`
is a separate, larger prototype. Both are preserved.

`GET /api/stations` executes the pipeline's existing `console.py list`, then
attaches `API/_system/config/workspace.json`. That file owns group labels and
the single `EXCLUDED_FROM_PAGE` list. The OpenAI station remains registered,
but is excluded from this page. Unmapped families appear under Front stations.

Each card shows the real station number, metadata, buttons, counts, last run,
goals, prompt SHA-256 hashes and quick-call presets. Refresh reads the files
again, so story prompt changes require no Rust rebuild. Catalog reads cannot
repair or rewrite station paths. The endpoint has a 60-second timeout and
returns HTTP 503 with a setup explanation if the pipeline or Python is missing.

## Install and open on Windows

1. Keep `Rust_AI_API` and `pipeline-workflows` beside one another, or set
   `PIPELINE_WORKFLOWS_ROOT` to the pipeline repository's location.
2. Run `INSTALL_FORGE.bat`. Rust and Python 3 must already be installed.
   The installer builds release Forge into `station/BACKSIDE`, then initializes
   its config, job templates and HTML files without overwriting existing files.
3. Run `station/4 OPEN WORKSPACE.bat`, then open `http://127.0.0.1:8790/`.
4. API Home lists stations and opens their registered launchers. For the detailed
   prompt catalog and Forge jobs, open `/workspace` and select Workbench.

`FORGE_PYTHON` can name a Python executable, including its full path. `PIPELINE_SYSTEM` names `API/_system` for the tray and
API Home; installation also registers the API folder in `config/apis.json`. The page
uses its serving origin by default; an embedding shell can set `window.FORGE_URL`.
Do not run the separate quick-call Picker on port 8790 at the same time as Forge.

Installed HTML overrides survive reinstall. To adopt a newer built-in page while
preserving edits, rename the installed `ui/api-workspace.html` before reinstalling.

## Execution and templates

The workspace catalog is read-only. API Home can launch the registered station
buttons through `/api/launch`, preserving existing ONE_MENU behavior, multi-pass
Stories gates and Python output receipts.
Quick Run and the existing Workflows controls execute **Forge job templates**;
pipeline station IDs are not converted into Forge jobs. `/workbench` also serves
the existing editable, importable API-pack frontend.

The reusable Python station template lives in the pipeline repo at
`docs/templates/FORGE_STATION/`. Register a new station once and refresh this page.
For a plain Forge job, add `templates/<job>.md` using the existing minijinja format.

## Verify

```
cargo test --workspace
cargo fmt --all -- --check
node scripts/test-station-ui.cjs ../pipeline-workflows
```

In pipeline-workflows:

```
python -m unittest discover -s API/_system/tests -p test_station_feed.py
```

The completeness check compares every on-disk station.json with the live feed,
checks visibility, ensures reads preserve the registry, and verifies story prompt
hashes against source bytes. No provider calls are needed.
