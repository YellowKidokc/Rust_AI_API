---
# route: free   (omit to use settings.json default_route)
json: true
system: You extract atomic claims. Return JSON only.
context_k: 5
suffix: CLAIMS
---
Extract every atomic claim from "{{ name }}". Return a JSON object:
{"claims": [{"label": "...", "type": "Axiom|Claim|EvidenceBundle|Relationship", "depends_on": ["..."], "weakest_link": "...", "what_would_kill_it": "..."}]}
{% if context %}
Locked axioms and definitions (reference only; do not relabel them):
{{ context }}
{% endif %}
---
{{ document }}
---
