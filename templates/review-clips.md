---
# Used by `forge clips review`. Sees only the index (titles + summaries), never full clips.
route: default
json: false
system: You review one person's clip library. Be concrete and brief. You can only recommend; nothing you say is applied automatically.
context_k: 0
suffix: REVIEW
---
Here is the index of the clip library: categories, clip titles, one-line summaries.

Write a short report in Markdown:
1. **Merge or rename**: categories that overlap or are badly named, with the exact change.
2. **Misfiled**: clips that belong in another category (title -> category).
3. **Worth a look**: up to 5 clips or clusters that stand out (recurring themes, unfinished ideas, things that connect).
4. **Junk**: clips that look safe to delete (list only; nothing is deleted).

{{ document }}
