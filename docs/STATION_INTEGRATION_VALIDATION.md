# Station integration validation — 2026-10-25

Integrated the existing remote `claude/fis-stations-tray` work rather than
maintaining a second API registry or installer. The pipeline companion branch
integrates `claude/fis-stations` and the previously untracked quick-call presets.

Verified on this Windows checkout:

- Merged Rust workspace suite: 71 tests passed (13 Forge, 39 core, 5 tray, 14 semtag).
- Rust formatting and Clippy with warnings denied passed.
- Python station-feed completeness and story source-hash checks passed.
- Actual-catalog JavaScript render test: 69 registered / 68 visible; Stories,
  Kait Stories, FIS and filtering passed. Home and workspace scripts parsed.
- HTML Assembly accepts a dry run without creating output.

The broad legacy Python suite was not green. Before merging FIS it reported
5 failures and 1 error in 29 tests. Integration fixes address the newly surfaced
Chat/Relocate menu collision and HTML Assembly's missing dry-run option. Remaining
legacy assertions include focus composition, strict front/group folder layouts,
hard-coded path reporting relative to the old `_system` layout, and requiring a
single PROMPT.md for every station despite multi-prompt stations. Those broader
contracts were not rewritten during this integration.

Seven FIS engine tests skip when the external David-OSv3 engine is unreachable.
This does not verify FIS execution against that engine. No paid provider calls
or production station runs were made. A browser was unavailable to the automation
runtime; visual acceptance pending on target machine. The render test and HTTP
checks cover the data contract, not visual layout or a paid end-to-end run.

Local runtime data, private context, taxonomy changes, backups, media, and imported
ZIP trees remain local and are excluded from the integration commits.
