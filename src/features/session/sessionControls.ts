import { useEffect, useMemo, useRef, useState } from "react";
import { create } from "zustand";

import type { OptionMenuGroup, OptionMenuItem } from "@/components/OptionMenuList";
import type { GoalStatus, GoalStep, Harness } from "@/lib/bindings";
import { activeShellIn, defaultTabState, focusedPaneId, useWorkstreamTabsStore } from "./workstreamTabs";

export type SessionControlsSnapshot = {
  harness: Harness;
  /** False once the session is terminal, when the composer hides its toolbar. */
  interactive: boolean;
  model: { label: string; hint: string | null; activeKey: string; groups: OptionMenuGroup[] };
  /** Null on OpenCode, and on a Claude model without effort tiers. */
  effort: { label: string; activeKey: string; items: OptionMenuItem[] } | null;
  /** Null on OpenCode, which has no mode menu. */
  permissionKey: string | null;
  /** The harness put itself in plan mode; it leaves on its own. */
  planning: boolean;
  context: { tokens: number; window: number; title: string } | null;
  /** Fixed values; the chip ticks its clock from `activeSince`, so nothing republishes per second. */
  goal: {
    id: string;
    status: GoalStatus;
    step: GoalStep | null;
    iteration: number;
    maxIterations: number;
    spent: string;
    activeMs: number;
    activeSince: number | null;
  } | null;
};

export type SessionControlActions = {
  pickModel(namespacedKey: string): void;
  pickEffort(key: string): void;
  setPermissionMode(key: string): void;
  openHarnessSettings(): void;
};

export type SessionControls = SessionControlsSnapshot & { actions: SessionControlActions };

type Entry = SessionControls & { owner: object };

/** Each chat pane's composer controls, published by `SessionChat` for the status bar. */
export const useSessionControlsStore = create<{ byShell: Record<string, Entry> }>(() => ({
  byShell: {},
}));

export const controlsKey = (workstreamId: string, shellId: number): string =>
  `${workstreamId}:${shellId}`;

// Icons are React elements, whose `_owner` can be circular.
export function controlsSignature(s: SessionControlsSnapshot): string {
  return JSON.stringify(s, (key, value: unknown) => (key === "icon" ? undefined : value));
}

export function usePublishSessionControls(
  key: string,
  snapshot: SessionControlsSnapshot,
  actions: SessionControlActions,
): void {
  const [owner] = useState(() => ({}));
  const latest = useRef({ snapshot, actions });
  latest.current = { snapshot, actions };
  const stableActions = useMemo<SessionControlActions>(
    () => ({
      pickModel: (k) => latest.current.actions.pickModel(k),
      pickEffort: (k) => latest.current.actions.pickEffort(k),
      setPermissionMode: (k) => latest.current.actions.setPermissionMode(k),
      openHarnessSettings: () => latest.current.actions.openHarnessSettings(),
    }),
    [],
  );
  const signature = controlsSignature(snapshot);

  // Keyed on the signature, so keystrokes and streamed deltas publish nothing.
  useEffect(() => {
    useSessionControlsStore.setState((s) => ({
      byShell: { ...s.byShell, [key]: { ...latest.current.snapshot, actions: stableActions, owner } },
    }));
  }, [key, signature, owner, stableActions]);

  // A remount under the same key may publish before the old instance cleans up.
  useEffect(
    () => () => {
      const { byShell } = useSessionControlsStore.getState();
      if (byShell[key]?.owner !== owner) return;
      const rest = { ...byShell };
      delete rest[key];
      useSessionControlsStore.setState({ byShell: rest });
    },
    [key, owner],
  );
}

const DEFAULT_TABS = defaultTabState();

/** The focused pane's controls; null when it is not a chat (PTY, terminal, viewer). */
export function useFocusedSessionControls(workstreamId: string | null): SessionControls | null {
  const shellId = useWorkstreamTabsStore((s) => {
    if (!workstreamId) return null;
    const tabs = s.byWs[workstreamId] ?? DEFAULT_TABS;
    return activeShellIn(tabs, focusedPaneId(tabs))?.id ?? null;
  });
  return useSessionControlsStore((s) =>
    workstreamId && shellId !== null ? (s.byShell[controlsKey(workstreamId, shellId)] ?? null) : null,
  );
}
