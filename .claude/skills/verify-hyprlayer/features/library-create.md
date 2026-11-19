# Library: create a thought

`New thought` opens a dialog with Title, Type, Scope, Project (hidden for global scope), Ticket and Tags. Create writes a templated markdown file with `status: draft` frontmatter into the thoughts tree and opens it in the Library. Plan, research, handoff and pr types start with stock `##` headings; a note starts empty.

## Sub-features

- `create-open` opens the dialog from the nav button, the blank-state button, or the `doc.new` chord.
- `create-global` creates a global-scope thought, needing no project.
- `create-project` creates a user- or shared-scope thought in a mapped git project.
- `create-gate` disables Create until there is a title, and while a non-global scope has no project.
- `create-exists` returns a `DocExists` error for the same date, ticket and title twice.
- `create-cancel` closes the dialog and writes nothing.

## How to get to it (user POV)

- In the Library nav, click `New thought`.
- On an empty shelf, click `New thought` in the blank state.
- Use the `doc.new` chord (default `mod+n`) while the Library is mounted.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** works for `create-global`, `create-gate` and `create-cancel`. The dialog lists only git projects, so the non-git `verify-demo` gives `No projects mapped`. <!-- src: src/features/library/CreateDocDialog.tsx:26 --> <!-- src: src/features/library/CreateDocDialog.tsx:100 -->
- **`create-project` needs shared** (a mapped git project in the app DB). In isolated mode, importing a git repo runs `thoughts init`, which writes the shared CLI config (SKILL.md).
- **Cost and risk:** every create writes a real file under `~/thoughts`, and auto-sync commits and pushes it within seconds. Get the user's OK, name it `zz-verify-scratch`, and delete it through the `thoughts/` symlink when done. The deletion syncs too.

Steps:

- **Open.** Go to the Library (`button[aria-label="Library"]`), then click `New thought` (`strategy: "text"`). A dialog titled `New thought` shows Title, Type (`note`), Scope (`user`), Project, Ticket (placeholder `ENG-1234`) and Tags (placeholder `refactor, ipc, phase-1`). <!-- src: src/features/library/LibraryNav.tsx:79 --> <!-- src: src/features/library/CreateDocDialog.tsx:65 --> <!-- src: src/features/library/CreateDocDialog.tsx:116 --> <!-- src: src/features/library/CreateDocDialog.tsx:123 -->
- **Gate (isolated).** With scope `user`, the Project dropdown reads `No projects mapped`, the warning "Add a project first, or pick the global scope." shows, and `Create` is disabled even with a title. <!-- src: src/features/library/CreateDocDialog.tsx:107 --> <!-- src: src/features/library/CreateDocDialog.tsx:39 -->
- **Title.** The title input has no label or placeholder handle, but it autofocuses. `webview_keyboard` `type` into `[role="dialog"] input` (the first one) with `zz verify scratch`. <!-- src: src/features/library/CreateDocDialog.tsx:69 -->
- **Type and scope.** Click `button[aria-label="Type"]`, then `[role="option"]` text `research`. Click `button[aria-label="Scope"]`, then `global`. The Project field disappears, and `Create` enables. <!-- src: src/features/library/CreateDocDialog.tsx:78 --> <!-- src: src/features/library/CreateDocDialog.tsx:86 --> <!-- src: src/features/library/CreateDocDialog.tsx:93 -->
- **Create.** Click `form button[type="submit"]`. The dialog closes, and the route becomes `/library?doc=<id>`, showing the new doc with Contents `Question`, `Findings`. <!-- src: src/features/library/CreateDocDialog.tsx:133 --> <!-- src: src/features/library/CreateDocDialog.tsx:58 -->
- **Read back.** `ls thoughts/global/shared/research/ | grep zz-verify-scratch` shows `<UTC date>-zz-verify-scratch.md`. Its frontmatter has `type: research`, `status: draft`, `scope: global`, and `author: <thoughts user>`. `invoke('query_docs', { query: { text: 'zz verify scratch', limit: 5 } })` lists it first. The slug `zz-verify-scratch` matches nothing (`total: 0`) even while the file exists. <!-- src: crates/hyprlayer-core/src/runtime/library.rs:426 --> <!-- src: crates/hyprlayer-core/src/docs/templates.rs:59 -->
- **Exists.** Repeat with the same title, type and scope. An `ErrorNote` reads `<absolute path> already exists` and `Pick a different title.`, and no second file is written. <!-- src: crates/hyprlayer-core/src/runtime/library.rs:456 --> <!-- src: src/features/library/CreateDocDialog.tsx:127 -->
- **Cancel.** Open the dialog, type `Discard me`, and click `Cancel`. No `[role="dialog"]` remains, and no `discard-me` file exists. <!-- src: src/features/library/CreateDocDialog.tsx:130 -->
- **Cleanup.** `rm thoughts/global/shared/research/*-zz-verify-scratch.md`, unless library-doc is using the file next. Create makes the type dir when it's missing, so `rmdir` it if it's now empty.
- **Proof.** Save screenshots of the gated dialog, the filled dialog and the opened doc, plus the `ls` output, the file head and the `query_docs` JSON.

## Gotchas

- The filename date is `new Date().toISOString().slice(0, 10)`, which is UTC. A local evening run is dated tomorrow, so glob `*-zz-verify-scratch.md`. <!-- src: src/features/library/CreateDocDialog.tsx:51 -->
- Global scope always writes to `global/shared/<type dir>/`. User scope writes `repos/<mapped>/<user>/…`, and shared writes `repos/<mapped>/shared/…`. <!-- src: crates/hyprlayer-core/src/runtime/library.rs:427 --> <!-- src: crates/hyprlayer-core/src/runtime/library.rs:433 -->
- A ticket is prefixed into the filename (`<date>-<ticket>-<slug>.md`). Leave Ticket empty so cleanup globs stay simple. <!-- src: crates/hyprlayer-core/src/docs/templates.rs:31 -->
- `create_doc` wraps its argument: `invoke('create_doc', { input: { projectRepoPath: null, scope: 'global', docType: 'note', title, date, ticket: null, tags: [] } })`. Use it only to seed, never as the action under test. <!-- src: src/lib/bindings.ts:37 -->
- `doc.new` is registered by the Library view only, so the chord does nothing on other routes. <!-- src: src/features/library/LibraryView.tsx:38 -->
- Enter doesn't submit under the bridge. Click `Create`.
