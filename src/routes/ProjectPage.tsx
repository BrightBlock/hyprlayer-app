import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useEffect, useState, type PointerEvent as ReactPointerEvent } from "react";

import {
  Button,
  ContextMenu,
  ContextMenuItem,
  Dialog,
  IconButton,
  PageError,
  PageSpinner,
  TextInput,
} from "../components/ui";
import { BookIcon, DocIcon, ModeIcon, PlusIcon, XIcon } from "../components/icons";
import { CreateWorkstreamDialog } from "../features/workstreams/CreateWorkstreamDialog";
import { DiffSidebar } from "../features/workstreams/DiffSidebar";
import { SessionChat, SessionStatusBadge, SessionTabDot } from "../features/session/SessionChat";
import { DocPreviewTab } from "../features/session/DocPreviewTab";
import { killSession, transcriptToText } from "../features/session/sessionStore";
import { MODE_STYLES, stageLabel, stageOf, type StageKey } from "../features/session/stages";
import {
  defaultTabState,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
  type WsTabState,
} from "../features/session/workstreamTabs";
import { WorkstreamChips } from "../features/workstreams/WorkstreamChips";
import { WorkstreamDocsPanel } from "../features/workstreams/WorkstreamDocsPanel";
import { useBootstrap, useSetWorkstreamStage, useWorkstreams } from "../lib/queries";
import { startEdgeDrag } from "../lib/resize";

const DIFF_SIDEBAR_COLLAPSED = 56;
const DIFF_SIDEBAR_MIN = 280;
const DIFF_SIDEBAR_MAX = 560;

/** A doc attached as context to ONE agent tab. */
export type AttachedDoc = {
  path: string;
  role: string;
};

/** A chat tab inside ONE workstream. Each workstream owns its own set.
 *  Context docs are PER TAB — each agent carries its own context. Once an
 *  agent session is started from this tab, `sessionId` binds the tab to that
 *  live (or terminal) session; its transcript lives in the session store, so
 *  it survives tab/workstream switches and re-attaches on demand. */
export type ShellTab = {
  id: number;
  title: string;
  mode: StageKey;
  draft: string;
  docs: AttachedDoc[];
  /** Model + reasoning-effort keys chosen for the NEXT session this tab starts
   *  (see models.ts); fixed at spawn. Undefined → the defaults. */
  model?: string;
  effort?: string;
  sessionId?: string;
  /** When set, this tab is a read-only markdown preview of a doc (its path),
   *  not an agent chat — opened from the diff sidebar, closed when done. */
  docPreviewPath?: string;
};

type ShellMenu = {
  shellId: number;
  x: number;
  y: number;
};

