# Reference vocabularies

The third part of a station, next to the job (`PROMPT.md`) and the output shape (`templates/OUTPUT/`):
the full list of every type there is. It is sent with every call as reference and never comes back.
The model reports only what is actually in the document, tagging each item with one code from here.

- Write each code in backticks: `` `cl-causal` ``. Tables, lists, prose around it: anything goes.
- Per run: `ONE_MENU.bat 49 --vocab TAG_TAXONOMY_v0.3` (a name from this folder, or a path). Repeat `--vocab` to stack several.
- As a station default, in its `station.json`: `"vocab": "TAG_TAXONOMY_v0.3"` or a list.
- Closed vocabulary: a backticked code in the reply that is in no vocabulary (and not in the template) is recorded in
  the receipt as `codes_unknown`. Each call's `codes_used` counts which codes were used.
- `UNCLASSIFIED` is always allowed: the model uses it, with a reason, for something that fits no code.
- The vocabulary goes first in the prompt, so every paper's prompt opens identically; providers with prompt caching
  (DeepSeek's is automatic) bill that repeated opening at the cache-hit rate.

`TAG_TAXONOMY_v0.3.md` here is a copy of `api_all/03_EVIDENCE/backside_SCRIPTS/TEMPLATES/TAG_TAXONOMY_v0.3.md`.
