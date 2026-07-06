import { useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";

import { EmptyState } from "@/components/ui";
import { CreateDocDialog } from "@/features/library/CreateDocDialog";
import { docQueryFromSearch } from "@/features/library/DocFilters";
import { DocDetail } from "@/features/library/DocDetail";
import { LibrarySidebar } from "@/features/library/LibrarySidebar";
import { resolveSyncProjectPath } from "@/features/library/thoughtsSync";
import { useThoughtsSync } from "@/features/library/useThoughtsSync";
import { useBootstrap, useDocs } from "@/lib/queries";
import { useLibraryScope } from "@/features/library/libraryScope";
import { LIBRARY_SIDEBAR_MAX, LIBRARY_SIDEBAR_MIN, useUiStore } from "@/lib/uiStore";
import { startEdgeDrag } from "@/lib/resize";
import { useCommand } from "@/lib/keybindings/registry";
import type { LibrarySearch } from "@/features/library/librarySearch";

/**
 * The Library: one master–detail view consolidating the old global browser,
 * per-project docs page, and full-screen doc editor. The "LIBRARY" panel (list +
 * project scope + filters) sits beside the inline document detail; everything —
 * filters, scope, the selected doc, and its body mode — lives in the URL.
 */
export function LibraryPage() {
  const search = useSearch({ from: "/library" });
  const navigate = useNavigate({ from: "/library" });
  const bootstrap = useBootstrap();
  const [createOpen, setCreateOpen] = useState(false);

  // "New Document" — registered only while the Library is open (its context).
  useCommand("doc.new", () => setCreateOpen(true));

  // Remember a real project scope as the most-recently-used one so the next
  // Library open defaults to it. "All projects" (no param) deliberately doesn't
  // overwrite it — it's a transient widening, not a new default.
  const setLastProject = useLibraryScope((s) => s.setLastProject);
  useEffect(() => {
    if (search.project) setLastProject(search.project);
  }, [search.project, setLastProject]);

  // Remember the whole view — scope, filters, and the open doc + mode — so
  // re-entering the Library from the nav rail resumes this tab where the user
  // left it instead of re-scoping and dropping the open document. `from` is
  // navigational provenance (which project's "Docs" opened it), not view state.
  const setLastSearch = useLibraryScope((s) => s.setLastSearch);
  const setProjectSearch = useLibraryScope((s) => s.setProjectSearch);
  useEffect(() => {
    const view = { ...search };
    delete view.from; // provenance, not view state — see the store's doc comment
    setLastSearch(view);
    // Also under its project, so that project's "Docs" entry reopens to it.
    if (view.project) setProjectSearch(view.project, view);
  }, [search, setLastSearch, setProjectSearch]);

  // Entering the Library (or re-scoping it) opportunistically runs
  // `hyprlayer thoughts sync` — pull + push, the same command the CLI and
  // skills use — so docs synced from elsewhere show up without a manual step.
  // The sidebar's sync icon surfaces the state and re-triggers on demand.
  const lastProject = useLibraryScope((s) => s.lastProject);
  const thoughtsSync = useThoughtsSync(
    resolveSyncProjectPath(search, lastProject, bootstrap.data?.projects ?? []),
  );

  // Drag-resizable list-panel width + collapsed state, in the UI store so both
  // survive leaving and re-opening the Library (the route unmounts; the store
  // doesn't). `resizingLibrary` is transient drag state, so it stays local.
  const libraryWidth = useUiStore((s) => s.libraryWidth);
  const setLibraryWidth = useUiStore((s) => s.setLibraryWidth);
  const libraryCollapsed = useUiStore((s) => s.libraryCollapsed);
  const [resizingLibrary, setResizingLibrary] = useState(false);

  const query = useMemo(() => docQueryFromSearch(search), [search]);
  const docs = useDocs(query);

  const update = (patch: Partial<LibrarySearch>) =>
    void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true });

  return (
    <div className="flex h-full min-h-0">
      <LibrarySidebar
        search={search}
        onUpdate={update}
        projects={bootstrap.data?.projects ?? []}
        result={docs.data}
        isLoading={docs.isLoading}
        isError={docs.isError}
        error={docs.error}
        onNewDoc={() => setCreateOpen(true)}
        sync={thoughtsSync}
        width={libraryWidth}
        collapsed={libraryCollapsed}
        resizing={resizingLibrary}
      />

      {!libraryCollapsed && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize library sidebar"
          className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
          onPointerDown={(e) =>
            startEdgeDrag(e, {
              startWidth: libraryWidth,
              min: LIBRARY_SIDEBAR_MIN,
              max: LIBRARY_SIDEBAR_MAX,
              direction: 1,
              onResize: setLibraryWidth,
              onStart: () => setResizingLibrary(true),
              onEnd: () => setResizingLibrary(false),
            })
          }
        />
      )}

      <section className="min-w-0 flex-1">
        {search.doc ? (
          <DocDetail
            id={search.doc}
            mode={search.mode ?? "preview"}
            onSetMode={(mode) => update({ mode: mode === "edit" ? "edit" : undefined })}
          />
        ) : (
          <EmptyState title="Select a document">
            Pick a document from the list to read or edit it.
          </EmptyState>
        )}
      </section>

      <CreateDocDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        initialProjectName={search.project}
      />
    </div>
  );
}
