import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useEffect, useState, type PointerEvent as ReactPointerEvent } from "react";

import { Badge, Button, Dialog, ErrorNote, TextInput, Spinner } from "../components/ui";
import { CreateWorkstreamDialog } from "../features/workstreams/CreateWorkstreamDialog";
import { DiffSidebar } from "../features/workstreams/DiffSidebar";
import { WorkstreamChips } from "../features/workstreams/WorkstreamChips";
import type { WorkstreamView } from "../lib/bindings";
import { useProjectDetail, useSetWorkstreamStage, useWorkstreams } from "../lib/queries";

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
function stageOf(view: WorkstreamView): StageKey {
  return (STAGE_KEYS.has(view.item.stage) ? view.item.stage : "general") as StageKey;
}

/** A chat tab inside ONE workstream. Each workstream owns its own set;
 *  local state until phase 3 turns shells into persisted agent sessions. */
type ShellTab = {
  id: number;
  title: string;
  mode: StageKey;
  draft: string;
};

const defaultShells = (): ShellTab[] => [{ id: 1, title: "Agent", mode: "general", draft: "" }];

type ShellMenu = {
  shellId: number;
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
  const workstreams = useWorkstreams(projectPath);
  const setStage = useSetWorkstreamStage(projectPath);

  const search = useSearch({ from: "/projects/$projectPath" });
  const navigate = useNavigate({ from: "/projects/$projectPath" });

  const storageKey = `hyprlayer:last-workstream:${projectPath}`;
  // The URL (?item=) is the source of truth — it's what the sidebar tree
  // highlights and links to; localStorage only seeds the first visit.
  const [activeItemId, setActiveItemId] = useState<string | null>(
    () => search.item ?? localStorage.getItem(storageKey),
  );

  useEffect(() => {
    if (search.item) {
      setActiveItemId(search.item);
      localStorage.setItem(storageKey, search.item);
    }
  }, [search.item, storageKey]);

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
  // Shell tabs are PER WORKSTREAM (1 workstream -> N tabs), keyed by
  // workstream id so switching workstreams never mixes tabs.
  const [shellsByWs, setShellsByWs] = useState<Record<string, ShellTab[]>>({});
  const [activeShellByWs, setActiveShellByWs] = useState<Record<string, number>>({});
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [renameShellId, setRenameShellId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [diffCollapsed, setDiffCollapsed] = useState(false);
  const [diffWidth, setDiffWidth] = useState(360);

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

  if (detail.isLoading || workstreams.isLoading) {
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
  if (workstreams.isError) {
    return (
      <div className="p-8">
        <ErrorNote error={workstreams.error} />
      </div>
    );
  }

  const project = detail.data!.project;
  const views = (workstreams.data ?? []).filter((view) => view.item.status === "active");
  const activeView = views.find((view) => view.item.id === activeItemId) ?? views[0] ?? null;
  const wsId = activeView?.item.id ?? null;

  // The active workstream's own tabs (1 workstream -> N shells).
  const shells = wsId ? (shellsByWs[wsId] ?? defaultShells()) : [];
  const activeShell =
    shells.find((s) => s.id === (wsId ? activeShellByWs[wsId] : undefined)) ?? shells[0] ?? null;
  const contextShell = shellMenu ? shells.find((s) => s.id === shellMenu.shellId) : null;
  const shellBeingRenamed = renameShellId
    ? shells.find((s) => s.id === renameShellId)
    : null;

  const wsStage: StageKey = activeView ? stageOf(activeView) : "general";
  const mode: StageKey = activeShell?.mode ?? "general";
  const currentMode = STAGES.find((s) => s.key === mode)!;
  const modeStyle = MODE_STYLES[mode];
  const draft = activeShell?.draft ?? "";

  const updateShells = (updater: (current: ShellTab[]) => ShellTab[]) => {
    if (!wsId) return;
    setShellsByWs((all) => ({ ...all, [wsId]: updater(all[wsId] ?? defaultShells()) }));
  };
  const patchShell = (shellId: number, patch: Partial<Pick<ShellTab, "title" | "mode" | "draft">>) => {
    updateShells((current) =>
      current.map((s) => (s.id === shellId ? { ...s, ...patch } : s)),
    );
  };
  const selectShell = (shellId: number) => {
    if (!wsId) return;
    setActiveShellByWs((all) => ({ ...all, [wsId]: shellId }));
  };
  const addShell = () => {
    if (!wsId) return;
    setShellsByWs((all) => {
      const current = all[wsId] ?? defaultShells();
      const nextId = Math.max(0, ...current.map((s) => s.id)) + 1;
      setActiveShellByWs((sel) => ({ ...sel, [wsId]: nextId }));
      return {
        ...all,
        [wsId]: [...current, { id: nextId, title: `Agent ${nextId}`, mode: "general", draft: "" }],
      };
    });
  };
  const closeShell = (shellId: number) => {
    if (!wsId) return;
    setShellsByWs((all) => {
      const current = all[wsId] ?? defaultShells();
      if (current.length === 1) return all;
      const index = current.findIndex((s) => s.id === shellId);
      const next = current.filter((s) => s.id !== shellId);
      if ((activeShellByWs[wsId] ?? current[0]?.id) === shellId) {
        const fallback = next[Math.max(0, index - 1)] ?? next[0];
        setActiveShellByWs((sel) => ({ ...sel, [wsId]: fallback.id }));
      }
      return { ...all, [wsId]: next };
    });
  };

  const selectItem = (id: string | null) => {
    setActiveItemId(id);
    if (id === null) {
      localStorage.removeItem(storageKey);
    } else {
      localStorage.setItem(storageKey, id);
    }
    // Keep the URL in sync so the sidebar highlight follows.
    void navigate({
      search: (prev) => ({ ...prev, item: id ?? undefined }),
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
    const modeLabel = STAGES.find((s) => s.key === shell.mode)?.label ?? shell.mode;
    const transcript = [
      `# ${activeView?.item.title ?? project.mappedName} — ${shell.title}`,
      `Project: ${project.mappedName}`,
      `Branch: ${activeView?.item.branch ?? "—"}`,
      `Mode: ${modeLabel}`,
      "",
      "Transcript is not connected yet. Current draft:",
      shell.draft || "(empty)",
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
                          <span className={`rounded p-0.5 ${MODE_STYLES[shell.mode].badge}`}>
                            <ModeIcon mode={shell.mode} />
                          </span>
                          {shell.title}
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
                    <button
                      type="button"
                      onClick={addShell}
                      className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-panel-2 hover:text-ink"
                      title="New tab"
                      aria-label="New tab"
                    >
                      <PlusIcon />
                    </button>
                  )}
                </div>
                {activeView && (
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="truncate text-sm font-medium text-ink">
                      {activeView.item.title}
                    </span>
                    <WorkstreamChips projectPath={projectPath} view={activeView} />
                    <span
                      className={`shrink-0 rounded px-1.5 py-0.5 text-xs font-medium ${MODE_STYLES[wsStage].badge}`}
                    >
                      {STAGES.find((s) => s.key === wsStage)!.label}
                    </span>
                    <Badge tone="dim">idle</Badge>
                  </div>
                )}
              </div>
            </div>
            {shellMenu && contextShell && (
              <div
                className="fixed z-50 min-w-44 rounded-md border border-edge bg-panel p-1 shadow-2xl"
                style={{ left: shellMenu.x, top: shellMenu.y }}
                onClick={(e) => e.stopPropagation()}
              >
                <button
                  type="button"
                  onClick={() => {
                    openRenameShell(contextShell);
                    setShellMenu(null);
                  }}
                  className="block w-full rounded px-3 py-1.5 text-left text-sm text-ink-dim hover:bg-panel-2 hover:text-ink"
                >
                  Rename tab
                </button>
                <button
                  type="button"
                  onClick={() => {
                    copyShellTranscript(contextShell);
                    setShellMenu(null);
                  }}
                  className="block w-full rounded px-3 py-1.5 text-left text-sm text-ink-dim hover:bg-panel-2 hover:text-ink"
                >
                  Copy transcript
                </button>
                <button
                  type="button"
                  disabled={shells.length === 1}
                  onClick={() => {
                    closeShell(contextShell.id);
                    setShellMenu(null);
                  }}
                  className="block w-full rounded px-3 py-1.5 text-left text-sm text-ink-dim hover:bg-panel-2 hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
                >
                  Close tab
                </button>
              </div>
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
                    <p className="text-base font-medium text-ink">Create your first workstream</p>
                    <p className="mx-auto mt-1 max-w-md text-sm leading-relaxed text-ink-dim">
                      Each workstream gets its own branch and worktree, so agents can research,
                      plan, and implement without touching your checkout.
                    </p>
                    <Button
                      variant="primary"
                      className="mt-4"
                      onClick={() => setCreateOpen(true)}
                    >
                      <PlusIcon />
                      New workstream
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
                      mode === "general"
                        ? "Ask the project agent…"
                        : `Ask the ${currentMode.label.toLowerCase()} agent…`
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
                            onClick={() => changeMode(s.key)}
                            className={`inline-flex h-8 w-8 items-center justify-center rounded-md border ${
                              s.key === mode
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
                        {currentMode.label}
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
          workstreamId={activeView?.item.id ?? null}
          collapsed={diffCollapsed}
          onToggleCollapsed={() => setDiffCollapsed((value) => !value)}
        />
      </aside>
    </div>
  );
}
