---
description: Connect to Simple Editor's live in-app MCP co-editing server
---

Simple Editor hosts its own MCP server (`src/mcp/`) so an agent can edit the *running* project
live — through the app's undo stack — instead of hand-editing the `.sedit` JSON on disk.

1. Confirm the app is running and the MCP server is enabled in Settings (it's off by default).
   If it's not running, tell the user to launch Simple Editor and toggle the co-editing server on
   in Settings before continuing.
2. Connect with:
   ```
   claude mcp add --transport http simple-editor http://127.0.0.1:7337/mcp
   ```
   (Port may differ if the user picked a non-default one in Settings — ask if unsure.)
3. Once connected, use the exposed tools (see `src/mcp/tools.rs` for the current catalogue) to
   read/mutate the live project instead of editing `.sedit` files directly, so changes show up on
   screen immediately and go through undo.
4. Prefer this mode whenever the user has the app open and wants to see edits applied live rather
   than reviewing a diff afterward.
