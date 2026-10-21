# Output templates

A station's prompt says **what to look for**; a template here says **what shape the answer comes back in**.
Swap the template to change the format; the job stays the same.

- Pick one per run: `ONE_MENU.bat 49 --template MATH_LAYER_TEMPLATE_v0.3` (a name from this folder, or a path to any `.md`).
- Or give a station a default in its `station.json`: `"template": "CLAIMS_LAYER_TEMPLATE_v0.3"`.
- The result is saved as `<station>.md` next to the station's json/xlsx/html.

Placeholders filled by the code, not the model (so they are true): `{{paper_id}}`, `{{source_sha256}}`
before the call; `{{provider}}`, `{{exact_model_string}}`, `{{ISO_8601_timestamp}}`,
`{{prompt_completion_total_cost_object}}` after it. Everything else in `{{...}}` is the model's to fill.

The three layer templates here are copies of `api_all/03_EVIDENCE/backside_SCRIPTS/TEMPLATES/` (v0.3).
More of David's templates: that folder (master paper, series aggregation, evidence dashboard) and
`api_all/02_CKG/backside_TEMPLATES/` (CKG and Lean atom templates). Copy any of them here to use it.
