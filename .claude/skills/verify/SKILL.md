---
name: verify
description: Build, launch, and drive the hyprlayer dev app to verify a frontend/backend change at the real surface (Tauri webview via the MCP bridge).
---

# Verify a change in the running app

The release app (`/Applications/Hyprlayer.app`) has no automation bridge and
uses single-instance — never launch a plain `tauri dev` (it collides and
silently exits) and never `pkill Hyprlayer` (matches the user's running app).

1. **Launch a parallel dev instance** (separate identifier + port, safe next to
   the release app):

   ```bash
   pnpm tauri dev --config src-tauri/tauri.dev.conf.json
   ```

   Wait for `MCP Bridge plugin initialized … on 0.0.0.0:9223` in its output.
   `cargo test` beforehand warms the same debug target dir.

2. **Attach** the tauri-mcp-server: `driver_session start` on port **9223**.

3. **Screenshot limit**: `webview_screenshot` times out on windows wider than
   ~1800 logical px. `manage_window resize` to ~1280×820 first, restore after
   (`info` reports physical px; resize takes logical).

4. **Drive** via `webview_interact` (text strategy works for buttons),
   `webview_keyboard type` for inputs. Native `<select>`s need JS:
   set `.value` via the prototype setter, then dispatch a bubbling `change`.

5. **Shared state warning**: the dev instance uses its own SQLite
   (`ai.brightblock.hyprlayer.dev`) but the SAME hyprlayer CLI config
   (`~/Library/Application Support/hyprlayer/config.json`) and thoughts tree
   as the real app. Don't submit config-writing flows (clone/import project,
   create profile) against real state — drive up to the submit and verify
   gating instead.

6. **Cleanup**: kill only the dev process you spawned (its PID /
   `target/debug/Hyprlayer`), stop the driver session, restore window size.
