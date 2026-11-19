# Server status

The status bar's bottom-right button tells the user whether the app is connected to its server and which one. Clicking it while connected opens a popover listing the configured servers. While disconnected, it opens the server dialog instead.

## Sub-features

- `status-connected` shows `connected` with a green dot and the title `Server: <name>`.
- `status-menu` opens the Servers popover, with the connected server marked current.

## How to get to it (user POV)

- Look at the status bar's right end.
- Click the server button there.

## Driving it with the MCP bridge

Preconditions:

- Any launched instance whose doctor passed (no seed needed).

- **Read the label.** Run `webview_find_element` with selector `button[aria-haspopup="dialog"][aria-label^="Server: "]`. Its text contains `connected`, and its `aria-label` is `Server: this machine` for a local profile with no server name.
- **Open the menu.** Run `webview_interact` `click` on that selector. A `ul[aria-label="Servers"]` appears with one `button[aria-current="true"]` row. On a fresh local profile, the row's accessible name is `Connected This machine DEFAULT this machine · v<server version> · 0 worktrees`.
- **Proof.** Save a screenshot of the open popover, plus the `webview_find_element` JSON of the button and the list.

## Gotchas

- In `compact` layouts the label text is hidden and only the dot shows. Check the `aria-label`, not the text.
- A server update in progress reads `updating` with a warn tone. That's not a failure, so wait it out.
- The popover reads live host checks. Remote hosts in a shared profile may show as unreachable while the local row is fine.
