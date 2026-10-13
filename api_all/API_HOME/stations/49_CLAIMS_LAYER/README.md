# 49_CLAIMS_LAYER

`ONE_MENU.bat 49 --item <paper or collection folder>`: one call per paper, the whole paper, filled into
`templates/OUTPUT/CLAIMS_LAYER_TEMPLATE_v0.3.md` (the default set in `station.json`). Change the shape with
`--template <name or path>`; change what it looks for in `PROMPT.md`. Output: `<station>.md` plus json/xlsx/html/receipt.
Accepted options: limit, workers, provider, model, focus, redo, template.
