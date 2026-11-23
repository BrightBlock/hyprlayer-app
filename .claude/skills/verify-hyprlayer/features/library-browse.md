# Library: browse the thoughts tree

The Library (`/library`) is a mail-style reader over every thoughts doc the server indexed. A left nav holds smart boxes (All thoughts, New, Drafts), type boxes, status boxes and a project scope. The middle list groups docs by date with author, project, title, excerpt and chips, and it has text search, a filter popover and sort. Right-clicking a row marks it read or unread. With a mapped git project in scope, the header shows a thoughts sync button that auto-syncs on arrival.

## Sub-features

- `lib-open` opens the Library from the nav rail and lands on the last view.
- `lib-boxes` filters the list by box. Each nav row shows that box's count.
- `lib-scope` scopes the list to one thoughts project, from a nav row or the project picker. You can pin projects there.
- `lib-search` searches titles, bodies and tickets once the query is 3+ characters, ranked by relevance.
- `lib-filter-sort` covers the Type/Status/Scope/Author/Tag filters and the Modified/Date/Title sort.
- `lib-read-state` covers the unread dot, the `Mark as read`/`Mark as unread` row menu, and auto-mark-read on open.
- `lib-blank` is the empty-shelf state: `New thought` or `Set up thoughts`.
- `lib-sync` is the header sync button (status title, pending dot, click to sync, failure card).

## How to get to it (user POV)

- Click the nav rail's Library icon (`aria-label` `Library`). Click it again to go back.
- Use the `nav.library` chord (default `ctrl+2` on mac).
- A doc result in Search, a doc card or graph node, or a project's docs entry opens `/library?doc=…`.

## Driving it with the MCP bridge

Preconditions:

- **Isolated is enough for everything except `lib-sync`.** Every profile reads the one CLI config, `~/Library/Application Support/hyprlayer/config.json`, and the server indexes its thoughts roots. So an isolated instance shows the real `~/thoughts` tree. Read state lives in the profile's own DB, so read/unread toggles there don't touch the user's state. <!-- src: crates/hyprlayer-core/src/env.rs:118 --> <!-- src: crates/hyprlayer-core/src/runtime/mod.rs:780 --> <!-- src: crates/hyprlayer-core/src/runtime/library.rs:225 -->
- "Thoughts are not configured yet" (`config_missing`) in isolated mode comes from per-project calls on the unmapped `verify-demo` seed, not from the Library. `get_thoughts_git_status` raises it for any repo with no mapping, and the sync button hides that code. <!-- src: crates/hyprlayer-core/src/runtime/library.rs:488 --> <!-- src: src/features/library/useThoughtsSync.ts:24 -->
- **`lib-sync` needs shared** (the `.dev` profile, which has mapped git projects). Cost: opening the Library there runs `sync_thoughts` (commit, pull, push of `~/thoughts`) at most once per 60s. The cooldown is module-wide and shared with the status bar's sync. <!-- src: src/features/library/useThoughtsSync.ts:65 --> <!-- src: src/features/library/thoughtsSync.ts:3 -->
- This recipe writes no thoughts files.

Steps:

