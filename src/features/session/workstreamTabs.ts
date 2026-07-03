/**
 * Per-workstream agent-tab strips, held module-side (not in ProjectPage's
 * component state) so a tab's binding to its session — plus its draft and
 * attached context docs — survives navigating away from the project page and
 * back. Persisted to localStorage so it also survives an app restart: the tab
 * keeps its `sessionId`, and on reopen the transcript is rebuilt from the
 * session's SQLite event log (see `restoreSessionIfNeeded` in [`sessionStore`]).
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { ShellTab } from "../../routes/ProjectPage";
import type { ActionAgentDefaults } from "./harnessing";

/** One workstream's whole tab strip, updated atomically.
 *
 *  A workstream can run a side-by-side split: each shell carries a `panel`
 *  ("main" | "right"), and the two panes track their own active shell —
 *  `activeId` for the main pane, `rightActiveId` for the right one. `rightSplit`
 *  is whether the right pane is open; `activePane` is the focused pane (only its
 *  active tab reads as fully selected). */
export type WsTabState = {
  shells: ShellTab[];
  activeId: number;
  rightSplit?: boolean;
  rightActiveId?: number;
  activePane?: "main" | "right";
  /** Agents archived out of this workstream's strip, newest first. Persisted
   *  alongside the rest of the strip, so archiving survives an app restart. */
  archived?: ArchivedShell[];
  /** Whether the full-pane "Archived agents" view is showing instead of the
   *  focused tab. Owned by the tab state (not component state) so every tab
   *  action can leave it in one place — focusing any tab clears it. */
  archiveOpen?: boolean;
};

/** A coarse session-lifecycle bucket frozen at archive time (the live entry
 *  isn't kept around while archived), used only to label the archive row.
 *  `stopped` lumps interrupted/killed/crashed. */
export type ArchivedStatus = "done" | "stopped" | "idle";

/** An agent lifted out of the visible strip but kept whole: the `shell` is
 *  restored verbatim on unarchive, so its session re-attaches (or rehydrates
 *  from the transcript) like any reopened tab. Never touches the branch/worktree. */
export type ArchivedShell = {
  shell: ShellTab;
  /** Epoch ms when archived; orders the list and renders "archived {ago}". */
  archivedAt: number;
  status: ArchivedStatus;
};

export const defaultTabState = (): WsTabState => ({
  shells: [
    { id: 1, title: "Agent", mode: "general", draft: "", docs: [], attachments: [], panel: "main" },
  ],
  activeId: 1,
});

/** Next free shell id for a workstream — ids only ever grow, so a tab's id is
 *  stable for its lifetime even as siblings come and go. Counts archived agents
 *  too: an archived shell keeps its id, so a freshly-minted tab must never reuse
 *  it (else unarchiving would collide with a live tab). */
export const nextShellId = (shells: ShellTab[], archived: ArchivedShell[] = []): number =>
  Math.max(0, ...shells.map((s) => s.id), ...archived.map((a) => a.shell.id)) + 1;

/**
 * Cycle the active tab within the *focused* pane by `dir` (+1 next, -1 previous),
 * wrapping around the ends. The split's two panes each have their own strip, so
 * cycling only moves through the pane you're looking at (per `activePane`) and
 * leaves the other pane's selection untouched. Pure: returns the next state, or
 * the input unchanged when the focused pane holds fewer than two tabs (nothing to
 * cycle). Tabs cycle in their on-screen order, so drag-reordering is respected.
 */
export function cycleActiveTab(state: WsTabState, dir: 1 | -1): WsTabState {
  const pane = state.activePane ?? "main";
  const inPane = (s: ShellTab) =>
    pane === "right" ? s.panel === "right" : (s.panel ?? "main") !== "right";
  const members = state.shells.filter(inPane);
  if (members.length < 2) return state;
  const activeId = pane === "right" ? state.rightActiveId : state.activeId;
  // An unresolved active id reads as the pane's first tab (the same fallback the
  // project page renders), so step relative to index 0.
  const from = Math.max(0, members.findIndex((s) => s.id === activeId));
  const nextId = members[(from + dir + members.length) % members.length].id;
  return pane === "right"
    ? { ...state, rightActiveId: nextId, activePane: "right" }
    : { ...state, activeId: nextId, activePane: "main" };
}

/**
 * Remove `shellId` from the strip, settling the active id / focused pane onto a
 * neighbour and collapsing the split when its last right-pane shell goes. The
 * shared core of closing and archiving — neither ends the session here, that's
 * the caller's job. Pure: returns the input unchanged when the shell is missing
 * or is the main pane's last tab (the main pane always keeps one agent).
 */
