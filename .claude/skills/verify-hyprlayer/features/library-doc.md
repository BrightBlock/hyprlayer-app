# Library: open, edit and save a doc

Selecting a doc opens it in the Library's detail pane. A toolbar shows its relative path, a Narrow/Wide toggle and Reveal in Finder. The body renders as markdown, and clicking it switches to a CodeMirror editor. A right rail holds Edit/Save with a save status, a Contents list built from `##`–`####` headings (click to jump, scrollspy highlight), and a Properties form for the frontmatter (title, type, date, status, ticket, project, scope, tags, author, related, raw YAML). Save writes body and frontmatter in one call. Leaving with unsaved changes asks to Save or Discard. A file changed on disk while you edit fails the save with a `Reload from disk` button.

## Sub-features

- `doc-open` renders the doc for `?doc=<id>`, with its path in the toolbar.
- `doc-rail` toggles the right rail (`Hide panels`/`Show panels`) and collapses its Contents/Properties sections.
- `doc-contents` lists the headings and jumps to one; an empty doc shows `Add a ## heading to build contents`.
- `doc-edit` enters the editor by clicking the body or `Edit`, and leaves it with `Done`, Escape, or a click elsewhere.
- `doc-props` edits a frontmatter field, showing `Unsaved` and, for status, `was X · unsaved`.
- `doc-save` saves: `Saved` flashes and the file changes on disk.
- `doc-unsaved-guard` opens the `Unsaved changes` dialog when you leave a dirty doc.
- `doc-conflict` turns an external edit during a dirty session into `doc_conflict` plus `Reload from disk`.

## How to get to it (user POV)

- In the Library list, click a row.
- Choose a doc result in Search (off a project route), a doc card, a related link, or a graph node.

## Driving it with the MCP bridge

Preconditions:

- **Isolated** works. It reads the real `~/thoughts` (see library-browse). Every save writes a real thoughts file, which auto-sync pushes (SKILL.md Evidence).
- **Read-only steps** (`doc-open`, `doc-rail`, `doc-contents`, entering and leaving the editor without typing) are safe on any doc.
- **Write steps** use only a scratch doc. Create `zz-verify-scratch` (type `research`, scope `global`) via library-create first, after the user OKs a scratch file reaching the thoughts remote. It lands at `thoughts/global/shared/research/<UTC date>-zz-verify-scratch.md`, with `## Question` and `## Findings` headings. <!-- src: crates/hyprlayer-core/src/docs/templates.rs:44 --> <!-- src: crates/hyprlayer-core/src/runtime/library.rs:433 -->
- Copy the file into the evidence dir before the first edit, and delete it at the end through the repo's `thoughts/` symlink. To check that a save keeps the file mode, `chmod 755` it first.

Steps:

