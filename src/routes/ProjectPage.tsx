import { useParams } from "@tanstack/react-router";
import { useEffect, useState, type PointerEvent as ReactPointerEvent } from "react";

import { Badge, Button, Dialog, ErrorNote, TextInput, Spinner } from "../components/ui";
import { useProjectDetail } from "../lib/queries";

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
type ShellTab = {
  id: number;
  title: string;
  mode: StageKey;
  draft: string;
};

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

function PanelRightIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path
        d="M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-13Z"
        stroke="currentColor"
        strokeWidth="1.8"
      />
      <path d="M14 4v16" stroke="currentColor" strokeWidth="1.8" />
    </svg>
  );
}

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
  const [shells, setShells] = useState<ShellTab[]>([
    { id: 1, title: "Agent", mode: "general", draft: "" },
  ]);
  const [activeShellId, setActiveShellId] = useState(1);
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
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

  if (detail.isLoading) {
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

  const project = detail.data!.project;
  const activeShell = shells.find((shell) => shell.id === activeShellId) ?? shells[0];
  const contextShell = shellMenu
    ? shells.find((shell) => shell.id === shellMenu.shellId)
    : null;
  const shellBeingRenamed = renameShellId
    ? shells.find((shell) => shell.id === renameShellId)
    : null;
  const stage = activeShell.mode;
  const currentStage = STAGES.find((s) => s.key === stage)!;
  const modeStyle = MODE_STYLES[stage];
  const updateActiveShell = (patch: Partial<Pick<ShellTab, "mode" | "draft">>) => {
    setShells((current) =>
      current.map((shell) => (shell.id === activeShell.id ? { ...shell, ...patch } : shell)),
    );
  };
  const addShell = () => {
    setShells((current) => {
      const nextId = Math.max(0, ...current.map((shell) => shell.id)) + 1;
      setActiveShellId(nextId);
      return [
        ...current,
        { id: nextId, title: `Agent ${nextId}`, mode: "general", draft: "" },
      ];
    });
  };
  const closeShell = (id: number) => {
    setShells((current) => {
      if (current.length === 1) return current;
      const index = current.findIndex((shell) => shell.id === id);
      const next = current.filter((shell) => shell.id !== id);
      if (id === activeShell.id) {
        const fallback = next[Math.max(0, index - 1)] ?? next[0];
        setActiveShellId(fallback.id);
      }
      return next;
    });
  };
  const openRenameShell = (shell: ShellTab) => {
    setRenameShellId(shell.id);
    setRenameValue(shell.title);
  };
  const commitRenameShell = () => {
    if (!shellBeingRenamed) return;
    const nextTitle = renameValue.trim();
    if (!nextTitle) return;
    setShells((current) =>
      current.map((item) =>
        item.id === shellBeingRenamed.id ? { ...item, title: nextTitle } : item,
      ),
    );
    setRenameShellId(null);
    setRenameValue("");
  };
  const copyShellTranscript = (shell: ShellTab) => {
    const mode = STAGES.find((s) => s.key === shell.mode)?.label ?? shell.mode;
    const transcript = [
      `# ${shell.title}`,
      `Project: ${project.mappedName}`,
      `Mode: ${mode}`,
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
                  {shells.map((shell) => (
                    <div
                      key={shell.id}
                      className={`inline-flex shrink-0 items-center rounded-md ${
                        shell.id === activeShell.id
                          ? "bg-panel-2 text-ink"
                          : "text-ink-dim hover:bg-panel-2 hover:text-ink"
                      }`}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        setActiveShellId(shell.id);
                        setShellMenu({ shellId: shell.id, x: e.clientX, y: e.clientY });
                      }}
                    >
                      <button
                        type="button"
                        onClick={() => setActiveShellId(shell.id)}
                        className="px-3 py-1.5 text-sm font-medium"
                      >
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
                  <button
                    type="button"
                    onClick={addShell}
                    className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-panel-2 hover:text-ink"
                    title="New agent shell"
                    aria-label="New agent shell"
                  >
                    <PlusIcon />
                  </button>
                </div>
                <div className="flex items-center gap-2">
                  <span className={`rounded px-1.5 py-0.5 text-xs font-medium ${modeStyle.badge}`}>
                    {currentStage.label}
                  </span>
                  <Badge tone="dim">idle</Badge>
                </div>
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
                  Rename chat
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
                  onClick={() => {
                    closeShell(contextShell.id);
                    setShellMenu(null);
                  }}
                  disabled={shells.length === 1}
                  className="block w-full rounded px-3 py-1.5 text-left text-sm text-bad hover:bg-bad/10 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  Close tab
                </button>
              </div>
            )}
            <Dialog
              open={Boolean(shellBeingRenamed)}
              onClose={() => {
                setRenameShellId(null);
                setRenameValue("");
              }}
              title="Rename chat"
            >
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
                  <Button
                    type="button"
                    onClick={() => {
                      setRenameShellId(null);
                      setRenameValue("");
                    }}
                  >
                    Cancel
                  </Button>
                  <Button type="submit" variant="primary" disabled={!renameValue.trim()}>
                    Rename
                  </Button>
                </div>
              </form>
            </Dialog>

            <div className="min-h-0 flex-1 overflow-y-auto p-5">
              <div className="mx-auto max-w-3xl space-y-4">
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
              </div>
            </div>

            <div className="border-t border-edge bg-panel p-4">
              <div className="mx-auto max-w-3xl">
                <textarea
                  value={activeShell.draft}
                  onChange={(e) => updateActiveShell({ draft: e.target.value })}
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
                          onClick={() => updateActiveShell({ mode: s.key })}
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
                  <Button variant="primary" disabled={!activeShell.draft.trim()}>
                    Start
                  </Button>
                </div>
              </div>
            </div>
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
        <div className="flex h-[61px] items-center border-b border-edge px-3">
          <div className={`flex w-full items-center gap-3 ${diffCollapsed ? "justify-center" : ""}`}>
            <button
              type="button"
              onClick={() => setDiffCollapsed((value) => !value)}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-panel-2 hover:text-ink"
              title={diffCollapsed ? "Expand diff sidebar" : "Collapse diff sidebar"}
              aria-label={diffCollapsed ? "Expand diff sidebar" : "Collapse diff sidebar"}
            >
              <PanelRightIcon />
            </button>
            {!diffCollapsed && (
              <div className="flex min-w-0 flex-1 items-center justify-between gap-3">
                <h2 className="text-sm font-semibold">Diff</h2>
                <Badge tone="dim">0 files</Badge>
              </div>
            )}
          </div>
        </div>
        {diffCollapsed ? (
          <div className="flex justify-center p-3">
            <Badge tone="dim">0</Badge>
          </div>
        ) : (
          <div className="space-y-4 p-4">
            <div className="rounded-lg border border-edge bg-panel-2 p-3">
              <p className="text-sm font-medium">No worktree diff yet</p>
              <p className="mt-1 text-xs leading-relaxed text-ink-dim">
                Diffs will be computed from the project worktree when work items land.
              </p>
            </div>
          </div>
        )}
      </aside>
    </div>
  );
}