export function removeShellState(state: WsTabState, shellId: number): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell) return state;
  const onRight = shell.panel === "right";
  const isMain = (s: ShellTab) => (s.panel ?? "main") !== "right";
  if (!onRight && state.shells.filter(isMain).length <= 1) return state;
  const remaining = state.shells.filter((s) => s.id !== shellId);
  if (onRight) {
    const rights = remaining.filter((s) => s.panel === "right");
    return {
      ...state,
      shells: remaining,
      rightActiveId:
        state.rightActiveId === shellId ? rights[rights.length - 1]?.id : state.rightActiveId,
      rightSplit: rights.length > 0 ? state.rightSplit : false,
      activePane: rights.length > 0 ? state.activePane : "main",
    };
  }
  const mains = remaining.filter(isMain);
  const activeId =
    state.activeId === shellId ? (mains[mains.length - 1]?.id ?? state.activeId) : state.activeId;
  return { ...state, shells: remaining, activeId };
}

/**
 * Archive `shellId`: the same strip removal as closing (`removeShellState`), but
 * the shell is kept whole in `archived` (newest first) instead of ended, so its
 * branch, worktree and transcript survive to resume on unarchive. No-op when the
 * shell is missing or is the main pane's last tab.
 */
export function archiveShellState(
  state: WsTabState,
  shellId: number,
  status: ArchivedStatus,
  archivedAt: number,
): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell) return state;
  const next = removeShellState(state, shellId);
  if (next === state) return state; // last main tab — nothing archived
  return { ...next, archived: [{ shell, archivedAt, status }, ...(state.archived ?? [])] };
}

/**
 * Restore an archived agent to the main strip and select it — re-homed into the
 * main pane (its old pane may have since closed) but otherwise verbatim, its
 * `sessionId` intact so the session re-attaches or rehydrates on mount. No-op
 * when no archived entry matches `shellId`.
 */
export function unarchiveShellState(state: WsTabState, shellId: number): WsTabState {
  const archived = state.archived ?? [];
  const entry = archived.find((a) => a.shell.id === shellId);
  if (!entry) return state;
  const restored: ShellTab = { ...entry.shell, panel: "main" };
  return {
    ...state,
    shells: [...state.shells, restored],
    archived: archived.filter((a) => a.shell.id !== shellId),
    activeId: restored.id,
    activePane: "main",
  };
}

/**
 * Close the archived-agents view when `next` refocuses a tab relative to `prev`.
 * A navigation (select/cycle/open a tab) clears it, but a removal (close/archive
 * shrinks the strip) and a background patch (no focus change) leave it open — so
 * managing the archive doesn't yank you out of the view. Pure; returns `next`
 * unchanged when the view should stay (or was already closed).
 */
export function clearArchiveOnRefocus(prev: WsTabState, next: WsTabState): WsTabState {
  const refocused =
    next.shells.length >= prev.shells.length &&
    (next.activeId !== prev.activeId ||
      next.activePane !== prev.activePane ||
      next.rightActiveId !== prev.rightActiveId);
  return refocused && next.archiveOpen ? { ...next, archiveOpen: false } : next;
}

type TabsStore = { byWs: Record<string, WsTabState> };

export const useWorkstreamTabsStore = create<TabsStore>()(
  persist(() => ({ byWs: {} }), {
    name: "hyprlayer:workstream-tabs",
    // v2 added `attachments` to every shell.
    // v3 added `dismissedSuggestions` to every shell — backfill it on rehydrate
    // so tabs persisted by an older build don't crash on accessing the field.
    // v4 added `titleLocked`: lock every pre-existing tab so auto-titling never
    // rewrites a name the user has already been looking at (custom or default);
    // only tabs created after this build start unlocked and get auto-titled.
    // v5 added per-workstream `archived` agents — backfill it to an empty list.
    version: 5,
    migrate: (persisted) => {
      const state = persisted as TabsStore | undefined;
      if (!state?.byWs) return { byWs: {} };
      for (const ws of Object.values(state.byWs)) {
        ws.shells = (ws.shells ?? []).map((s) => ({
          ...s,
          docs: s.docs ?? [],
          attachments: s.attachments ?? [],
          dismissedSuggestions: s.dismissedSuggestions ?? [],
          titleLocked: s.titleLocked ?? true,
        }));
        ws.archived = ws.archived ?? [];
      }
      return state;
    },
  }),
);

/** Apply a functional update to the whole `{ wsId -> tab strip }` map. */
export function updateWorkstreamTabs(
  next: (prev: Record<string, WsTabState>) => Record<string, WsTabState>,
): void {
  useWorkstreamTabsStore.setState((s) => ({ byWs: next(s.byWs) }));
}

/**
 * A transient, one-shot "focus this shell's composer" signal. An action that
 * swaps which agent is on screen — e.g. toggling the terminal away — sets the
 * target shell id; that shell's composer consumes it once mounted and focuses
 * its textarea, so the user lands ready to type. Deliberately NOT persisted and
 * NOT part of the tab strip: it's a fire-and-forget signal, cleared the instant
 * it's honored so it never re-steals focus on a later render.
 */
export const useComposerFocusStore = create<{ pendingShellId: number | null }>(() => ({
  pendingShellId: null,
}));

/** Ask `shellId`'s composer to grab focus as soon as it's on screen. */
export function requestComposerFocus(shellId: number): void {
  useComposerFocusStore.setState({ pendingShellId: shellId });
}

