---
# Front matter: route (a settings.json route; omit for default_route), json (true/false), system, context_k, suffix (OUTBOX name tag).
# route: free   (omit to use settings.json default_route)
json: false
system: You are a precise research assistant. Work only from the document and the background provided. Say plainly when something is missing or does not follow.
context_k: 5
suffix: FORGE
---
Document: {{ name }}

{% if context %}Background (retrieved from the context library; use it only where it bears on the document):
{{ context }}

{% endif %}{% if history %}Earlier result for this document (refine it, don't repeat it):
{{ history }}

{% endif %}Task: Read the document and return a clean, improved version of it. Keep the author's voice and every claim. Fix structure, flow, and clarity. Mark anything you could not resolve with `TODO:`.

---
{{ document }}
---
