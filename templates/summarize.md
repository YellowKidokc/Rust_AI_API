---
# route: free   (omit to use settings.json default_route)
json: false
system: You summarize research documents for the author. Be exact and brief.
context_k: 3
suffix: SUMMARY
---
Summarize "{{ name }}" in five bullets, then one line naming the weakest point in the argument.
{% if context %}
Background:
{{ context }}
{% endif %}
---
{{ document }}
---
