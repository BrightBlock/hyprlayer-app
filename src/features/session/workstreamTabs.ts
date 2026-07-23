/**
 * Per-workstream agent-tab strips, held module-side (not in ProjectPage state) so a
 * tab's session binding + draft + context docs survive navigation and app restart.
 * Persisted to localStorage; on reopen the transcript rebuilds from the session's
 * SQLite event log (see `restoreSessionIfNeeded` in [`sessionStore`]).
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { Attachment } from "./attachments";
import type { ActionAgentDefaults } from "@/lib/domain/harnessing";
import type { StageKey } from "@/lib/domain/stages";

/** A doc attached as context to ONE agent tab. */
export type AttachedDoc = {
  path: string;
  role: string;
  /** Auto-suggested by the pane's mode (not user-attached). */
  suggested?: boolean;
};

/** A chat tab inside ONE workstream (each owns its own set). Context docs are
 *  PER TAB. Once a session starts here, `sessionId` binds the tab to that live (or
 *  terminal) session; its transcript lives in the session store, surviving switches
 *  and re-attaching on demand. */
export type ShellTab = {
  id: number;
  title: string;
  /** Set once the title is "claimed" (auto-generated from the first prompt or
   *  manually renamed) so auto-titling never overwrites it. Undefined on a fresh tab. */
  titleLocked?: boolean;
  mode: StageKey;
  draft: string;
  docs: AttachedDoc[];
  /** Paths of suggested docs the user dismissed; never re-suggested for the tab's
   *  lifetime, even if the mode switches back. Also covers auto-captured handoff docs,
   *  so a detached one isn't re-attached by the in-session re-write. */
  dismissedSuggestions?: string[];
  /** User dismissed the "context is filling up — create a handoff?" nudge for this
   *  tab. Suppresses only that suggestion; an auto-captured handoff doc (into [`docs`])
   *  still surfaces the ready-to-resume banner. */
  handoffNudgeDismissed?: boolean;
  /** Files/screenshots staged in this tab's composer. Volatile sources are copied into
   *  the worktree's `.hyprlayer/attachments/` first; sent with the next turn, then cleared. */
  attachments: Attachment[];
  /** Model + reasoning-effort keys chosen for the NEXT session this tab starts
   *  (see models.ts); fixed at spawn. Undefined → the defaults. */
  model?: string;
  effort?: string;
  /** Permission-mode key chosen for the NEXT session (see permissions.ts);
   *  fixed at spawn. Undefined → "auto" (the stage's default posture). */
  permissionMode?: string;
  /** Which harness the NEXT session runs on ("claude" | "codex" | "opencode");
   *  fixed at spawn, persisted so switching tabs doesn't reset it.
   *  Undefined → "claude". */
  harness?: string;
  /** OpenCode `provider/model` id for the NEXT session when `harness` is
   *  "opencode" (Claude's model lives in `model`). Undefined → the OpenCode default. */
  opencodeModel?: string;
  /** Codex model id + reasoning effort for the NEXT session when `harness` is
   *  "codex". Kept separate from `model`/`effort` because Codex's model ids and
   *  effort ladder are its own (see CODEX_EFFORTS), so one tab can remember a
   *  pick per harness. Undefined → the Codex defaults. */
  codexModel?: string;
  codexEffort?: string;
  sessionId?: string;
  /** When set, this tab is a read-only markdown preview of a doc (its path),
   *  not an agent chat — opened from the diff sidebar, closed when done. */
  docPreviewPath?: string;
  /** When set, this tab is a rich diff of a worktree file (its path), against
   *  either its staged or working version. Opened from the Changes panel. */
  diffView?: { path: string; staged: boolean };
  /** When set, this tab is a read-only view of a worktree file (its path).
   *  Opened from the Files tree. */
  filePath?: string;
  /** When set, this tab is an interactive terminal (PTY) for the worktree.
   *  `terminalId` binds it to a live backend PTY once opened; absent until then. */
  terminalView?: { terminalId?: string };
  /** When set, this tab is a Multiplexer agent: an engine CLI in a PTY instead of the
   *  native chat (presence ⇒ kind "pty", else "native"; only when `multiplexer.enabled`).
   *  `engineId` selects the engine; `terminalId` binds a live backend PTY once spawned
   *  (persisted, so it reattaches on reload); `bypass` = bypass-permissions mode, so a
   *  cold-start relaunch resumes that way. Live working/blocked/done/idle state lives in
   *  the multiplexer store, not here. */
  ptyAgent?: {
    engineId: string;
    terminalId?: string;
    bypass?: boolean;
    /** This tab's own conversation id, so it resumes ITS conversation, not the most-recent
     *  in the worktree (two agents in one workstream no longer collide). Minted at creation
     *  for "assign" engines (Claude, Gemini); learned from disk for "capture" (Codex, Antigravity). */
    sessionId?: string;
    /** Whether a conversation has begun (first keystroke). Persisted so a cold-start relaunch
     *  resumes by sessionId only when there's something to resume — else an untouched tab is fresh. */
    started?: boolean;
  };
  /** Which pane this shell lives in. Absent/"main" → main pane; "right" → the right split. */
  panel?: "main" | "right";
};

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

