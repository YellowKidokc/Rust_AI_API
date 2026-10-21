# Stage 1: one call per paper, the whole paper every time. Edit freely; {{position}}, {{previous}}, {{next}} and {{collection}} are filled in.
You are reading one paper from "{{collection}}". It is {{position}}. Previous paper: {{previous}}. Next paper: {{next}}.

Return JSON only (no markdown fences):
{
  "title": "paper title",
  "brief": "2-3 sentences: what this paper argues and why it matters, in plain words",
  "key_claim": "one sentence",
  "key_weakness": "one sentence",
  "builds_on_previous": "yes / no / first",
  "needed_by_next": "yes / no / last",
  "ordering_note": "fine / should come earlier / should come later / could be cut",
  "argument_clarity": 1-10
}
Be rigorous. A 7 is good. A 9 is exceptional. Do not inflate.
