import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

import { events } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";

export type DockTab = "setup" | "run" | "archive" | { term: number };

type DockState = {
  open: boolean;
  tab: DockTab;
  terminals: number[];
  nextTerm: number;
  termIds: Record<number, string>;
  collapsed: boolean;
};

type RunInfo = { port: number | null };

type ScriptsState = {
  dock: Record<string, DockState>;
  running: Record<string, RunInfo>;
  pendingRun: Record<string, boolean>;
  activeWorkstream: string | null;
  dockPosition: DockPosition;
  dockHeight: number;
  dockMaximized: boolean;
};

export const DOCK_HEIGHT_MIN = 140;
export const DOCK_HEIGHT_MAX = 760;

type DockPosition = "bottom" | "right";

const DEFAULT_DOCK: DockState = {
  open: false,
  tab: { term: 1 },
  terminals: [1],
  nextTerm: 2,
  termIds: {},
  collapsed: false,
};

type PersistedScriptsState = {
  dockPosition: DockPosition;
  dockHeight: number;
  dock: Record<string, Partial<DockState>>;
};

export const useScriptsStore = create<ScriptsState>()(
  persist<ScriptsState, [], [], PersistedScriptsState>(
    () => ({
      dock: {},
      running: {},
      pendingRun: {},
      activeWorkstream: null,
      dockPosition: "bottom",
      dockHeight: 320,
      dockMaximized: false,
    }),
    {
      name: scopedKey("hyprlayer:scripts-dock"),
      version: 1,
      partialize: (s) => ({
        dockPosition: s.dockPosition,
        dockHeight: s.dockHeight,
        // Terminal ids persist so a relaunch reattaches to terminals the server
        // kept alive (ADR-0009); TerminalTab opens a fresh one when an id no
        // longer attaches.
        dock: Object.fromEntries(
          Object.entries(s.dock).map(([id, d]) => [
            id,
            {
              open: d.open,
              tab: d.tab,
              terminals: d.terminals,
              nextTerm: d.nextTerm,
              termIds: d.termIds,
              collapsed: d.collapsed,
            },
          ]),
        ),
      }),
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Partial<PersistedScriptsState>;
        const dock: Record<string, DockState> = {};
        for (const [id, d] of Object.entries(p.dock ?? {})) {
          dock[id] = { ...DEFAULT_DOCK, ...d };
        }
        return { ...current, ...p, dock };
      },
    },
  ),
);

export const setDockHeight = (height: number): void => {
  useScriptsStore.setState({ dockHeight: height, dockMaximized: false });
};

export const toggleDockMaximized = (wsId: string): void => {
  const next = !useScriptsStore.getState().dockMaximized;
  if (next) patchDock(wsId, { open: true, collapsed: false });
  useScriptsStore.setState({ dockMaximized: next });
};

export const setActiveWorkstream = (wsId: string | null): void => {
  useScriptsStore.setState({ activeWorkstream: wsId });
};

export const toggleDockPosition = (): void => {
  useScriptsStore.setState((s) => ({
    dockPosition: s.dockPosition === "bottom" ? "right" : "bottom",
  }));
};

export function dockStateFor(wsId: string): DockState {
  return useScriptsStore.getState().dock[wsId] ?? DEFAULT_DOCK;
}

function patchDock(wsId: string, patch: Partial<DockState>): void {
  useScriptsStore.setState((s) => ({
    dock: { ...s.dock, [wsId]: { ...(s.dock[wsId] ?? DEFAULT_DOCK), ...patch } },
  }));
}

export const closeDock = (wsId: string): void => patchDock(wsId, { open: false });
export const toggleDock = (wsId: string): void =>
  patchDock(wsId, { open: !dockStateFor(wsId).open });

export const selectDockTab = (wsId: string, tab: DockTab): void =>
  patchDock(wsId, { tab, open: true });

export const toggleDockCollapsed = (wsId: string): void =>
  patchDock(wsId, { collapsed: !dockStateFor(wsId).collapsed });

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

export function setDockTermId(wsId: string, term: number, termId: string): void {
  const cur = dockStateFor(wsId);
  patchDock(wsId, { termIds: { ...cur.termIds, [term]: termId } });
}

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

export function setRunning(wsId: string, info: RunInfo | null): void {
  useScriptsStore.setState((s) => {
    const running = { ...s.running };
    if (info) running[wsId] = info;
    else delete running[wsId];
    return { running };
  });
}

export function requestRun(wsId: string): void {
  patchDock(wsId, { open: true, tab: "run" });
  useScriptsStore.setState((s) => ({ pendingRun: { ...s.pendingRun, [wsId]: true } }));
}

export function consumeRun(wsId: string): void {
  useScriptsStore.setState((s) => {
    if (!s.pendingRun[wsId]) return s;
    const pendingRun = { ...s.pendingRun };
    delete pendingRun[wsId];
    return { pendingRun };
  });
}

export function hydrateRunning(ids: string[]): void {
  useScriptsStore.setState((s) => {
    const running: Record<string, RunInfo> = {};
    for (const id of ids) running[id] = s.running[id] ?? { port: null };
    return { running };
  });
}

export const scriptRunBridge: EventBridge = () => [
  events.scriptRunChanged.listen((event) => {
    const { workstreamId, kind, phase, port } = event.payload;
    if (kind !== "run") return;
    if (phase === "running") setRunning(workstreamId, { port: port ?? null });
    else setRunning(workstreamId, null);
  }),
];
