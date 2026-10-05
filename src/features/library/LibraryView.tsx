import { useEffect, useState } from "react";

import { PaneFrame } from "@/components/paneFrame";
import { EmptyState } from "@/components/ui";
import { useCommand } from "@/lib/keybindings/registry";
import { useBootstrap, useDocCounts } from "@/lib/queries";
import {
  LIBRARY_LIST_MAX,
  LIBRARY_LIST_MIN,
  LIBRARY_NAV_WIDTH,
  SIDE_PANEL_MIN,
  useUiStore,
} from "@/lib/uiStore";
import { useViewport } from "@/lib/useViewport";
import { CreateDocDialog } from "./CreateDocDialog";
import { DocDetail } from "./DocDetail";
import { LibraryBlank } from "./LibraryBlank";
import { LibraryNav } from "./LibraryNav";
import { useLibraryScope } from "./libraryScope";
import type { LibrarySearch } from "./librarySearch";
import { MessageList } from "./MessageList";
import { resolveSyncProjectPath } from "./thoughtsSync";

export function LibraryView({
  search,
  onUpdate,
}: {
  search: LibrarySearch;
  onUpdate: (patch: Partial<LibrarySearch>) => void;
}) {
  const bootstrap = useBootstrap();
  const config = bootstrap.data?.config;
  const projects = bootstrap.data?.projects ?? [];
  // Assume configured until bootstrap lands so "Set up thoughts" never flashes.
  const thoughtsConfigured = bootstrap.data === undefined || (config?.state === "ready" && config.configured);
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
  const targetProject = resolveSyncProjectPath(search, lastProject, projects);

  const listWidth = useUiStore((s) => s.libraryListWidth);
  const setListWidth = useUiStore((s) => s.setLibraryListWidth);
  const navCollapsed = useUiStore((s) => s.libraryCollapsed);
  const narrowDrawer = useUiStore((s) => s.narrowDrawer);
  const setNarrowDrawer = useUiStore((s) => s.setNarrowDrawer);
  const { narrow } = useViewport();

  useEffect(() => {
    if (useUiStore.getState().narrowDrawer === "library") setNarrowDrawer(null);
  }, [search.box, search.project, setNarrowDrawer]);

  const total = useDocCounts(undefined).data?.total;
  const projectName = projects.find((p) => p.thoughtsProject === search.project)?.mappedName ?? search.project;

  const createDialog = (
    <CreateDocDialog open={createOpen} onClose={() => setCreateOpen(false)} initialProjectName={search.project} />
  );

  // Unscoped, so an empty project keeps the nav and its scope picker.
  if (!thoughtsConfigured || total === 0) {
    return (
      <>
        <LibraryBlank
          thoughtsConfigured={thoughtsConfigured}
          targetProject={targetProject}
          onNewNote={() => setCreateOpen(true)}
        />
        {createDialog}
      </>
    );
  }

  return (
    <>
      <PaneFrame
        nav={
          <LibraryNav
            search={search}
            onUpdate={onUpdate}
            projects={projects}
            onNewDoc={() => setCreateOpen(true)}
          />
        }
        list={<MessageList search={search} onUpdate={onUpdate} projects={projects} projectName={projectName} />}
        detail={search.doc ? <DocDetail id={search.doc} /> : <EmptyState title="Select a document" />}
        navWidth={LIBRARY_NAV_WIDTH}
        navShown={narrow ? narrowDrawer === "library" : !navCollapsed}
        overlayNav={narrow}
        listWidth={narrow ? SIDE_PANEL_MIN : listWidth}
        listMin={LIBRARY_LIST_MIN}
        listMax={LIBRARY_LIST_MAX}
        onListWidth={setListWidth}
        listLabel="Resize library list"
      />
      {createDialog}
    </>
  );
}
