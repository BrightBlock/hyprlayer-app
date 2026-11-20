# Search everything

A spotlight palette for jumping to a workstream, a live agent, a file in the focused worktree, or a thoughts document. Results come in groups (Workstreams, Agents, Files, Documents), at most 8 each. Workstreams and agents fuzzy-match on any input. Documents need 3+ characters and use the thoughts index's relevance search. Choosing a doc opens it in the Library, or as a preview tab when a workstream is focused.

## Sub-features

- `search-open` opens the palette from the dashboard hero search or the `search.open` chord. Escape or a backdrop click closes it.
- `search-empty` shows `Type to search workstreams, documents, and agents` while there are no results.
- `search-workstream` fuzzy-matches title, branch and project path; choosing one opens the workstream.
- `search-agent` lists active sessions with an activity dot; choosing one opens the workstream and focuses that agent tab.
- `search-doc` shows up to 8 docs once the query is 3+ characters; choosing one opens `/library?doc=…`, or a doc preview tab in the focused workstream.
- `search-file` lists worktree files for the focused workstream only.
- `search-keys` covers ArrowUp/ArrowDown wrap-around and Enter on the active row.

## How to get to it (user POV)

- On the dashboard, click the hero search ("Search N documents, workstreams, agents…").
- Use the `search.open` chord (default `mod+p`) anywhere except onboarding.

## Driving it with the MCP bridge

Preconditions:

- **Isolated**, seeded with `verify-demo` plus one workstream (`Verify map seed` from workstream-create) for `search-workstream`. Docs come from the real `~/thoughts` index in either mode (see library-browse).
- `search-agent` needs an active session, which costs a live agent. Skip it unless the run already has one.
- Read-only: nothing is written.

Steps:

- **Open.** On `/`, `webview_interact` `click` with selector `workstreams, agents…` and `strategy: "text"`. `[role="combobox"][aria-label="Search everything"]` appears, holding `input[placeholder="Search workstreams, documents, agents…"]`, and the listbox shows the empty hint or recent workstreams. <!-- src: src/features/dashboard/Observatory.tsx:102 --> <!-- src: src/features/dashboard/Observatory.tsx:110 --> <!-- src: src/features/search/SearchPalette.tsx:213 --> <!-- src: src/features/search/SearchPalette.tsx:233 -->
- **Workstream.** `webview_keyboard` `type` `seed` into the input. A `Workstreams` group lists `Verify map seed`, tagged with its stage. Click its `[role="option"]`. The palette closes, and the route is `/projects/<path>?item=<id>`. Read back with `invoke('list_workstreams', { projectPath, includeArchived: false })`: the id matches. <!-- src: src/features/search/search.ts:27 --> <!-- src: src/features/search/SearchPalette.tsx:87 --> <!-- src: src/features/search/SearchPalette.tsx:30 -->
- **Doc (from the dashboard).** Go back to `/` and reopen. Type 2 characters: there's no `Documents` group, even right after an earlier 3+ character query. Type a third: after the 120ms debounce, `Documents` lists up to 8 rows (title, `project · type`). The palette fetches 12 and each group keeps 8. <!-- src: src/features/search/search.ts:33 --> Compare with `invoke('query_docs', { query: { text: '<q>', sort: 'relevance', limit: 12 } })`, whose first 8 match the palette's order. Click the first row. The route becomes `/library?doc=<id>`, and the doc renders. <!-- src: src/features/search/useSearchSources.ts:52 --> <!-- src: src/features/search/useSearchSources.ts:54 --> <!-- src: src/features/search/SearchPalette.tsx:48 -->
- **Doc (in a workstream).** With `?item=` on a project route, choosing a doc opens a preview tab in that workstream instead. The URL stays on the project route. <!-- src: src/features/search/SearchPalette.tsx:45 -->
- **Keys.** With 2+ results, `webview_keyboard` `press` `ArrowDown`. The next `[role="option"]` gets `aria-selected="true"`, and ArrowUp from the first wraps to the last. <!-- src: src/features/search/SearchPalette.tsx:186 --> <!-- src: src/features/search/SearchPalette.tsx:88 -->
- **Close.** Press Escape in the input. No `[aria-label="Search everything"]` remains. <!-- src: src/features/search/SearchPalette.tsx:228 -->
- **Proof.** Save screenshots of the empty palette, each result group and the landing route, plus the `query_docs`/`list_workstreams` JSON.

## Gotchas

- Choosing a `Files` result only closes the palette. The `file` nav case is a no-op, so don't report it as opening the file. <!-- src: src/features/search/SearchPalette.tsx:51 -->
- `Files` only appears with a focused workstream (`/projects/…?item=`) and a non-empty query. <!-- src: src/features/search/useSearchSources.ts:102 -->
- The hero search has no aria-label; match it by text. The chord path is a `keydown` on `window` resolved by `event.key`, so `window.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', metaKey: true }))` reaches the real resolver. A shared profile may have rebound it: read `settings.keybindings.overrides` from `get_app_settings` first. <!-- src: src/lib/keybindings/useKeybindings.ts:47 --> <!-- src: src/lib/keybindings/chord.ts:98 --> <!-- src: src/lib/keybindings/commands.ts:33 -->
- Enter is a `keydown` handler on the combobox, not a form submit. Whether the bridge's Enter reaches it isn't grounded, so click the option.
- On onboarding, the chord is swallowed. <!-- src: src/routes/Root.tsx:775 -->
- The doc group follows the debounced query, while the workstream and agent groups follow the live one. Wait ~200ms after typing before reading Documents. <!-- src: src/features/search/useSearchSources.ts:29 -->
