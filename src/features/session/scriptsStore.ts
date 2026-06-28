import { create } from "zustand";

/** Which dock tab is selected: one of the three pinned lifecycle scripts, or an
 *  ad-hoc terminal addressed by its (dock-local) shell id. */
export type DockTab = "setup" | "run" | "archive" | { term: number };

type DockState = {
  /** Whether the bottom dock is expanded for this workstream. Default closed —
   *  the chat keeps full height until the user opens it (rail icon / ⌘R). */
  open: boolean;
  /** The selected tab. */
  tab: DockTab;
  /** Ad-hoc terminal shell ids opened in the dock (the `zsh` / `+` tabs). */
  terminals: number[];
  /** Monotonic counter for the next ad-hoc terminal's shell id. */
  nextTerm: number;
  /** Live backend PTY id per ad-hoc terminal shell id, so a collapsed dock
   *  re-attaches its terminals on reopen instead of orphaning them. */
  termIds: Record<number, string>;
  /** Right-docked: body collapsed in place (header-only). Persisted here — not in
   *  the component — so it survives the dock unmount/remount when the sidebar is
   *  toggled. Ignored while bottom-docked. */
  collapsed: boolean;
};

/** A workstream's live Run, surfaced to the rail (green dot) + dock (Open :PORT).
 *  `port` is the detected dev-server port, null until the output reveals it. */
type RunInfo = { port: number | null };

type ScriptsState = {
  /** Per-workstream dock UI state. */
  dock: Record<string, DockState>;
  /** Workstreams with a live Run process (drives the rail's green indicator). */
  running: Record<string, RunInfo>;
  /** One-shot "start the Run now" flag, set by ▶ / ⌘R and consumed by the dock
   *  (which owns the run view). Lets ⌘R start a run even when the dock is closed. */
  pendingRun: Record<string, boolean>;
  /** The workstream currently on screen, published by ProjectPage — the rail's
   *  dock toggle acts on it. Null when no workstream is open. */
  activeWorkstream: string | null;
  /** Where the dock lives: "bottom" (under the agent view) or "right" (stacked
   *  in the right sidebar, below Changes / above CI checks). Global preference. */
  dockPosition: DockPosition;
  /** Dock height (px), drag-resizable from its top edge. Global preference. */
  dockHeight: number;
};

/** Drag bounds for the dock's height. */
export const DOCK_HEIGHT_MIN = 140;
export const DOCK_HEIGHT_MAX = 760;

export type DockPosition = "bottom" | "right";

const DEFAULT_DOCK: DockState = {
  open: false,
  tab: "run",
  // Seed one ad-hoc terminal so users discover that terminals live in the dock —
  // it shows next to Setup/Run; its PTY spawns lazily when the tab is selected.
  terminals: [1],
  nextTerm: 2,
  termIds: {},
  collapsed: false,
};

export const useScriptsStore = create<ScriptsState>(() => ({
  dock: {},
  running: {},
  pendingRun: {},
  activeWorkstream: null,
  dockPosition: "bottom",
  dockHeight: 320,
}));

/** Set the dock height (px); caller clamps to [DOCK_HEIGHT_MIN, DOCK_HEIGHT_MAX]. */
export const setDockHeight = (height: number): void =>
  useScriptsStore.setState({ dockHeight: height });

/** Publish which workstream is on screen (ProjectPage owns this). */
export const setActiveWorkstream = (wsId: string | null): void =>
  useScriptsStore.setState({ activeWorkstream: wsId });

/** Flip the dock between the bottom and the right sidebar. */
export const toggleDockPosition = (): void =>
  useScriptsStore.setState((s) => ({
    dockPosition: s.dockPosition === "bottom" ? "right" : "bottom",
  }));

/** Read a workstream's dock state (defaulted), outside React. */
export function dockStateFor(wsId: string): DockState {
  return useScriptsStore.getState().dock[wsId] ?? DEFAULT_DOCK;
}

function patchDock(wsId: string, patch: Partial<DockState>): void {
  useScriptsStore.setState((s) => ({
    dock: { ...s.dock, [wsId]: { ...(s.dock[wsId] ?? DEFAULT_DOCK), ...patch } },
  }));
}

export const openDock = (wsId: string): void => patchDock(wsId, { open: true });
export const closeDock = (wsId: string): void => patchDock(wsId, { open: false });
export const toggleDock = (wsId: string): void =>
  patchDock(wsId, { open: !dockStateFor(wsId).open });

/** Select a tab and ensure the dock is open. */
export const selectDockTab = (wsId: string, tab: DockTab): void =>
  patchDock(wsId, { tab, open: true });

/** Collapse/expand the right-docked dock body in place (header stays). */
export const toggleDockCollapsed = (wsId: string): void =>
  patchDock(wsId, { collapsed: !dockStateFor(wsId).collapsed });

/** Open a new ad-hoc terminal in the dock and focus it. Returns its shell id. */
export function addDockTerminal(wsId: string): number {
  const cur = dockStateFor(wsId);
  const id = cur.nextTerm;
  patchDock(wsId, {
    open: true,
    terminals: [...cur.terminals, id],
    nextTerm: id + 1,
    tab: { term: id },
  });
  return id;
}

/** Record the live PTY id for an ad-hoc terminal (so it re-attaches on reopen). */
export function setDockTermId(wsId: string, term: number, termId: string): void {
  const cur = dockStateFor(wsId);
  patchDock(wsId, { termIds: { ...cur.termIds, [term]: termId } });
}

/** Close an ad-hoc terminal tab; if it was active, fall back to the Run tab.
 *  Returns the backend PTY id (if known) so the caller can close it. */
export function closeDockTerminal(wsId: string, term: number): string | undefined {
  const cur = dockStateFor(wsId);
  const terminals = cur.terminals.filter((t) => t !== term);
  const termIds = { ...cur.termIds };
  const termId = termIds[term];
  delete termIds[term];
  const wasActive = typeof cur.tab === "object" && cur.tab.term === term;
  patchDock(wsId, { terminals, termIds, tab: wasActive ? "run" : cur.tab });
  return termId;
}

/** Mark/clear a workstream's live Run. Passing `null` clears it. */
export function setRunning(wsId: string, info: RunInfo | null): void {
  useScriptsStore.setState((s) => {
    const running = { ...s.running };
    if (info) running[wsId] = info;
    else delete running[wsId];
    return { running };
  });
}

/** Ask the dock to start a Run (opens it, selects the Run tab, flags a start).
 *  The dock consumes the flag with {@link consumeRun}. */
export function requestRun(wsId: string): void {
  patchDock(wsId, { open: true, tab: "run" });
  useScriptsStore.setState((s) => ({ pendingRun: { ...s.pendingRun, [wsId]: true } }));
}

/** Clear the pending-run flag once the dock has acted on it. */
export function consumeRun(wsId: string): void {
  useScriptsStore.setState((s) => {
    if (!s.pendingRun[wsId]) return s;
    const pendingRun = { ...s.pendingRun };
    delete pendingRun[wsId];
    return { pendingRun };
  });
}

/** Seed the live-Run set from the backend (app start), replacing what's there. */
export function hydrateRunning(ids: string[]): void {
  useScriptsStore.setState((s) => {
    const running: Record<string, RunInfo> = {};
    for (const id of ids) running[id] = s.running[id] ?? { port: null };
    return { running };
  });
}
