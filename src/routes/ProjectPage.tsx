import { useParams } from "@tanstack/react-router";
import { useEffect, useState, type PointerEvent as ReactPointerEvent } from "react";

import { Badge, Button, Dialog, ErrorNote, TextInput, Spinner } from "../components/ui";
import { ArchiveWorkItemDialog } from "../features/work-items/ArchiveWorkItemDialog";
import { CreateWorkItemDialog } from "../features/work-items/CreateWorkItemDialog";
import { DiffSidebar } from "../features/work-items/DiffSidebar";
import { WorkItemChips } from "../features/work-items/WorkItemChips";
import type { WorkItemView } from "../lib/bindings";
import {
  useProjectDetail,
  useRenameWorkItem,
  useSetWorkItemStage,
  useWorkItems,
} from "../lib/queries";

const DIFF_SIDEBAR_COLLAPSED = 56;
const DIFF_SIDEBAR_MIN = 280;
const DIFF_SIDEBAR_MAX = 560;

const STAGES = [
  { key: "general", label: "General" },
  { key: "research", label: "Research" },
  { key: "plan", label: "Plan" },
  { key: "implement", label: "Implement" },
  { key: "validate", label: "Validate" },
  { key: "ship", label: "Ship" },
] as const;

type StageKey = (typeof STAGES)[number]["key"];

const STAGE_KEYS = new Set<string>(STAGES.map((s) => s.key));

/** Backend stage is a plain string; anything unknown renders as general. */
function stageOf(view: WorkItemView): StageKey {
  return (STAGE_KEYS.has(view.item.stage) ? view.item.stage : "general") as StageKey;
}

type ShellMenu = {
  itemId: string;
  x: number;
  y: number;
};

const MODE_STYLES: Record<
  StageKey,
  {
    activeButton: string;
    badge: string;
    composer: string;
  }
> = {
  general: {
    activeButton: "border-edge bg-edge/60 text-ink",
    badge: "bg-edge/60 text-ink-dim",
    composer: "border-edge",
  },
  research: {
    activeButton: "border-sky-400/40 bg-sky-400/10 text-sky-300",
    badge: "bg-sky-400/10 text-sky-300",
    composer: "border-sky-400/40",
  },
  plan: {
    activeButton: "border-amber-400/40 bg-amber-400/10 text-amber-300",
    badge: "bg-amber-400/10 text-amber-300",
    composer: "border-amber-400/40",
  },
  implement: {
    activeButton: "border-emerald-400/40 bg-emerald-400/10 text-emerald-300",
    badge: "bg-emerald-400/10 text-emerald-300",
    composer: "border-emerald-400/40",
  },
  validate: {
    activeButton: "border-violet-400/40 bg-violet-400/10 text-violet-300",
    badge: "bg-violet-400/10 text-violet-300",
    composer: "border-violet-400/40",
  },
  ship: {
    activeButton: "border-rose-400/40 bg-rose-400/10 text-rose-300",
    badge: "bg-rose-400/10 text-rose-300",
    composer: "border-rose-400/40",
  },
};

