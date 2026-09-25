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
import { SIDE_PANEL_MAX, SIDE_PANEL_MIN, useUiStore } from "@/lib/uiStore";
import { startEdgeDrag } from "@/lib/resize";
import { useViewport } from "@/lib/useViewport";
import { useCommand } from "@/lib/keybindings/registry";
import type { LibrarySearch } from "@/features/library/librarySearch";

export function LibraryPage() {
  const search = useSearch({ from: "/library" });
  const navigate = useNavigate({ from: "/library" });
  const bootstrap = useBootstrap();
  const [createOpen, setCreateOpen] = useState(false);

  useCommand("doc.new", () => setCreateOpen(true));

  const setLastProject = useLibraryScope((s) => s.setLastProject);
  useEffect(() => {
    if (search.project) setLastProject(search.project);
  }, [search.project, setLastProject]);

  const setLastSearch = useLibraryScope((s) => s.setLastSearch);
  const setProjectSearch = useLibraryScope((s) => s.setProjectSearch);
  useEffect(() => {
    const view = { ...search };
    delete view.from;
    setLastSearch(view);
    if (view.project) setProjectSearch(view.project, view);
  }, [search, setLastSearch, setProjectSearch]);

  const lastProject = useLibraryScope((s) => s.lastProject);
  const thoughtsSync = useThoughtsSync(
    resolveSyncProjectPath(search, lastProject, bootstrap.data?.projects ?? []),
  );

  const libraryWidth = useUiStore((s) => s.libraryWidth);
  const setLibraryWidth = useUiStore((s) => s.setLibraryWidth);
  const libraryCollapsed = useUiStore((s) => s.libraryCollapsed);
  const { narrow } = useViewport();

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
        width={narrow ? SIDE_PANEL_MIN : libraryWidth}
        collapsed={libraryCollapsed}
      />

      {!libraryCollapsed && !narrow && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize library sidebar"
          className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
          onPointerDown={(e) =>
            startEdgeDrag(e, {
              startWidth: libraryWidth,
              min: SIDE_PANEL_MIN,
              max: SIDE_PANEL_MAX,
              direction: 1,
              onResize: setLibraryWidth,
            })
          }
        />
      )}

      <section className="min-w-0 flex-1">
        {search.doc ? (
          <DocDetail id={search.doc} />
        ) : (
          <EmptyState title="Select a document" />
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
