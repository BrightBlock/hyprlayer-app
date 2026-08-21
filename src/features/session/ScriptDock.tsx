import { useEffect, useRef, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { Popover } from "@/components/ui";
import {
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  DockBottomIcon,
  DockRightIcon,
  EraserIcon,
  GlobeIcon,
  PlayIcon,
  PlusIcon,
  StopIcon,
  TerminalIcon,
  WrenchIcon,
  WarningTriangleIcon,
  XIcon,
} from "@/components/icons";
import { commands, type ScriptKind, type ScriptStatus } from "@/lib/bindings";
import {
  forgetScriptOutput,
  useBootstrap,
  useScriptOutput,
  useScriptRuntime,
  useScripts,
  useShellName,
} from "@/lib/queries";
import { formatChord } from "@/lib/keybindings/chord";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { IS_MACOS } from "@/lib/platform";
import { formatRelativeTime } from "@/lib/format";
import { ScriptOutputView } from "./ScriptOutputView";
import { ScriptRunView } from "./ScriptRunView";
import { TerminalTab } from "./TerminalTab";
import {
  addDockTerminal,
  closeDock,
  closeDockTerminal,
  consumeRun,
  DOCK_HEIGHT_MAX,
  DOCK_HEIGHT_MIN,
  selectDockTab,
  setDockHeight,
  setDockTermId,
  toggleDockCollapsed,
  toggleDockPosition,
  useScriptsStore,
} from "./scriptsStore";
import { startEdgeDrag } from "@/lib/resize";
import { ScrollableTabStrip } from "@/components/ScrollableTabs";

const cx = (...parts: Array<string | false | null | undefined>): string =>
  parts.filter(Boolean).join(" ");

// Only Setup + Run get dock tabs. Archive runs at teardown when the dock is
// already gone (nothing to watch), and its commands are visible in Project
// Settings, so it has no dock tab.
const SCRIPTS: { kind: Exclude<ScriptKind, never>; name: string; trigger: string; icon: ReactNode }[] = [
  { kind: "setup", name: "Setup", trigger: "Runs once when this workstream is created.", icon: <WrenchIcon size={13} /> },
  { kind: "run", name: "Run", trigger: "Runs each time you press ▶. The previous process is stopped first.", icon: <PlayIcon size={12} /> },
];

/** A mounted run view. `exited` marks the Run whose process is gone but whose
 *  output we keep on screen, a frozen log, not a live terminal. */
type View = { kind: ScriptKind; gen: number; attachId: string | null; exited: boolean };

export function ScriptDock({
  workstreamId,
  projectPath,
  onEditScripts,
  orientation = "bottom",
}: {
  workstreamId: string;
  projectPath: string;
  onEditScripts: () => void;
  /** "bottom" sits under the agent view (dark, like the editor); "right" lives in
   *  the lighter sidebar, so its chrome/idle surfaces use the panel tone to match. */
  orientation?: "bottom" | "right";
}) {
  const dock = useScriptsStore((s) => s.dock[workstreamId]);
  const running = useScriptsStore((s) => s.running[workstreamId]);
  const pending = useScriptsStore((s) => !!s.pendingRun[workstreamId]);
  const dockPosition = useScriptsStore((s) => s.dockPosition);
  const dockHeight = useScriptsStore((s) => s.dockHeight);
  const dockMaximized = useScriptsStore((s) => s.dockMaximized);
  const runtime = useScriptRuntime(workstreamId);
  const scripts = useScripts(projectPath);
  // Label ad-hoc terminal tabs by the shell they actually spawn (zsh/pwsh/…).
  const shellName = useShellName().data ?? "shell";
  // The Run shortcut, rendered for the host OS (⌘R on macOS, Ctrl+R elsewhere),
  // honoring any user rebinding of scripts.run.
  const overrides = useBootstrap().data?.settings.keybindings?.overrides ?? {};
  const runKey = formatChord(effectiveChord(overrides, "scripts.run", IS_MACOS) ?? "", IS_MACOS);

  const tab = dock?.tab ?? "run";
  const terminals = dock?.terminals ?? [];
  const termIds = dock?.termIds ?? {};
  const isRunning = !!running;
  const port = running?.port ?? runtime.data?.port ?? null;
  // Setup's + Run's captured last-run output (fetched only while their tab
  // shows). Run's backs the read-only view after a Run exits, the output stays
  // put until the next ▶ instead of snapping back to the idle prompt.
  const setupOutput = useScriptOutput(workstreamId, "setup", tab === "setup");
  const runOutput = useScriptOutput(workstreamId, "run", tab === "run");

  const [view, setView] = useState<View | null>(null);
  const genRef = useRef(0);
  // Bumped by Clear; ScriptRunView wipes its screen when this changes.
  const [clearNonce, setClearNonce] = useState(0);

  const cmdsFor = (kind: ScriptKind): string => scripts.data?.[kind] ?? "";
  const statusFor = (kind: ScriptKind) => runtime.data?.statuses.find((s) => s.kind === kind);

  // Start a fresh run of `kind` in the body (▶ / Run X). No-op when that script
  // is unconfigured: ⌘R arrives here without the ▶ button's `canRun` guard, and
  // spawning would only bounce off the backend's "no script configured" error.
  // The dock still lands on the tab, whose empty state explains the nothing.
  const start = (kind: ScriptKind): void => {
    if (!cmdsFor(kind).trim()) return;
    setView({ kind, gen: ++genRef.current, attachId: null, exited: false });
    selectDockTab(workstreamId, kind);
  };

  // ⌘R / rail elsewhere asked to start the Run, consume the one-shot flag. Wait
  // for the project's scripts to load first: deciding against an unresolved query
  // reads every project as unconfigured and would swallow a legitimate ⌘R.
  useEffect(() => {
    if (pending && !scripts.isPending) {
      start("run");
      consumeRun(workstreamId);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending, workstreamId, scripts.isPending, scripts.data]);

  // Opened the dock while a Run was already alive → attach to it.
  const liveRunId = runtime.data?.runTerminalId ?? null;
  useEffect(() => {
    if (liveRunId && !view) {
      setView({ kind: "run", gen: ++genRef.current, attachId: liveRunId, exited: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveRunId]);

  const stop = (): void => void commands.scriptStop(workstreamId);

  // Clear this script's output. A live run keeps streaming into a wiped screen;
  // a finished one has nothing left but its log, so drop the view and fall back
  // to the idle prompt. The command forgets the captured last-run snapshot and
  // the live PTY's retained scrollback, so neither a re-attach nor the run's exit
  // can bring back what was cleared.
  const clearOutput = (kind: ScriptKind): void => {
    const shown = view?.kind === kind ? view : null;
    const liveId = shown && !shown.exited ? shown.attachId : null;
    forgetScriptOutput(workstreamId, kind);
    if (shown?.exited) setView(null);
    else if (shown) setClearNonce((n) => n + 1);
    void commands.scriptClearOutput(workstreamId, kind, liveId);
  };
  const openPort = (): void => {
    if (port) void openUrl(`http://localhost:${port}`);
  };
  const closeTerm = (term: number): void => {
    const id = closeDockTerminal(workstreamId, term);
    if (id) void commands.terminalClose(id);
  };

  const activeKind = typeof tab === "string" ? (tab as ScriptKind) : null;
  // The split button's primary action targets the active script (Setup tab →
  // Setup), defaulting to Run from a terminal tab.
  const mainKind: ScriptKind = activeKind === "setup" ? "setup" : "run";
  // Clear only shows on the script tabs (a shell clears itself), and only bites
  // when the body holds something: a mounted run view, or a captured last run.
  const activeOutput = activeKind === "setup" ? setupOutput.data : runOutput.data;
  const canClear = view?.kind === activeKind || activeOutput != null;
  // In the sidebar, blend with the panel tone; at the bottom, use the surface
  // tone (same token the xterm bg resolves to) so chrome and bodies theme with
  // the app instead of staying dark in light mode.
  const right = orientation === "right";
  const chromeBg = right ? "bg-panel" : "bg-surface";
  const idleBg = right ? "bg-panel" : "bg-surface";
  // Right-docked, the chevron collapses the body in place (header stays), like the
  // CI-checks section. Persisted in the store so it survives the dock's
  // unmount/remount on a sidebar toggle; only effective while right-docked.
  const collapsed = right && (dock?.collapsed ?? false);
  // ⌘⇧F maximizes the body to its full height; collapse (right-dock) still wins.
  const bodyHeight = dockMaximized ? DOCK_HEIGHT_MAX : dockHeight;

  return (
    <div
      className={cx("relative flex shrink-0 flex-col border-t border-edge", chromeBg)}
      style={{ height: collapsed ? undefined : bodyHeight }}
    >
      {/* Drag the top edge to resize the dock's height (not while collapsed). */}
      {!collapsed && (
        <div
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize scripts dock"
          onPointerDown={(e) =>
            startEdgeDrag(e, {
              startWidth: bodyHeight,
              min: DOCK_HEIGHT_MIN,
              max: DOCK_HEIGHT_MAX,
              direction: -1,
              axis: "y",
              onResize: setDockHeight,
            })
          }
          className="absolute -top-1 left-0 right-0 z-10 h-2 cursor-row-resize bg-transparent transition-colors hover:bg-panel-2"
        />
      )}
      {/* header, single row: the left chrome and right Run button stay pinned,
          and the tab strip scrolls (with edge chevrons) when it gets too narrow.
          Right-docked it matches the other sidebar section headers (`h-9`); the
          bottom dock keeps its taller toolbar. */}
      <div
        className={cx(
          "flex shrink-0 items-center gap-1 border-b border-edge px-2 py-1",
          right ? "h-9" : "h-[42px]",
        )}
      >
        <button
          type="button"
          // Right-docked: collapse the body in place (keep the header). Bottom:
          // minimize the whole dock to the rail.
          onClick={() => (right ? toggleDockCollapsed(workstreamId) : closeDock(workstreamId))}
          title={right ? (collapsed ? "Expand" : "Collapse") : "Minimize to sidebar"}
          aria-label={right ? "Toggle scripts dock" : "Minimize scripts dock"}
          className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-dim hover:bg-panel-2 hover:text-ink"
        >
          {right && collapsed ? <ChevronRightIcon size={14} /> : <ChevronDownIcon size={14} />}
        </button>
        <button
          type="button"
          onClick={toggleDockPosition}
          title={dockPosition === "bottom" ? "Dock to right sidebar" : "Dock to bottom"}
          aria-label="Toggle dock position"
          className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-dim hover:bg-panel-2 hover:text-ink"
        >
          {dockPosition === "bottom" ? <DockRightIcon size={13} /> : <DockBottomIcon size={13} />}
        </button>

        <ScrollableTabStrip
          grow
          trailing={
            <button
              type="button"
              onClick={() => addDockTerminal(workstreamId)}
              title="New terminal"
              aria-label="New terminal"
              className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-dim hover:bg-panel-2 hover:text-ink"
            >
              <PlusIcon size={13} />
            </button>
          }
        >
          {SCRIPTS.map((s) => {
            const st = statusFor(s.kind);
            const active = tab === s.kind;
            return (
              <DockTab key={s.kind} active={active} onClick={() => selectDockTab(workstreamId, s.kind)} icon={s.icon} label={s.name}>
                {s.kind === "run" && isRunning && <Dot pulse />}
                {s.kind !== "run" && st?.phase === "passed" && <CheckIcon size={11} className="text-ok" />}
                {s.kind !== "run" && st?.phase === "failed" && <WarningTriangleIcon size={11} className="text-bad" />}
              </DockTab>
            );
          })}
          {terminals.length > 0 && <span className="mx-1 h-4 w-px shrink-0 bg-edge" />}
          {terminals.map((t) => (
            <DockTab
              key={`term-${t}`}
              active={typeof tab === "object" && tab.term === t}
              onClick={() => selectDockTab(workstreamId, { term: t })}
              icon={<TerminalIcon size={13} />}
              label={shellName}
            >
              <span
                role="button"
                tabIndex={0}
                aria-label="Close terminal"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTerm(t);
                }}
                className="ml-0.5 grid h-4 w-4 place-items-center rounded text-ink-dim/70 hover:bg-panel-2 hover:text-ink"
              >
                <XIcon size={10} />
              </span>
            </DockTab>
          ))}
        </ScrollableTabStrip>

        {/* right-side controls: Open :PORT while running, plus a split Run/Stop
            button whose chevron opens "Configure scripts". Pinned, never moves. */}
        <div className="flex shrink-0 items-center gap-2">
          {isRunning && port && (
            <button
              type="button"
              onClick={openPort}
              className="inline-flex h-7 items-center gap-1.5 rounded-md border border-edge bg-panel-2 px-2.5 text-xs text-ink hover:bg-edge/60"
            >
              <GlobeIcon size={13} className="text-ink-dim" /> Open{" "}
              <span className="font-mono text-brand">:{port}</span>
            </button>
          )}
          {activeKind && (
            <button
              type="button"
              onClick={() => clearOutput(activeKind)}
              disabled={!canClear}
              title="Clear output"
              aria-label="Clear output"
              className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-dim hover:bg-panel-2 hover:text-ink disabled:pointer-events-none disabled:opacity-40"
            >
              <EraserIcon size={13} />
            </button>
          )}
          <RunSplit
            running={isRunning}
            label={mainKind === "run" ? "Run" : "Run setup"}
            canRun={!!cmdsFor(mainKind).trim()}
            compact={right}
            runKey={runKey}
            onRun={() => start(mainKind)}
            onStop={stop}
            onConfigure={onEditScripts}
          />
        </div>
      </div>

      {/* body, hidden when collapsed (right-docked); the header stays visible. */}
      {!collapsed &&
        (typeof tab === "object" ? (
        <TerminalTab
          key={`${workstreamId}:term:${tab.term}`}
          workstreamId={workstreamId}
          shellId={tab.term}
          terminalId={termIds[tab.term]}
          autoFocus
          onTerminalId={(id) => setDockTermId(workstreamId, tab.term, id)}
        />
      ) : view && view.kind === tab ? (
        <ScriptRunView
          key={`${workstreamId}:${tab}:${view.gen}`}
          workstreamId={workstreamId}
          kind={tab}
          attachId={view.attachId}
          gen={view.gen}
          clearNonce={clearNonce}
          onStarted={(id) => setView((v) => (v ? { ...v, attachId: id } : v))}
          // A finished Run stays on screen (its output is the point) until the
          // next ▶ remounts it; Setup hands off to its status+output body below.
          // Flagged `exited` so Clear knows the log is frozen, not streaming.
          onExit={() => setView((v) => (!v || v.kind === "setup" ? null : { ...v, exited: true }))}
          // Remounted onto an exited Run (tab switch / collapse): drop the view so
          // the captured output shows, instead of the empty terminal spawning anew.
          onDead={() => setView(null)}
        />
      ) : tab === "run" ? (
        runOutput.data != null ? (
          <ScriptOutputView output={runOutput.data} />
        ) : (
          <RunIdle
            empty={!cmdsFor("run").trim()}
            onRun={() => start("run")}
            onEdit={onEditScripts}
            bg={idleBg}
            runKey={runKey}
          />
        )
      ) : (
        <SetupBody
          status={statusFor("setup")}
          output={setupOutput.data ?? null}
          loading={setupOutput.isLoading}
          onEdit={onEditScripts}
          bg={idleBg}
        />
      ))}
    </div>
  );
}

function DockTab({
  active,
  onClick,
  icon,
  label,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon: ReactNode;
  label: string;
  children?: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cx(
        "flex h-[30px] shrink-0 items-center gap-1.5 rounded-lg border px-3 text-xs whitespace-nowrap",
        active
          ? "border-edge bg-panel-2 font-semibold text-ink"
          : "border-transparent font-medium text-ink-dim hover:text-ink",
      )}
    >
      <span className={active ? "text-accent" : "text-ink-dim"}>{icon}</span>
      {label}
      {children}
    </button>
  );
}

function Dot({ pulse }: { pulse?: boolean }) {
  return (
    <span
      className="inline-block h-1.5 w-1.5 rounded-full bg-working"
      style={pulse ? { boxShadow: "0 0 0 3px color-mix(in srgb, var(--color-working) 25%, transparent)" } : undefined}
    />
  );
}

function Kbd({ children }: { children: ReactNode }) {
  return (
    <span className="rounded border border-edge bg-surface px-1 py-px font-mono text-2xs text-ink-dim">
      {children}
    </span>
  );
}

/** Combined Run/Stop button + a chevron that drops a "Configure" menu. The menu
 *  is anchored to the button group, opens downward, spans the group's exact
 *  width, and shares its panel/border so it reads as an extension of the box. */
function RunSplit({
  running,
  label,
  canRun,
  compact,
  runKey,
  onRun,
  onStop,
  onConfigure,
}: {
  running: boolean;
  label: string;
  canRun: boolean;
  /** Icon-only main button (no label/shortcut), used in the narrow sidebar so
   *  the button keeps a fixed, small width and never reflows during resize. */
  compact: boolean;
  /** Run shortcut formatted for the host OS (e.g. "⌘R" / "Ctrl+R"). */
  runKey: string;
  onRun: () => void;
  onStop: () => void;
  onConfigure: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  return (
    <div ref={ref} className="inline-flex items-stretch">
      <button
        type="button"
        onClick={running ? onStop : onRun}
        disabled={!running && !canRun}
        title={
          !running && !canRun
            ? "No script configured. Use Configure to add one"
            : compact
              ? `${running ? "Stop" : label}${runKey ? ` (${runKey})` : ""}`
              : undefined
        }
        className={cx(
          "inline-flex h-7 items-center gap-1.5 rounded-l-md border border-edge bg-panel-2 text-xs font-medium hover:bg-edge/60 disabled:cursor-not-allowed disabled:opacity-50",
          compact ? "px-2" : "px-3",
          running ? "text-bad" : "text-ink",
          open && "rounded-bl-none",
        )}
      >
        {running ? <StopIcon size={11} /> : <PlayIcon size={11} className="text-ink-dim" />}
        {!compact && (
          <>
            {running ? "Stop" : label} {runKey && <Kbd>{runKey}</Kbd>}
          </>
        )}
      </button>
      <button
        type="button"
        aria-label="Script options"
        title="Script options"
        onClick={() => setOpen((o) => !o)}
        className={cx(
          "inline-flex h-7 items-center rounded-r-md border border-l-0 border-edge bg-panel-2 px-1.5 text-ink-dim hover:bg-edge/60",
          open && "rounded-br-none",
        )}
      >
        <ChevronDownIcon size={12} />
      </button>
      {/* The Popover's own panel chrome is neutralized (rounded-none/border-0/
          bg-transparent/p-0/shadow-none) and the visible box lives on the div
          below instead — Popover's base `rounded-md` and this menu's flat-top
          `rounded-b-md` are different tailwind-merge groups, so merging them
          onto one element would leave both in the compiled output with the
          winner decided by Tailwind's own generation order, the exact
          landmine `cx()` exists to avoid. */}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={ref}
        align="end"
        sideOffset={0}
        matchAnchorWidth
        className="rounded-none border-0 bg-transparent p-0 shadow-none"
      >
        <div className="w-max min-w-full overflow-hidden rounded-b-md border border-t-0 border-edge bg-panel-2">
          <button
            type="button"
            onClick={() => {
              setOpen(false);
              onConfigure();
            }}
            className="block w-full px-3 py-1.5 text-left text-[12.5px] text-ink hover:bg-edge/60"
          >
            Configure
          </button>
        </div>
      </Popover>
    </div>
  );
}

function RunIdle({
  empty,
  onRun,
  onEdit,
  bg,
  runKey,
}: {
  empty: boolean;
  onRun: () => void;
  onEdit: () => void;
  bg: string;
  runKey: string;
}) {
  return (
    <div className={cx("flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center", bg)}>
      {empty ? (
        <>
          <p className="text-sm text-ink-dim">No Run script configured for this project.</p>
          <button
            type="button"
            onClick={onEdit}
            className="inline-flex items-center gap-1.5 rounded-md border border-edge bg-panel-2 px-3 py-1.5 text-xs text-ink hover:bg-edge/60"
          >
            Edit scripts <span aria-hidden>↗</span>
          </button>
        </>
      ) : (
        <>
          <TerminalIcon size={46} strokeWidth={1.4} className="text-ink-dim/40" />
          <button
            type="button"
            onClick={onRun}
            className="inline-flex items-center gap-2 rounded-md border border-edge bg-panel-2 px-3.5 py-2 text-sm font-medium text-ink hover:bg-edge/60"
          >
            <PlayIcon size={12} className="text-ink-dim" /> Start Run {runKey && <Kbd>{runKey}</Kbd>}
          </button>
          <p className="text-xs text-ink-dim/70">Test your changes here.</p>
        </>
      )}
    </div>
  );
}

/** Setup tab: a status line + the last run's captured output (more useful than
 *  echoing the command, which lives in Project Settings). */
function SetupBody({
  status,
  output,
  loading,
  onEdit,
  bg,
}: {
  status: ScriptStatus | undefined;
  output: string | null;
  loading: boolean;
  onEdit: () => void;
  bg: string;
}) {
  const statusText = !status
    ? "Runs once when this workstream is created."
    : status.phase === "running"
      ? "Running…"
      : `${
          status.phase === "passed"
            ? "Passed"
            : `Failed${status.exitCode != null ? ` (exit ${status.exitCode})` : ""}`
        }${status.at != null ? ` · ran ${formatRelativeTime(status.at)}` : ""}`;
  return (
    <div className={cx("flex min-h-0 flex-1 flex-col", bg)}>
      <div className="flex shrink-0 items-center gap-1.5 border-b border-edge px-4 py-2 text-xs text-ink-dim">
        {status?.phase === "passed" && <CheckIcon size={11} className="text-ok" />}
        {status?.phase === "failed" && <WarningTriangleIcon size={11} className="text-bad" />}
        <span>{statusText}</span>
        <button
          type="button"
          onClick={onEdit}
          className="ml-auto inline-flex h-6 items-center gap-1 rounded px-1.5 text-ink-dim hover:bg-panel-2 hover:text-ink"
        >
          Configure <span aria-hidden>↗</span>
        </button>
      </div>
      {output ? (
        <ScriptOutputView output={output} />
      ) : (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-sm text-ink-dim/70">
          {loading
            ? ""
            : "Setup hasn't run in this session yet. It runs automatically when a workstream is created."}
        </div>
      )}
    </div>
  );
}
