# Windows desktop app: notes (2026-10-13)

David's wishes for the desktop layer (`semtag-desk`, milestones 6-7), written down to build from.
Server-side pieces live in CLIPBOARD-REACT (ClipSync on the Synology); see its `NOTES.md`.

## Three sizes of the same app

| Level | What it is | Content |
|---|---|---|
| 1. Taskbar square | Tray / taskbar icon; click opens a small panel | The clipboard HTML David provides (POF 2828 / nerve `clipboard3.html` style), with AI wired in |
| 2. Paste bar | A thin bar docked to a screen edge | Fast paste: slots, latest clips, prompts one click away |
| 3. Full window | The large view | Everything below |

The UI is David's HTML, not a redesign: the Tauri app loads it and gives it a small JS bridge
(clipboard read/write, hotkeys, paste into the previous window). It talks to the Synology server API.

## Full window holds

- Clipboard history, with **links detected automatically** (URLs get their own tab/marker; YouTube, PDFs, GitHub recognized).
- Prompts and hotstrings (`lean4` -> prompt), fast paste slots (`slot:N`).
- **Research** links.
- **Schedules and workflows** (below).

## Schedules and workflows

- **DeepSeek overview** on a schedule chosen in the app: every 1, 4, 6, 8 or 12 hours. It reads the
  clipboard history since the last run and writes a short overview (themes, open threads, things worth keeping).
- **Ollama organizer**: local model searches the history and organizes it (tags, categories,
  duplicates, "keep" suggestions). It proposes; nothing is deleted without a confirm.
- A workflow is a named list of steps (AI action, script, search, send to station) that can run on a
  schedule or a hotkey. Runs and their results are logged.
- Better placed on the Synology (runs while the PC is off); the desktop app shows and edits them.

## Sync and Cloudflare

- Synology holds everything. The desktop app syncs with it over the LAN, or through the Cloudflare
  Tunnel when away.
- Cloudflare keeps a small set (10 to 100) of **saved clips**: a pinned set reachable even when
  the NAS is off. Size is a setting.
- The app can also talk MCP to the server (`/api/mcp`), so any MCP client sees the same clips.

## AI agents on the computer

- Agents David controls that can do work on the PC: open apps, run registered scripts/.bat files,
  type or paste text.
- Safety (from ROADMAP "extensions"): the `control` capability is off by default, asks before each
  run, shows an "AI is driving" banner, `Esc` stops it, every step is logged, and an agent can only
  run scripts David registered, never invent commands.

## Order (suggested)

1. Taskbar square + full window loading the HTML, connected to the Synology API.
2. Paste bar + hotstrings + paste-into-previous-window.
3. Link detection, Cloudflare saved set.
4. Schedules: DeepSeek overview, Ollama organizer.
5. Workflows, then agents with the `control` safeguards.
