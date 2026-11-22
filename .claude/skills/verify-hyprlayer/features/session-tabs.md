# Session tabs

A workstream's tab strip holds its agent tabs, starting with one `Agent` tab. `New agent` adds another chat tab, `Agent N`, or opens an engine menu when the multiplexer is enabled. Each tab keeps its own draft, mode, model and session, and shows a mode icon and a state dot. Right-clicking a tab offers `Rename tab`, `Copy transcript`, split options, `Archive agent` and `Close agent`/`Close tab`. Archiving a tab whose agent is still working asks first (`Stop agent?`). The last tab can't be archived or closed.

## Sub-features

- `tab-new` adds a native chat tab from `New agent`.
- `tab-new-menu` shows the `New agent in <workstream>` engine menu when the multiplexer is on.
- `tab-select` switches tabs, keeping each tab's draft.
- `tab-rename` renames a tab and pushes the title to its session.
- `tab-archive` archives a tab, and marks its session archived.
- `tab-archive-working` shows the `Stop agent?` confirm for a busy agent.
- `tab-last` keeps at least one agent in the strip.

## How to get to it (user POV)

- Click `+` (`New agent`) at the right end of the tab strip. <!-- src: src/routes/ProjectPage.tsx:1195 -->
- Click a tab to select it, or right-click it for its menu. <!-- src: src/routes/ProjectPage.tsx:1107 -->

## Driving it with the MCP bridge

Preconditions:

- **Isolated** instance with the `Verify map seed` workstream ([workstream-create.md](./workstream-create.md)). The multiplexer is off by default, so `New agent` adds a chat tab directly. <!-- src: src/features/multiplexer/engines.ts:59 --> <!-- src: src/routes/ProjectPage.tsx:705 -->
- New, select and rename on unstarted tabs are **free**. Rename-push and archive read-back need a started session: use the free path in [session-send.md](./session-send.md) ($0 with tab titles off).

- **New.** Click `button[aria-label="New agent"]`. A second `[role="tab"]` reading `Agent 2` appears and is `aria-selected="true"`, with an empty composer. <!-- src: src/routes/ProjectPage.tsx:655 --> <!-- src: src/routes/ProjectPage.tsx:1100 -->
- **Drafts are per tab.** Type `draft-two` into `[data-pane-content] textarea`. Click `Agent` (`strategy: "text"`) and the composer is empty. Click `Agent 2` and `draft-two` is back. Read back each shell's `draft` through the live `workstreamTabs` module (SKILL.md, Drive step 5): `useWorkstreamTabsStore.getState().byWs[<wsId>].shells`. <!-- src: src/routes/ProjectPage.tsx:968 -->
- **Rename.** Right-click `Agent 2` (`strategy: "text"`), then click `Rename tab`. In the `Rename tab` dialog, type `Renamed` into `[role="dialog"] input` and click `Rename`. The tab reads `Renamed`, and its shell has `titleLocked: true`. If the tab has a session, `list_sessions` (`{ workstreamId }`) then shows `title: "Renamed"` for it. <!-- src: src/routes/ProjectPage.tsx:1464 --> <!-- src: src/routes/ProjectPage.tsx:1685 --> <!-- src: src/routes/ProjectPage.tsx:1703 --> <!-- src: src/routes/ProjectPage.tsx:1025 --> <!-- src: src/features/session/sessionTitleSync.ts:67 -->
- **Archive.** With a finished session in `Renamed`, right-click it and click `Archive agent`. The tab leaves the strip. `list_sessions` shows a non-null `archivedAt` for its session, and the tab store's `archived[0].shell.title` is `Renamed`. <!-- src: src/routes/ProjectPage.tsx:1538 --> <!-- src: src/routes/ProjectPage.tsx:838 -->
- **Archive while working.** Start the long turn from session-send.md in a tab, then archive it. `Stop agent?` opens, reading "… is still working. Archiving it stops the current turn." `Cancel` keeps the tab. `Stop and archive` archives it and ends the session. <!-- src: src/features/session/StopAgentDialog.tsx:16 --> <!-- src: src/features/session/StopAgentDialog.tsx:17 --> <!-- src: src/routes/ProjectPage.tsx:847 -->
- **Last tab.** With one tab left, its menu's `Archive agent` is disabled with the hint `Keep at least one agent in the strip.`, `Close agent` and both split items are disabled too, and the tab shows no close `×`. <!-- src: src/routes/ProjectPage.tsx:1475 --> <!-- src: src/routes/ProjectPage.tsx:1543 --> <!-- src: src/routes/ProjectPage.tsx:589 --> <!-- src: src/routes/ProjectPage.tsx:1215 -->
- **Proof.** Screenshot the strip after each step and the `Stop agent?` dialog. Save the tab-store JSON and `list_sessions` JSON.

## Gotchas

- An inactive tab's close `×` (`aria-label` `Archive <title>` or `Close <title>`) is `invisible` until hover. Select the tab first, or use the context menu. <!-- src: src/routes/ProjectPage.tsx:234 --> <!-- src: src/routes/ProjectPage.tsx:245 -->
- A generated title types itself in at 28ms per character, so text matches on the tab fail mid-animation. Wait, or read the tab store. With tab titles off (the free seed), tabs keep `Agent N`. <!-- src: src/features/session/TabTitle.tsx:6 -->
- Tab state lives in webview `localStorage` per Vite origin (`hyprlayer:workstream-tabs`), not in the server. An isolated run on a reused port can restore old tabs. <!-- src: src/features/session/workstreamTabs.ts:373 -->
- `SessionChat` remounts on every tab switch, so view-only state (scroll position, an open tool row) resets.
- With the multiplexer on, `New agent` opens a menu of engines plus `Native chat`, and a new tab may be a PTY agent instead of a chat. That's a different recipe. <!-- src: src/routes/ProjectPage.tsx:1662 -->
- Closing a tab (`Close agent`) drops it without archiving. Use `Archive agent` when the session should stay listed. <!-- src: src/routes/ProjectPage.tsx:1549 -->
