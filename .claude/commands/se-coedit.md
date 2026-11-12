---
description: Connect to Simple Editor's live in-app MCP co-editing server
---

Simple Editor hosts its own MCP server (`src/mcp/`) so an agent can edit the *running* project
live — through the app's undo stack — instead of hand-editing the `.sedit` JSON on disk.

1. Confirm the app is running and the MCP server is enabled in Settings (it's off by default).
   If it's not running, tell the user to launch Simple Editor and toggle the co-editing server on
   in Settings before continuing.
2. Connect with the command from **Settings ▸ General ▸ "Copy Claude Code command"** — ask the
   user to paste it. Don't assume a port: when the configured one (default 7337) is taken the app
   binds the next free one (up to +20), and only that button and the status line next to it
   ("running at http://127.0.0.1:<port>/mcp") show the port actually in use. The command has the
   shape `claude mcp add --transport http simple-editor http://127.0.0.1:<port>/mcp`.
3. Once connected, use the exposed tools (`tools/list`; the server's `instructions` say how) to
   read/mutate the live project instead of editing `.sedit` files directly, so changes show up on
   screen immediately and go through undo. Start each request with `editor.context`, wrap its edits
   in `edit.begin{label}` … `edit.end` (or one `edit.batch`) so the user gets one undo step, check
   the result with `render.frame` / `render.contact_sheet`, and loop on `assistant.wait` to take
   requests and comments from the user live.
4. Prefer this mode whenever the user has the app open and wants to see edits applied live rather
   than reviewing a diff afterward.