function PlusIcon() {
  return (
    <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

function XIcon() {
  return (
    <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none">
      <path d="m7 7 10 10M17 7 7 17" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

function ModeIcon({ mode }: { mode: (typeof STAGES)[number]["key"] }) {
  switch (mode) {
    case "research":
      return (
        <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
          <circle cx="10.5" cy="10.5" r="5.5" stroke="currentColor" strokeWidth="1.8" />
          <path d="m15 15 4 4" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      );
    case "plan":
      return (
        <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
          <path d="M7 7h11M7 12h11M7 17h11" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          <path d="m3.5 7 .8.8L6 6M3.5 12l.8.8L6 11M3.5 17l.8.8L6 16" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "implement":
      return (
        <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
          <path d="m9 8-4 4 4 4M15 8l4 4-4 4" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "validate":
      return (
        <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
          <path d="m5 12 4 4 10-10" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "ship":
      return (
        <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
          <path d="M12 3 7 13l5-2 5 2-5-10Z" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" />
          <path d="M8 17h8M10 21h4" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      );
    case "general":
      return (
        <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
          <path d="M5 6.5A2.5 2.5 0 0 1 7.5 4h9A2.5 2.5 0 0 1 19 6.5v6A2.5 2.5 0 0 1 16.5 15H11l-4 4v-4A2.5 2.5 0 0 1 5 12.5v-6Z" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" />
        </svg>
      );
  }
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function ProjectPage() {
  const { projectPath } = useParams({ from: "/projects/$projectPath" });
  const detail = useProjectDetail(projectPath);
  const workItems = useWorkItems(projectPath);
  const renameItem = useRenameWorkItem(projectPath);
  const setStage = useSetWorkItemStage(projectPath);

  const storageKey = `hyprlayer:last-work-item:${projectPath}`;
  const [activeItemId, setActiveItemId] = useState<string | null>(() =>
    localStorage.getItem(storageKey),
  );
  // Drafts stay local until sessions land; keyed by work-item id.
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [archiveTarget, setArchiveTarget] = useState<WorkItemView | null>(null);
  const [renameItemId, setRenameItemId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [diffCollapsed, setDiffCollapsed] = useState(false);
  const [diffWidth, setDiffWidth] = useState(360);

  useEffect(() => {
    setActiveItemId(localStorage.getItem(storageKey));
  }, [storageKey]);

  useEffect(() => {
    if (!shellMenu) return;
    const close = () => setShellMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [shellMenu]);

  if (detail.isLoading || workItems.isLoading) {
    return (
      <div className="flex justify-center p-12">
        <Spinner />
      </div>
    );
  }
  if (detail.isError) {
    return (
      <div className="p-8">
        <ErrorNote error={detail.error} />
      </div>
    );
  }
  if (workItems.isError) {
    return (
      <div className="p-8">
        <ErrorNote error={workItems.error} />
      </div>
    );
  }

  const project = detail.data!.project;
  const views = (workItems.data ?? []).filter((view) => view.item.status === "active");
  const activeView = views.find((view) => view.item.id === activeItemId) ?? views[0] ?? null;
  const contextView = shellMenu
    ? views.find((view) => view.item.id === shellMenu.itemId)
    : null;
  const viewBeingRenamed = renameItemId
    ? views.find((view) => view.item.id === renameItemId)
    : null;
  const stage: StageKey = activeView ? stageOf(activeView) : "general";
  const currentStage = STAGES.find((s) => s.key === stage)!;
  const modeStyle = MODE_STYLES[stage];
  const draft = activeView ? (drafts[activeView.item.id] ?? "") : "";

  const selectItem = (id: string | null) => {
    setActiveItemId(id);
    if (id === null) {
      localStorage.removeItem(storageKey);
    } else {
      localStorage.setItem(storageKey, id);
    }
  };
  const updateDraft = (value: string) => {
    if (!activeView) return;
    const id = activeView.item.id;
    setDrafts((current) => ({ ...current, [id]: value }));
  };
  const changeStage = (key: StageKey) => {
    if (!activeView || key === stage) return;
    setStage.mutate({ id: activeView.item.id, stage: key });
  };
  const openRename = (view: WorkItemView) => {
    setRenameItemId(view.item.id);
    setRenameValue(view.item.title);
  };
  const closeRename = () => {
    setRenameItemId(null);
    setRenameValue("");
    renameItem.reset();
  };
  const commitRename = () => {
    if (!viewBeingRenamed || renameItem.isPending) return;
    const nextTitle = renameValue.trim();
    if (!nextTitle) return;
    renameItem.mutate(
      { id: viewBeingRenamed.item.id, title: nextTitle },
      { onSuccess: closeRename },
    );
  };
  const handleArchived = (id: string) => {
    setDrafts(({ [id]: _removed, ...rest }) => rest);
    if (activeView?.item.id !== id) return;
    const index = views.findIndex((view) => view.item.id === id);
    const fallback = views[index - 1] ?? views[index + 1] ?? null;
    selectItem(fallback ? fallback.item.id : null);
  };
  const copyShellTranscript = (view: WorkItemView) => {
    const mode = STAGES.find((s) => s.key === stageOf(view))?.label ?? view.item.stage;
    const transcript = [
      `# ${view.item.title}`,
      `Project: ${project.mappedName}`,
      `Branch: ${view.item.branch}`,
      `Mode: ${mode}`,
      "",
      "Transcript is not connected yet. Current draft:",
      drafts[view.item.id] || "(empty)",
      "",
    ].join("\n");
    void navigator.clipboard.writeText(transcript);
  };
  const startDiffResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (diffCollapsed) return;

    event.preventDefault();
    const startX = event.clientX;
    const startWidth = diffWidth;
    const previousCursor = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;

    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    const onPointerMove = (moveEvent: PointerEvent) => {
      setDiffWidth(
        clamp(
          startWidth - (moveEvent.clientX - startX),
          DIFF_SIDEBAR_MIN,
          DIFF_SIDEBAR_MAX,
        ),
      );
    };
    const onPointerUp = () => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousSelect;
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, { once: true });
  };

  return (
    <div className="flex h-full min-h-0 bg-surface">
      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex min-h-0 flex-1">
          <main className="flex min-w-0 flex-1 flex-col">
            <div className="border-b border-edge bg-panel px-5">
              <div className="flex min-w-0 items-center justify-between gap-4">
                <div className="flex min-w-0 gap-1 overflow-x-auto py-2">
                  {views.map((view) => (
                    <div
                      key={view.item.id}
                      className={`inline-flex shrink-0 items-center rounded-md ${
                        view.item.id === activeView?.item.id
                          ? "bg-panel-2 text-ink"
                          : "text-ink-dim hover:bg-panel-2 hover:text-ink"
                      }`}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        selectItem(view.item.id);
                        setShellMenu({ itemId: view.item.id, x: e.clientX, y: e.clientY });
                      }}
                    >
                      <button
                        type="button"
                        onClick={() => selectItem(view.item.id)}
                        className="px-3 py-1.5 text-sm font-medium"
                      >
                        {view.item.title}
                      </button>
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          setArchiveTarget(view);
                        }}
                        className="mr-1 inline-flex h-5 w-5 items-center justify-center rounded text-ink-dim hover:bg-edge hover:text-ink"
                        title={`Archive ${view.item.title}`}
                        aria-label={`Archive ${view.item.title}`}
                      >
                        <XIcon />
                      </button>
                    </div>
                  ))}
                  <button
                    type="button"
                    onClick={() => setCreateOpen(true)}
                    className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-panel-2 hover:text-ink"
                    title="New work item"
                    aria-label="New work item"
                  >
                    <PlusIcon />
                  </button>
                </div>
                {activeView && (
                  <div className="flex min-w-0 items-center gap-2">
                    <WorkItemChips projectPath={projectPath} view={activeView} />
                    <span className={`shrink-0 rounded px-1.5 py-0.5 text-xs font-medium ${modeStyle.badge}`}>
                      {currentStage.label}
                    </span>
                    <Badge tone="dim">idle</Badge>
                  </div>
                )}
              </div>
            </div>
            {shellMenu && contextView && (
              <div
                className="fixed z-50 min-w-44 rounded-md border border-edge bg-panel p-1 shadow-2xl"
                style={{ left: shellMenu.x, top: shellMenu.y }}
                onClick={(e) => e.stopPropagation()}
              >
                <button
                  type="button"
                  onClick={() => {
                    openRename(contextView);
                    setShellMenu(null);
                  }}
                  className="block w-full rounded px-3 py-1.5 text-left text-sm text-ink-dim hover:bg-panel-2 hover:text-ink"
                >
                  Rename work item
                </button>
                <button
                  type="button"
                  onClick={() => {
                    copyShellTranscript(contextView);
                    setShellMenu(null);
                  }}
                  className="block w-full rounded px-3 py-1.5 text-left text-sm text-ink-dim hover:bg-panel-2 hover:text-ink"
                >
                  Copy transcript
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setArchiveTarget(contextView);
                    setShellMenu(null);
                  }}
                  className="block w-full rounded px-3 py-1.5 text-left text-sm text-bad hover:bg-bad/10 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  Archive work item
                </button>
              </div>
            )}
            <Dialog open={Boolean(viewBeingRenamed)} onClose={closeRename} title="Rename work item">
              <form
                className="space-y-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  commitRename();
                }}
              >
                <TextInput
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  autoFocus
                  disabled={renameItem.isPending}
                />
                {renameItem.isError && <ErrorNote error={renameItem.error} />}
                <div className="flex justify-end gap-2">
                  <Button type="button" onClick={closeRename} disabled={renameItem.isPending}>
                    Cancel
                  </Button>
                  <Button
                    type="submit"
                    variant="primary"
                    busy={renameItem.isPending}
                    disabled={!renameValue.trim()}
                  >
                    Rename
                  </Button>
                </div>
              </form>
            </Dialog>
            <CreateWorkItemDialog
              projectPath={projectPath}
              open={createOpen}
              onClose={() => setCreateOpen(false)}
              onCreated={(view) => selectItem(view.item.id)}
            />
            <ArchiveWorkItemDialog
              projectPath={projectPath}
              target={archiveTarget}
              onClose={() => setArchiveTarget(null)}
              onArchived={handleArchived}
            />

            <div className="min-h-0 flex-1 overflow-y-auto p-5">
              <div className="mx-auto max-w-3xl space-y-4">
                {activeView ? (
                  <div className="rounded-lg border border-edge bg-panel p-4">
                    <div className="mb-2 flex items-center gap-2">
                      <Badge tone="accent">system</Badge>
                      <span className="text-xs text-ink-dim">spec-driven flow</span>
                    </div>
                    <p className="text-sm leading-relaxed text-ink-dim">
                      This is the project agent shell. Use the mode controls by the composer to run
                      the next turn as general chat or as a Research, Plan, Implement, Validate, or
                      Ship step.
                    </p>
                  </div>
                ) : (
                  <div className="rounded-lg border border-edge bg-panel p-6 text-center">
                    <p className="text-base font-medium text-ink">Create your first work item</p>
                    <p className="mx-auto mt-1 max-w-md text-sm leading-relaxed text-ink-dim">
                      Each work item gets its own branch and worktree, so agents can research,
                      plan, and implement without touching your checkout.
                    </p>
                    <Button
                      variant="primary"
                      className="mt-4"
                      onClick={() => setCreateOpen(true)}
                    >
                      <PlusIcon />
                      New work item
                    </Button>
                  </div>
                )}
              </div>
            </div>

            {activeView && (
              <div className="border-t border-edge bg-panel p-4">
                <div className="mx-auto max-w-3xl">
                  <textarea
                    value={draft}
                    onChange={(e) => updateDraft(e.target.value)}
                    rows={3}
                    placeholder={
                      stage === "general"
                        ? "Ask the project agent…"
                        : `Ask the ${currentStage.label.toLowerCase()} agent…`
                    }
                    className={`selectable w-full resize-none rounded-md border bg-panel-2 px-3 py-2 text-sm text-ink placeholder:text-ink-dim/70 focus:outline-none ${modeStyle.composer}`}
                  />
                  <div className="mt-2 flex items-center justify-between gap-3">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="shrink-0 text-xs text-ink-dim">Mode</span>
                      <div className="flex flex-wrap gap-1">
                        {STAGES.map((s) => (
                          <button
                            key={s.key}
                            type="button"
                            onClick={() => changeStage(s.key)}
                            className={`inline-flex h-8 w-8 items-center justify-center rounded-md border ${
                              s.key === stage
                                ? MODE_STYLES[s.key].activeButton
                                : "border-edge bg-panel-2 text-ink-dim hover:bg-edge/60 hover:text-ink"
                            }`}
                            title={`${s.label} mode`}
                            aria-label={`${s.label} mode`}
                          >
                            <ModeIcon mode={s.key} />
                          </button>
                        ))}
                      </div>
                      <span className={`rounded px-1.5 py-0.5 text-xs font-medium ${modeStyle.badge}`}>
                        {currentStage.label}
                      </span>
                    </div>
                    <Button variant="primary" disabled={!draft.trim()}>
                      Start
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
          workItemId={activeView?.item.id ?? null}
          collapsed={diffCollapsed}
          onToggleCollapsed={() => setDiffCollapsed((value) => !value)}
        />
      </aside>
    </div>
  );
}