/** Which pane a tab lives in — `panel` is optional, and absent means "main". */
const paneOf = (s: ShellTab): "main" | "right" => (s.panel === "right" ? "right" : "main");

/** The focused pane and its tabs, in on-screen order (so drag-reordering counts). */
function focusedPane(state: WsTabState): { pane: "main" | "right"; members: ShellTab[] } {
  const pane = state.activePane ?? "main";
  return { pane, members: state.shells.filter((s) => paneOf(s) === pane) };
}

/** Focus `id` in `pane`, making that pane the focused one. */
function focusShell(state: WsTabState, pane: "main" | "right", id: number): WsTabState {
  return pane === "right"
    ? { ...state, rightActiveId: id, activePane: "right" }
    : { ...state, activeId: id, activePane: "main" };
}

/**
 * Cycle the active tab within the *focused* pane by `dir` (+1 next, -1 previous),
 * wrapping around the ends. The split's two panes each have their own strip, so
 * cycling only moves through the pane you're looking at (per `activePane`) and
 * leaves the other pane's selection untouched. Pure: returns the next state, or
 * the input unchanged when the focused pane holds fewer than two tabs (nothing to
 * cycle). Tabs cycle in their on-screen order, so drag-reordering is respected.
 */
export function cycleActiveTab(state: WsTabState, dir: 1 | -1): WsTabState {
  const { pane, members } = focusedPane(state);
  if (members.length < 2) return state;
  const activeId = pane === "right" ? state.rightActiveId : state.activeId;
  // An unresolved active id reads as the pane's first tab (the same fallback the
  // project page renders), so step relative to index 0.
  const from = Math.max(0, members.findIndex((s) => s.id === activeId));
  return focusShell(state, pane, members[(from + dir + members.length) % members.length].id);
}

/**
 * Select the `index`-th tab (0-based) of the focused pane — Safari's ⌘1–⌘8. A
 * no-op when the pane has no such tab, as ⌘5 does with four tabs open. Pure.
 */
export function selectTabAt(state: WsTabState, index: number): WsTabState {
  const { pane, members } = focusedPane(state);
  const target = members[index];
  return target ? focusShell(state, pane, target.id) : state;
}

/**
 * Select the focused pane's last tab — Safari's ⌘9, which means the rightmost
 * tab, never the ninth. Pure; a no-op on an empty pane.
 */
export function selectLastTab(state: WsTabState): WsTabState {
  const { pane, members } = focusedPane(state);
  const target = members[members.length - 1];
  return target ? focusShell(state, pane, target.id) : state;
}

/**
 * Remove `shellId` from the strip, settling the active id / focused pane onto a
 * neighbour and collapsing the split when its last right-pane shell goes. The
 * shared core of closing and archiving — neither ends the session here, that's
 * the caller's job. Pure: returns the input unchanged when the shell is missing
 * or is the main pane's last tab (the main pane always keeps one agent).
 *
 * Closing an *inactive* tab leaves the selection alone. Closing the active one
 * selects the tab to its right, or the new rightmost tab when the closed one was
 * last: Safari's rule.
 */
