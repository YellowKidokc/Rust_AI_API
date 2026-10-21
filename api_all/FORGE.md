# api_all inside forge

`api_all/` is YellowKidokc/API_ALL imported as is (commit b7c2273). Every file is byte-identical to the
source except:

- `03_EVIDENCE/backside_SCRIPTS/pipeline_watcher.exe` was left out (a binary; copy it in locally if needed).
- `API_HOME/engine/llm.py` gained one switch, `FORGE_URL` (below), and `API_HOME/tests/test_forge_gateway.py` tests it.

## Running the ONE_MENU stations through forge

`API_HOME/engine/llm.py` is the only network gateway the ONE_MENU stations use. Set `FORGE_URL` and every
call goes to forge's OpenAI-compatible gateway instead of straight to DeepSeek/OpenAI, so routing, the
DeepSeek→Claude fallback, the response cache, the cost ledger and receipts all live in forge. Unset it and
the stations behave exactly as before.

```bat
forge serve                                   :: terminal 1 (127.0.0.1:8790)
set FORGE_URL=http://127.0.0.1:8790           :: terminal 2
set FORGE_ROUTE=default                       :: optional: default | free | reasoning | local | premium | test
api_all\API_HOME\ONE_MENU.bat 40 --item <paper folder>
```

- Without `FORGE_ROUTE`, the station's model name (`deepseek-chat`) is sent and forge pins that model.
  A model no forge route lists (e.g. `gpt-5-mini`) is refused, so set `FORGE_ROUTE` when using `--provider openai`.
- The Python side needs no provider key when `FORGE_URL` is set; forge holds the keys.
- `FORGE_TOKEN`, if set, is sent as `Authorization: Bearer` (for when forge's gateway auth lands, HANDOFF §5 step 4).
- `FORGE_ROUTE=test` uses forge's mock provider: no network, no cost. Use it to check a station's wiring.

Verified: station 47 (NEW_PAPER) then station 40 (ANALYTICAL_ARMS) against a real `forge serve` on the `test`
route: 5 parallel calls, all 5 in forge's ledger, bundle (json/xlsx/html/run.json) written as before.
Not verified: a real DeepSeek call through this path.

## Not routed through forge yet

The legacy scripts the ONE_MENU adapters launch (`legacy_*` keys in `config/paths.json`) still call providers
directly with their own URLs. They run unchanged; their calls just don't reach forge's ledger. The active ones:

| Script | Direct call |
|---|---|
| `01_YOUTUBE/deepseek-home/{home,index_video,lens_pass,watch_pipeline}.py` | `openai.OpenAI(base_url="https://api.deepseek.com")` |
| `03_EVIDENCE/backside_SCRIPTS/{turbo_pipeline_runner,series_evaluator,series_grand_synthesizer}.py` | provider table with URLs |
| `03_EVIDENCE/backside_SCRIPTS/{three_dials_annotate,argument_builder}.py` | `API_URL` constant |
| `05_API_DEEP_STATIONS/{ATOMS,AXIOM_NODES}/SCRIPTS/llm_client.py` | provider table |
| `07_PIPELINE_WORKFLOWS_API/EvidenceChainIntake/SCRIPTS/{epistemic_intake_v2,semantic_analysis,daily_api_preflight}.py` | `API_URL` constant |
| `02_CKG/backside_CKG/workbench/providers.py` | `api_url` class attributes |
| `06_LEAN/LEAN_ATOM_EXTRACTOR/{axiom_registry.py,lean_atom/exporter/briefs.py}` | URL constant |

Each needs the same small change (read the base URL from `FORGE_URL` when set). It's left until a station is
in daily use, so each edit is tested against the run it serves. `09_SOURCE_API_FOLDERS/` and the `_backup_*`
folders are archive copies and aren't meant to be edited.
