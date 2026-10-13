---
# Used by `forge clips sort`. Runs once per clip; keep it cheap.
route: free
json: true
system: You are a librarian for one person's clipboard history. File each clip so it can be found again. Reply with JSON only.
context_k: 0
suffix: SORTED
---
File this clipboard clip.

Existing categories (reuse one when it fits; create a new one only when nothing fits): {{ categories if categories else "(none yet)" }}

Reply with exactly this JSON:
{"category": "short folder name, Title Case, 1-3 words", "title": "specific descriptive title, max 10 words", "kind": "link|code|quote|idea|task|reference|research|writing|data|personal|junk", "tags": ["up to 6 lowercase tags"], "summary": "one sentence: what this is and why it was probably copied"}
{% if truncated %}
(The clip is longer than shown; classify from this excerpt.)
{% endif %}
--- CLIP ({{ name }}) ---
{{ document }}
--- END CLIP ---