- **Open.** In the list, click the row with text `zz-verify-scratch`, or reach it through `location` with `/library?doc=<encoded id>`. The toolbar shows `meta.relPath`, plus `[aria-label="Document width"]` and `button[aria-label="Reveal in Finder"]`. <!-- src: src/features/library/DocDetail.tsx:257 --> <!-- src: src/features/library/DocDetail.tsx:261 --> <!-- src: src/features/library/DocDetail.tsx:273 -->
- **Read back.** Take the id from `new URL(location.href).searchParams.get('doc')`, then call `invoke('get_doc', { id })`. You get `meta.relPath`, `body`, `frontmatter`, `fmKeys` and `mtimeMs`. The id is the canonical `/Users/jt/thoughts/…` path. <!-- src: src/lib/bindings.ts:32 -->
- **Contents.** The rail's `Contents` section header shows `2`, with buttons `Question` and `Findings`. Click `Findings`, and its button gets the accent bar and `font-semibold`. <!-- src: src/features/library/DocRail.tsx:160 --> <!-- src: src/features/library/DocRail.tsx:86 -->
- **Rail toggle.** Click `button[aria-label="Hide panels"]`. The rail goes, and the toolbar grows its own `Edit` and `Save`. Click `Show panels` to restore it. <!-- src: src/routes/Root.tsx:1052 --> <!-- src: src/features/library/DocDetail.tsx:278 -->
- **Edit a property.** In the rail, `webview_keyboard` `type` the full new value into `#fm-title` (`zz verify scratch edited`; `type` replaces, it doesn't append). `SaveStatus` reads `Unsaved`, and the rail's `Save` enables. <!-- src: src/features/library/DocProperties.tsx:37 --> <!-- src: src/components/ui.tsx:888 --> <!-- src: src/features/library/DocRail.tsx:156 -->
- **Save.** Click the rail's `Save` (`strategy: "text"`; see Gotchas). `Saving…` shows, then `Saved` for about 1.8s. `get_doc` returns `meta.fm.title` ending in ` edited`, and `grep '^title:' <file>` shows it on disk. `stat -f %Sp <file>` still reads `-rwxr-xr-x`. <!-- src: src/components/ui.tsx:896 --> <!-- src: src/features/library/useDocSave.ts:9 -->
- **Body edit.** Click the rendered body. CodeMirror (`.cm-editor`) mounts, and the rail button reads `Done`. Typing into CodeMirror through the bridge isn't grounded (contenteditable, not a controlled input), so prove `doc-save` with the property path and only screenshot entering and leaving the editor. <!-- src: src/features/library/DocDetail.tsx:345 --> <!-- src: src/features/library/DocRail.tsx:154 -->
- **Unsaved guard.** Make the doc dirty again, then click another list row. A dialog titled `Unsaved changes` reads "This document has unsaved changes." with `Cancel`, `Discard` and `Save`. `Discard` navigates and leaves the file as it was. <!-- src: src/features/library/DocDetail.tsx:402 --> <!-- src: src/features/library/DocDetail.tsx:409 --> <!-- src: src/features/library/DocDetail.tsx:415 -->
- **Conflict.** Dirty `#fm-title`, then append a line to the file from the shell: `printf '\nexternal\n' >> thoughts/global/shared/research/<file>`. Click `Save`. An error note reads "The document changed on disk since you opened it." with `Reload from disk`. Clicking that adopts the disk content, and the `external` line renders. <!-- src: crates/hyprlayer-core/src/runtime/library.rs:53 --> <!-- src: src/features/library/DocDetail.tsx:297 --> <!-- src: src/features/library/useDocSave.ts:117 -->
- **Cleanup.** `rm thoughts/global/shared/research/<UTC date>-zz-verify-scratch.md`. `rmdir` the research dir if create made it and it's now empty. Then `query_docs` with `{ text: 'zz verify scratch', limit: 10 }` should list no item whose `relPath` contains `zz-verify-scratch`. Don't query the slug: it returns `total: 0` even while the file exists.
- **Proof.** Save screenshots of open, dirty, saved, the guard dialog and the conflict, the `get_doc` JSON before and after, the `grep` output, and the final `query_docs`.

## Gotchas

- With the rail open (the default), Edit and Save exist only in the rail header. With it closed, they exist only in the toolbar. There is never more than one `Save` button. <!-- src: src/features/library/DocDetail.tsx:278 --> <!-- src: src/lib/uiStore.ts:110 -->
- Clicking `Edit`/`Done` relies on `onMouseDown` preventDefault to stop the editor's blur-exit from racing the toggle. Bridge clicks send no pointer events, so prefer Escape to leave the editor. <!-- src: src/features/library/DocRail.tsx:153 --> <!-- src: src/features/library/DocDetail.tsx:315 -->
- Leaving the window blurs the editor without exiting, because exit-on-blur checks `document.hasFocus()`. An unfocused dev window can keep the editor up. <!-- src: src/features/library/DocDetail.tsx:312 -->
- A save that composes to identical bytes writes nothing (mtime unchanged) but still marks the doc read. A no-op save isn't proof of a write; check the mtime. <!-- src: crates/hyprlayer-core/src/runtime/library.rs:361 -->
- External edits are adopted silently only while the doc is clean. The conflict path needs a dirty draft first. <!-- src: src/features/library/useDocSave.ts:52 -->
- The guard covers in-app navigation only (`enableBeforeUnload: false`). A reload drops unsaved edits without asking. <!-- src: src/features/library/useDocSave.ts:103 -->
- `Reveal in Finder` opens Finder outside the webview. Proof stops at the button.