- **Open.** `webview_interact` `click` on `button[aria-label="Library"]`. The nav shows `New thought`, the `Project scope` button and `nav[aria-label="Library"]`, and the list header reads `All thoughts` with `N across M projects`. <!-- src: src/routes/Root.tsx:1134 --> <!-- src: src/features/library/LibraryNav.tsx:87 --> <!-- src: src/features/library/LibraryNav.tsx:98 --> <!-- src: src/features/library/MessageList.tsx:109 -->
- **Read back counts.** `window.__TAURI__.core.invoke('count_docs', { project: null, filters: {} })`. `total` equals the `All thoughts` row count, and `unread` equals the `New` row count. <!-- src: src/lib/bindings.ts:29 --> <!-- src: src/features/library/libraryBoxes.ts:51 -->
- **Box.** Click the `Plans` nav row (`strategy: "text"`). The URL gains `box=plan`, and the header reads `Plans`. Every row's chip reads `plan`. Read back with `invoke('query_docs', { query: { types: ['plan'], limit: 1 } })`: `total` matches the header count. <!-- src: src/features/library/libraryBoxes.ts:20 --> <!-- src: src/features/library/MessageList.tsx:106 --> <!-- src: src/features/library/MailRow.tsx:95 -->
- **Scope.** Click `button[aria-label="Project scope"]`, then type a name into `input[aria-label="Find a project"]` and click its `[role="option"]`. The scope button shows the project, and `location.search` carries `project=<name>`. <!-- src: src/features/library/ProjectPicker.tsx:111 --> <!-- src: src/features/library/ProjectPicker.tsx:129 -->
- **Search.** `webview_keyboard` `type` into `input[aria-label="Search the library"]`. The URL gains `q=` from the first character, but under 3 characters the list doesn't change. At 3+ it narrows. Read back with `query_docs` and `{ text: '<q>', sort: 'relevance', limit: 5 }`. <!-- src: src/features/library/MessageList.tsx:155 --> <!-- src: src/features/library/DocFilters.tsx:29 -->
- **Filter / sort.** Click `button[aria-label="Filter"]`, pick from the popover (`[aria-label="Filters"]`), and `[aria-label="Filters applied"]` appears. Click `button[aria-label="Sort"]` and pick `Title`: the date group headers go away. <!-- src: src/features/library/MessageList.tsx:120 --> <!-- src: src/features/library/ListFilterPopover.tsx:37 --> <!-- src: src/features/library/MessageList.tsx:128 --> <!-- src: src/features/library/MessageList.tsx:97 -->
- **Read state.** A fresh isolated profile shows no unread rows (`New 0`), so start with `Mark as unread`: right-click a row and click it. The row gains `[aria-label="Unread"]`, and `New` and `count_docs().unread` go up by one. Right-click it again and click `Mark as read`. The dot clears, the count drops back, and `invoke('get_doc', { id })` returns `meta.unread: false`. Take the `id` from `query_docs(...).items[0].id`. <!-- src: src/features/library/MailRow.tsx:75 --> <!-- src: src/features/library/MailRow.tsx:125 -->
- **Sync (shared only).** Read the status from the `aria-label` of `button[aria-label^="Sync thoughts: "]`; the `title` sits on its wrapper span. After the status prefix, it reads `Thoughts synced. Click to sync now`, `N local change(s) to push. Click to sync`, or `Thoughts sync failed: …`. Right after a sync it flashes `Thoughts synced`, and during one it reads `Syncing thoughts…`. Read back with `invoke('get_thoughts_git_status', { projectPath })` (`dirty`, `ahead`, `behind`, `hasRemote`), where `projectPath` is an app project path from `get_bootstrap().projects`, not the thoughts dir. Clicking it is a real push. <!-- src: src/features/library/ThoughtsSyncButton.tsx:24 --> <!-- src: src/features/library/ThoughtsSyncButton.tsx:120 --> <!-- src: src/features/library/ThoughtsSyncButton.tsx:155 --> <!-- src: src/features/library/ThoughtsSyncButton.tsx:35 --> <!-- src: src/lib/bindings.ts:40 -->
- **Proof.** Save screenshots of each box, scope and search state, plus the `count_docs`/`query_docs` JSON.

## Gotchas

- Opening a doc marks it read (once per open, on settled data). In shared mode that changes the user's unread state, so mark it unread again afterwards. The Library reopens the last view, often with `?doc=`, so just arriving can mark a doc read: check its `get_doc` `meta.unread` before and after. <!-- src: src/features/library/DocDetail.tsx:65 -->
- The Library nav is also `nav[aria-label="Library"]`, so target the rail button as `button[aria-label="Library"]`. <!-- src: src/features/library/LibraryNav.tsx:98 -->
- `lib-blank` only renders when thoughts aren't configured or the index holds zero docs. Neither happens on this machine, so report it as unreachable. <!-- src: src/features/library/LibraryView.tsx:76 -->
- `count_docs` needs both keys (`project` and `filters`), and `query_docs` wraps its argument as `{ query }`. <!-- src: src/lib/bindings.ts:28 -->
- The project rows and picker come from the index (`counts.projects`), not the app's project list. An isolated profile still lists every thoughts project, labelled by raw name. <!-- src: src/features/library/LibraryNav.tsx:42 -->
- Raw `mark_doc_read` still refreshes the list and the `New` count: the server's `DocStateChanged` event invalidates them. An open doc pane isn't re-read. Drive the row menu anyway, and use IPC only to read back. <!-- src: src/lib/events.ts:171 -->