export function removeShellState(state: WsTabState, shellId: number): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell) return state;
  const pane = paneOf(shell);
  const onRight = pane === "right";
  if (!onRight && state.shells.filter((s) => paneOf(s) === "main").length <= 1) return state;

  const before = state.shells.filter((s) => paneOf(s) === pane);
  const closedIndex = before.findIndex((s) => s.id === shellId);
  const after = before.filter((s) => s.id !== shellId);
  // The tab that was to the right now sits at the closed tab's index; past the
  // end (the closed tab was last) that clamps back onto the new final tab.
  const successor = after[Math.min(closedIndex, after.length - 1)]?.id;

  const remaining = state.shells.filter((s) => s.id !== shellId);
  if (onRight) {
    return {
      ...state,
      shells: remaining,
      rightActiveId: state.rightActiveId === shellId ? successor : state.rightActiveId,
      rightSplit: after.length > 0 ? state.rightSplit : false,
      activePane: after.length > 0 ? state.activePane : "main",
    };
  }
  return {
    ...state,
    shells: remaining,
    activeId: state.activeId === shellId ? (successor ?? state.activeId) : state.activeId,
  };
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
 * Bind a freshly-started session to an existing (unstarted) shell tab and focus
 * it — the module-level twin of SessionChat's `onSessionStarted`, for prompt
 * runs that start a session on the focused tab from outside the component.
 */
export function bindSessionToShell(
  workstreamId: string,
  shellId: number,
  sessionId: string,
): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    const shell = current.shells.find((s) => s.id === shellId);
    if (!shell) return all;
    const inRightPane = shell.panel === "right";
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: current.shells.map((s) => (s.id === shellId ? { ...s, sessionId } : s)),
        activeId: inRightPane ? current.activeId : shellId,
        rightActiveId: inRightPane ? shellId : current.rightActiveId,
        rightSplit: inRightPane ? true : current.rightSplit,
        activePane: inRightPane ? "right" : "main",
        archiveOpen: false,
      },
    };
  });
}

/**
 * Open a fresh plain-chat tab bound to `sessionId` and select it — the landing
 * spot for a prompt run when the focused tab can't take the session (it's a doc
 * preview, a dead action tab, or already bound). Named after the prompt so the
 * strip reads what's running.
 */
export function openPromptTab(workstreamId: string, sessionId: string, title: string): void {
  updateWorkstreamTabs((all) => {
    const current = { ...(all[workstreamId] ?? defaultTabState()), archiveOpen: false };
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
            sessionId,
            panel: "main",
            // The prompt's name IS the intended title — don't let auto-titling
            // overwrite it from the first turn.
            titleLocked: true,
          },
        ],
        activeId: nextId,
        activePane: "main",
      },
    };
  });
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

/** The shell fields that make an action tab's composer show the harness/model/
 *  effort its session launched with: Claude carries a model + effort key, Codex a
 *  model id + its own effort, OpenCode a `provider/model` id. Cross-harness fields
 *  are cleared so a switch never leaves a stale model behind. Empty when there are
 *  no defaults (focusing a live tab), so nothing is overwritten. */
function shellSeedFromDefaults(d?: ActionAgentDefaults): Partial<ShellTab> {
  if (!d) return {};
  const cleared = {
    model: undefined,
    effort: undefined,
    opencodeModel: undefined,
    codexModel: undefined,
    codexEffort: undefined,
  };
  if (d.harness === "claude") {
    return {
      ...cleared,
      harness: "claude",
      model: d.model ?? undefined,
      effort: d.effort ?? undefined,
    };
  }
  if (d.harness === "codex") {
    return {
      ...cleared,
      harness: "codex",
      codexModel: d.model ?? undefined,
      codexEffort: d.effort ?? undefined,
    };
  }
  return { ...cleared, harness: d.harness, opencodeModel: d.model ?? undefined };
}

/**
 * Open (creating if absent) the dedicated Ship tab for a workstream, bind it to
 * `sessionId`, and select it. Shipping reuses one tab per workstream rather than
 * being a chosen chat mode (see `shipWorkstream` in [`sessionStore`]).
 */
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