export function ProjectPage() {
  const { projectPath } = useParams({ from: "/projects/$projectPath" });
  const bootstrap = useBootstrap();
  const workstreams = useWorkstreams(projectPath);
  const setStage = useSetWorkstreamStage(projectPath);

  const search = useSearch({ from: "/projects/$projectPath" });
  const navigate = useNavigate({ from: "/projects/$projectPath" });

  const storageKey = `hyprlayer:last-workstream:${projectPath}`;
  // Selection is DERIVED from the URL every render — never mirrored into
  // state. (A state mirror + URL canonicalizer once fought each other into
  // an infinite navigate loop when switching between two workstreams.)
  const activeItemId = search.item ?? localStorage.getItem(storageKey);

  // Shell tabs are PER WORKSTREAM (1 workstream -> N tabs), keyed by workstream
  // id so switching workstreams never mixes tabs. Held in a module store so a
  // tab's bound session (and its draft/docs) survives leaving this page.
  const tabsByWs = useWorkstreamTabsStore((s) => s.byWs);
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [renameShellId, setRenameShellId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [diffCollapsed, setDiffCollapsed] = useState(false);
  const [diffWidth, setDiffWidth] = useState(360);

  // ?create=true (sidebar "+") pops the dialog once, then leaves the URL.
  useEffect(() => {
    if (search.create) {
      setCreateOpen(true);
      void navigate({
        search: (prev) => ({ ...prev, create: undefined }),
        replace: true,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.create]);

  // Hooks live above the early returns. Selection resolves to the URL's
  // ?item= when it names a live workstream, else last-used, else the first.
  const views = (workstreams.data ?? []).filter((view) => view.item.status === "active");
  const activeView = views.find((view) => view.item.id === activeItemId) ?? views[0] ?? null;
  const wsId = activeView?.item.id ?? null;

  // Canonicalize the URL to the resolved selection. A valid ?item= always
  // resolves to itself (no rewrite, no loop); we only write when the param
  // is missing, or names something the SETTLED list doesn't contain (e.g.
  // archived elsewhere) — never mid-refetch, where a just-created id can
  // legitimately be absent for a moment.
  useEffect(() => {
    if (!wsId) return;
    localStorage.setItem(storageKey, wsId);
    const stale =
      search.item !== undefined &&
      search.item !== wsId &&
      workstreams.isSuccess &&
      !workstreams.isFetching;
    if (search.item === undefined || stale) {
      void navigate({
        search: (prev) => ({ ...prev, item: wsId }),
        replace: true,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wsId, search.item, storageKey, workstreams.isSuccess, workstreams.isFetching]);

  if (bootstrap.isLoading || workstreams.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;
  if (workstreams.isError) return <PageError error={workstreams.error} />;

  // The bootstrap cache already names every project — no per-page
  // `storage info` subprocess needed just for the mapped name.
  const project = bootstrap.data!.projects.find((p) => p.path === projectPath);
  if (!project) {
    return (
      <PageError
        error={{
          code: "project_not_found",
          message: `No hyprlayer project is mapped at ${projectPath}`,
          remedy: "Add the repository as a project first.",
        }}
      />
    );
  }

  // The active workstream's own tab strip (1 workstream -> N shells).
  const tabs = wsId ? (tabsByWs[wsId] ?? defaultTabState()) : null;
  const shells = tabs?.shells ?? [];
  const docsActive = tabs?.docsOpen ?? false;
  const activeShell = tabs
    ? (tabs.shells.find((s) => s.id === tabs.activeId) ?? tabs.shells[0] ?? null)
    : null;
  const contextShell = shellMenu ? shells.find((s) => s.id === shellMenu.shellId) : null;
  const shellBeingRenamed = renameShellId
    ? shells.find((s) => s.id === renameShellId)
    : null;

  const wsStage: StageKey = activeView ? stageOf(activeView) : "general";
  const mode: StageKey = activeShell?.mode ?? "general";

  const updateTabs = (updater: (current: WsTabState) => WsTabState) => {
    if (!wsId) return;
    updateWorkstreamTabs((all) => ({ ...all, [wsId]: updater(all[wsId] ?? defaultTabState()) }));
  };
  const patchShell = (
    shellId: number,
    patch: Partial<
      Pick<ShellTab, "title" | "mode" | "draft" | "docs" | "model" | "effort" | "sessionId">
    >,
  ) => {
    updateTabs((current) => ({
      ...current,
      shells: current.shells.map((s) => (s.id === shellId ? { ...s, ...patch } : s)),
    }));
  };
  const selectShell = (shellId: number) => {
    updateTabs((current) => ({ ...current, activeId: shellId, docsOpen: false }));
  };
  const selectDocsTab = () => {
    updateTabs((current) => ({ ...current, docsOpen: true }));
  };
  const addShell = () => {
    updateTabs((current) => {
      const nextId = Math.max(0, ...current.shells.map((s) => s.id)) + 1;
      return {
        shells: [
          ...current.shells,
          { id: nextId, title: `Agent ${nextId}`, mode: "general", draft: "", docs: [] },
        ],
        activeId: nextId,
        docsOpen: false,
      };
    });
  };
  // Open an agent-created doc as a read-only preview tab in the agent strip —
  // reuse an existing preview tab for the same doc rather than stacking dupes.
  const openDocPreview = (path: string, title: string) => {
    updateTabs((current) => {
      const existing = current.shells.find((s) => s.docPreviewPath === path);
      if (existing) return { ...current, activeId: existing.id, docsOpen: false };
      const nextId = Math.max(0, ...current.shells.map((s) => s.id)) + 1;
      return {
        ...current,
        shells: [
          ...current.shells,
          { id: nextId, title, mode: "general", draft: "", docs: [], docPreviewPath: path },
        ],
        activeId: nextId,
        docsOpen: false,
      };
    });
  };
  const closeShell = (shellId: number) => {
    if (!tabs || tabs.shells.length === 1) return;
    // Closing the tab ends its session — there's no End button; the window IS
    // the handle. (No-op if the session already ended or was never started.)
    const sid = tabs.shells.find((s) => s.id === shellId)?.sessionId;
    if (sid) void killSession(sid);
    updateTabs((current) => {
      if (current.shells.length === 1) return current;
      const index = current.shells.findIndex((s) => s.id === shellId);
      const remaining = current.shells.filter((s) => s.id !== shellId);
      const activeId =
        current.activeId === shellId
          ? (remaining[Math.max(0, index - 1)] ?? remaining[0]).id
          : current.activeId;
      return { ...current, shells: remaining, activeId };
    });
  };
  const openDoc = (id: string) => {
    void navigate({ to: "/library/doc", search: { id, mode: "preview" } });
  };
  // Context docs are per agent tab: attach/detach mutate the ACTIVE shell.
  const attachDocToShell = (path: string, role: string) => {
    if (!activeShell) return;
    const docs = activeShell.docs.some((d) => d.path === path)
      ? activeShell.docs
      : [...activeShell.docs, { path, role }];
    patchShell(activeShell.id, { docs });
  };
  const detachDocFromShell = (path: string) => {
    if (!activeShell) return;
    patchShell(activeShell.id, { docs: activeShell.docs.filter((d) => d.path !== path) });
  };

  // The URL is the single source of truth; the canonicalize effect persists
  // the last-used id once the selection settles.
  const selectItem = (id: string) => {
    void navigate({
      search: (prev) => ({ ...prev, item: id }),
      replace: true,
    });
  };
  const updateDraft = (value: string) => {
    if (activeShell) patchShell(activeShell.id, { draft: value });
  };
  const changeMode = (key: StageKey) => {
    if (!activeView || !activeShell || key === mode) return;
    patchShell(activeShell.id, { mode: key });
    // The workstream's pipeline position follows the latest mode choice.
    setStage.mutate({ id: activeView.item.id, stage: key });
  };
  const changeModel = (key: string) => {
    if (activeShell) patchShell(activeShell.id, { model: key });
  };
  const changeEffort = (key: string) => {
    if (activeShell) patchShell(activeShell.id, { effort: key });
  };
  const openRenameShell = (shell: ShellTab) => {
    setRenameShellId(shell.id);
    setRenameValue(shell.title);
  };
  const closeRenameShell = () => {
    setRenameShellId(null);
    setRenameValue("");
  };
  const commitRenameShell = () => {
    if (!shellBeingRenamed) return;
    const nextTitle = renameValue.trim();
    if (!nextTitle) return;
    patchShell(shellBeingRenamed.id, { title: nextTitle });
    closeRenameShell();
  };
  const copyShellTranscript = (shell: ShellTab) => {
    const body = shell.sessionId
      ? transcriptToText(shell.sessionId)
      : `Draft (no session started yet):\n${shell.draft || "(empty)"}`;
    const transcript = [
      `# ${activeView?.item.title ?? project.mappedName} — ${shell.title}`,
      `Project: ${project.mappedName}`,
      `Branch: ${activeView?.item.branch ?? "—"}`,
      `Mode: ${stageLabel(shell.mode)}`,
      "",
      body,
      "",
    ].join("\n");
    void navigator.clipboard.writeText(transcript);
  };
  const startDiffResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (diffCollapsed) return;
    startEdgeDrag(event, {
      startWidth: diffWidth,
      min: DIFF_SIDEBAR_MIN,
      max: DIFF_SIDEBAR_MAX,
      direction: -1,
      onResize: setDiffWidth,
    });
  };

  return (
    <div className="flex h-full min-h-0 bg-surface">
      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex min-h-0 flex-1">
          <main className="flex min-w-0 flex-1 flex-col">
            <div className="border-b border-edge bg-panel px-5">
              <div className="flex min-w-0 items-center justify-between gap-4">
                <div className="flex min-w-0 gap-1 overflow-x-auto py-2">
                  {activeView &&
                    shells.map((shell) => (
                      <div
                        key={shell.id}
                        className={`inline-flex shrink-0 items-center rounded-md ${
                          shell.id === activeShell?.id
                            ? "bg-panel-2 text-ink"
                            : "text-ink-dim hover:bg-panel-2 hover:text-ink"
                        }`}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          selectShell(shell.id);
                          setShellMenu({ shellId: shell.id, x: e.clientX, y: e.clientY });
                        }}
                      >
                        <button
                          type="button"
                          onClick={() => selectShell(shell.id)}
                          className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium"
                        >
                          {shell.docPreviewPath ? (
                            <span className="text-ink-dim">
                              <DocIcon size={13} />
                            </span>
                          ) : (
                            <span className={`rounded p-0.5 ${MODE_STYLES[shell.mode].badge}`}>
                              <ModeIcon mode={shell.mode} />
                            </span>
                          )}
                          {shell.title}
                          {!shell.docPreviewPath && <SessionTabDot sessionId={shell.sessionId} />}
                        </button>
                        {shells.length > 1 && (
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              closeShell(shell.id);
                            }}
                            className="mr-1 inline-flex h-5 w-5 items-center justify-center rounded text-ink-dim hover:bg-edge hover:text-ink"
                            title={`Close ${shell.title}`}
                            aria-label={`Close ${shell.title}`}
                          >
                            <XIcon />
                          </button>
                        )}
                      </div>
                    ))}
                  {activeView && (
                    <IconButton onClick={addShell} title="New tab" aria-label="New tab">
                      <PlusIcon />
                    </IconButton>
                  )}
                </div>
                {activeView && (
                  <div className="flex min-w-0 items-center gap-2">
                    <WorkstreamChips view={activeView} />
                    <span
                      className={`shrink-0 rounded px-1.5 py-0.5 text-xs font-medium ${MODE_STYLES[wsStage].badge}`}
                    >
                      {stageLabel(wsStage)}
                    </span>
                    <SessionStatusBadge sessionId={activeShell?.sessionId} />
                    <IconButton
                      onClick={selectDocsTab}
                      active={docsActive}
                      title="Project documents"
                      aria-label="Project documents"
                    >
                      <BookIcon />
                    </IconButton>
                  </div>
                )}
              </div>
            </div>
            {shellMenu && contextShell && (
              <ContextMenu position={shellMenu} onClose={() => setShellMenu(null)}>
                <ContextMenuItem
                  onClick={() => {
                    openRenameShell(contextShell);
                    setShellMenu(null);
                  }}
                >
                  Rename tab
                </ContextMenuItem>
                <ContextMenuItem
                  onClick={() => {
                    copyShellTranscript(contextShell);
                    setShellMenu(null);
                  }}
                >
                  Copy transcript
                </ContextMenuItem>
                <ContextMenuItem
                  disabled={shells.length === 1}
                  onClick={() => {
                    closeShell(contextShell.id);
                    setShellMenu(null);
                  }}
                >
                  Close tab
                </ContextMenuItem>
              </ContextMenu>
            )}
            <Dialog open={Boolean(shellBeingRenamed)} onClose={closeRenameShell} title="Rename tab">
              <form
                className="space-y-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  commitRenameShell();
                }}
              >
                <TextInput
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  autoFocus
                />
                <div className="flex justify-end gap-2">
                  <Button type="button" onClick={closeRenameShell}>
                    Cancel
                  </Button>
                  <Button type="submit" variant="primary" disabled={!renameValue.trim()}>
                    Rename
                  </Button>
                </div>
              </form>
            </Dialog>
            <CreateWorkstreamDialog
              projectPath={projectPath}
              open={createOpen}
              onClose={() => setCreateOpen(false)}
              onCreated={(view) => selectItem(view.item.id)}
            />

            {docsActive && activeView ? (
              <div className="min-h-0 flex-1 overflow-y-auto">
                <WorkstreamDocsPanel
                  mappedName={project.mappedName}
                  targetTitle={activeShell?.title ?? null}
                  attachedPaths={(activeShell?.docs ?? []).map((d) => d.path)}
                  onAttach={attachDocToShell}
                  onDetach={detachDocFromShell}
                  onOpenDoc={openDoc}
                />
              </div>
            ) : activeView && activeShell?.docPreviewPath ? (
              <DocPreviewTab
                key={`${activeView.item.id}:${activeShell.id}`}
                path={activeShell.docPreviewPath}
                title={activeShell.title}
              />
            ) : activeView && activeShell ? (
              <SessionChat
                key={`${activeView.item.id}:${activeShell.id}`}
                view={activeView}
                shell={activeShell}
                onDraftChange={updateDraft}
                onModeChange={changeMode}
                onModelChange={changeModel}
                onEffortChange={changeEffort}
                onSessionStarted={(sessionId) => patchShell(activeShell.id, { sessionId })}
                onResetSession={() => patchShell(activeShell.id, { sessionId: undefined })}
                onOpenDoc={openDoc}
                onDetachDoc={detachDocFromShell}
                onBrowseDocs={selectDocsTab}
              />
            ) : (
              <div className="min-h-0 flex-1 overflow-y-auto p-5">
                <div className="mx-auto max-w-3xl space-y-4">
                  <div className="rounded-lg border border-edge bg-panel p-6 text-center">
                    <p className="text-base font-medium text-ink">Create your first workstream</p>
                    <p className="mx-auto mt-1 max-w-md text-sm leading-relaxed text-ink-dim">
                      Each workstream gets its own branch and worktree, so agents can research,
                      plan, and implement without touching your checkout.
                    </p>
                    <Button variant="primary" className="mt-4" onClick={() => setCreateOpen(true)}>
                      <PlusIcon />
                      New workstream
                    </Button>
                  </div>
                </div>
              </div>
            )}
          </main>
        </div>
      </section>

      {!diffCollapsed && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize diff sidebar"
          className="w-1 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
          onPointerDown={startDiffResize}
        />
      )}

      <aside
        className="min-w-0 shrink-0 border-l border-edge bg-panel"
        style={{ width: diffCollapsed ? DIFF_SIDEBAR_COLLAPSED : diffWidth }}
      >
        <DiffSidebar
          workstreamId={activeView?.item.id ?? null}
          collapsed={diffCollapsed}
          onToggleCollapsed={() => setDiffCollapsed((value) => !value)}
          onOpenDoc={openDocPreview}
        />
      </aside>
    </div>
  );
}