/** Claim a pending focus for `shellId`: clears the signal and returns true iff it
 *  was the requested shell, so the composer focuses exactly once. */
export function consumeComposerFocus(shellId: number): boolean {
  if (useComposerFocusStore.getState().pendingShellId !== shellId) return false;
  useComposerFocusStore.setState({ pendingShellId: null });
  return true;
}

/**
 * Open (or re-focus) a read-only doc-preview tab for `path` in a workstream's
 * main pane, and select it. The module-level twin of ProjectPage's
 * `openDocPreview`, callable from outside the project page (e.g. the global
 * search palette) so opening a library doc keeps you in your current workstream
 * instead of navigating away. Reuses an already-open tab for the same doc.
 */
export function openDocPreviewTab(workstreamId: string, path: string, title: string): void {
  updateWorkstreamTabs((all) => {
    // Opening a tab from outside the project page also leaves the archived-agents
    // view (the in-page `updateTabs` funnel that normally clears it can't reach).
    const current = { ...(all[workstreamId] ?? defaultTabState()), archiveOpen: false };
    const existing = current.shells.find((s) => s.docPreviewPath === path);
    if (existing) {
      const inRightPane = existing.panel === "right";
      return {
        ...all,
        [workstreamId]: {
          ...current,
          ...(inRightPane
            ? { rightActiveId: existing.id, rightSplit: true }
            : { activeId: existing.id }),
          activePane: inRightPane ? "right" : "main",
        },
      };
    }
    const nextId = nextShellId(current.shells, current.archived);
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: [
          ...current.shells,
          {
            id: nextId,
            title,
            mode: "general",
            draft: "",
            docs: [],
            attachments: [],
            docPreviewPath: path,
            panel: "main",
          },
        ],
        activeId: nextId,
        activePane: "main",
      },
    };
  });
}

/**
 * Open (creating if absent) the dedicated Ship tab for a workstream, bind it to
 * `sessionId`, and select it. Shipping reuses one tab per workstream rather than
 * being a chosen chat mode (see `shipWorkstream` in [`sessionStore`]).
 */
/** The shell fields that make an action tab's composer show the harness/model/
 *  effort its session launched with: Claude carries a model + effort key,
 *  OpenCode a `provider/model` id. Cross-harness fields are cleared so a switch
 *  (Claude ⇄ OpenCode) never leaves a stale model behind. Empty when there are
 *  no defaults (focusing a live tab), so nothing is overwritten. */
function shellSeedFromDefaults(d?: ActionAgentDefaults): Partial<ShellTab> {
  if (!d) return {};
  if (d.harness === "claude") {
    return {
      harness: "claude",
      model: d.model ?? undefined,
      effort: d.effort ?? undefined,
      opencodeModel: undefined,
    };
  }
  return {
    harness: d.harness,
    opencodeModel: d.model ?? undefined,
    model: undefined,
    effort: undefined,
  };
}

export function openShipTab(
  workstreamId: string,
  sessionId: string,
  defaults?: ActionAgentDefaults,
): void {
  openDedicatedTab(workstreamId, sessionId, "ship", "Ship", defaults);
}

/**
 * Open (creating if absent) the dedicated Commit tab for a workstream. Like
 * shipping, committing reuses one tab per workstream (see `commitWorkstream`).
 */
export function openCommitTab(
  workstreamId: string,
  sessionId: string,
  defaults?: ActionAgentDefaults,
): void {
  openDedicatedTab(workstreamId, sessionId, "commit", "Commit", defaults);
}

/** Shared logic for the action-driven tabs (Ship, Commit): reuse the workstream's
 *  single `mode` tab if it exists (rebinding it to `sessionId`), else create one
 *  titled `title`; either way select it. */
function openDedicatedTab(
  workstreamId: string,
  sessionId: string,
  mode: ShellTab["mode"],
  title: string,
  defaults?: ActionAgentDefaults,
): void {
  // Seed the action tab's composer to match what its session launched with, so
  // the model/effort picker reflects the configured Commit/Ship defaults. Empty
  // when just focusing an existing tab (no fresh launch) — nothing is touched.
  const seed = shellSeedFromDefaults(defaults);
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    const existing = current.shells.find((s) => s.mode === mode);
    if (existing) {
      const inRightPane = existing.panel === "right";
      return {
        ...all,
        [workstreamId]: {
          ...current,
          shells: current.shells.map((s) =>
            s.id === existing.id ? { ...s, sessionId, ...seed } : s,
          ),
          activeId: inRightPane ? current.activeId : existing.id,
          rightActiveId: inRightPane ? existing.id : current.rightActiveId,
          rightSplit: inRightPane ? true : current.rightSplit,
          activePane: inRightPane ? "right" : "main",
        },
      };
    }
    const nextId = nextShellId(current.shells, current.archived);
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: [
          ...current.shells,
          {
            id: nextId,
            title,
            mode,
            draft: "",
            docs: [],
            attachments: [],
            sessionId,
            panel: "main",
            ...seed,
          },
        ],
        activeId: nextId,
        activePane: "main",
      },
    };
  });
}
