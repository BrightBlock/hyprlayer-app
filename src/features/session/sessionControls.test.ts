// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { leaf } from "./paneTree";
import {
  contextWindowFor,
  controlsKey,
  controlsSignature,
  useFocusedSessionControls,
  usePublishSessionControls,
  useSessionControlsStore,
  type SessionControlActions,
  type SessionControlsSnapshot,
} from "./sessionControls";
import { defaultTabState, useWorkstreamTabsStore, type WsTabState } from "./workstreamTabs";

function snapshot(patch: Partial<SessionControlsSnapshot> = {}): SessionControlsSnapshot {
  return {
    harness: "claude",
    interactive: true,
    model: {
      label: "Opus 5",
      hint: "1M",
      activeKey: "claude:opus",
      groups: [
        {
          key: "claude",
          label: "Claude Code",
          icon: createElement("span", null, "C"),
          items: [
            { key: "claude:opus", label: "Opus 5", icon: createElement("span", null, "C") },
            { key: "claude:sonnet", label: "Sonnet 5" },
          ],
        },
      ],
    },
    effort: {
      label: "High",
      activeKey: "high",
      items: [
        { key: "medium", label: "Medium" },
        { key: "high", label: "High" },
      ],
    },
    permissionKey: "default",
    context: { tokens: 24_000, window: 200_000, title: "Context 24k / 200k" },
    ...patch,
  };
}

function actions(): SessionControlActions {
  return {
    pickModel: vi.fn(),
    pickEffort: vi.fn(),
    setPermissionMode: vi.fn(),
    openHarnessSettings: vi.fn(),
  };
}

const entry = (key: string) => useSessionControlsStore.getState().byShell[key];

beforeEach(() => {
  useSessionControlsStore.setState({ byShell: {} });
  useWorkstreamTabsStore.setState({ byWs: {} });
});

afterEach(() => {
  cleanup();
});

describe("controlsSignature", () => {
  it("is the same for equal data in new objects", () => {
    expect(controlsSignature(snapshot())).toBe(controlsSignature(snapshot()));
  });

  it("ignores icons, and does not choke on a circular one", () => {
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    const withIcon = snapshot();
    withIcon.model.groups[0].icon = circular as unknown as ReactNode;
    withIcon.model.groups[0].items[0].icon = createElement("b");
    expect(() => controlsSignature(withIcon)).not.toThrow();
    expect(controlsSignature(withIcon)).toBe(controlsSignature(snapshot()));
  });

  it("changes with each value the bar shows", () => {
    const base = controlsSignature(snapshot());
    const changed: SessionControlsSnapshot[] = [
      snapshot({ model: { ...snapshot().model, label: "Sonnet 5" } }),
      snapshot({ model: { ...snapshot().model, activeKey: "claude:sonnet" } }),
      snapshot({ effort: { ...snapshot().effort!, activeKey: "medium", label: "Medium" } }),
      snapshot({ effort: null }),
      snapshot({ permissionKey: "plan" }),
      snapshot({ interactive: false }),
      snapshot({ context: { tokens: 25_000, window: 200_000, title: "Context 25k / 200k" } }),
    ];
    for (const s of changed) expect(controlsSignature(s)).not.toBe(base);
  });
});

describe("contextWindowFor", () => {
  it("prefers the session's own window on every harness", () => {
    expect(contextWindowFor(32_768, "opencode", 200_000)).toBe(32_768);
    expect(contextWindowFor(400_000, "codex", 200_000)).toBe(400_000);
    expect(contextWindowFor(1_000_000, "claude", 200_000)).toBe(1_000_000);
  });

  it("falls back to the picked model's window only on Claude", () => {
    expect(contextWindowFor(null, "claude", 200_000)).toBe(200_000);
    expect(contextWindowFor(null, "codex", 1_000_000)).toBeNull();
    expect(contextWindowFor(null, "opencode", 1_000_000)).toBeNull();
  });

  it("treats a non-positive session window as unknown", () => {
    expect(contextWindowFor(0, "codex", 200_000)).toBeNull();
    expect(contextWindowFor(0, "claude", 200_000)).toBe(200_000);
  });
});

describe("usePublishSessionControls", () => {
  const KEY = controlsKey("w1", 1);

  it("publishes under the shell's key without re-rendering the publisher", () => {
    let renders = 0;
    renderHook(() => {
      renders += 1;
      usePublishSessionControls(KEY, snapshot(), actions());
    });
    expect(entry(KEY)?.model.label).toBe("Opus 5");
    expect(renders).toBe(1);
  });

  it("does not re-publish identical data, yet calls the newest closure", () => {
    const first = actions();
    const second = actions();
    const { rerender } = renderHook(({ a }) => usePublishSessionControls(KEY, snapshot(), a), {
      initialProps: { a: first },
    });
    const published = entry(KEY);

    const writes = vi.fn();
    const off = useSessionControlsStore.subscribe(writes);
    rerender({ a: second });
    off();

    expect(writes).not.toHaveBeenCalled();
    expect(entry(KEY)).toBe(published);
    published.actions.pickModel("claude:sonnet");
    expect(second.pickModel).toHaveBeenCalledWith("claude:sonnet");
    expect(first.pickModel).not.toHaveBeenCalled();
  });

  it("re-publishes when a shown value changes", () => {
    const { rerender } = renderHook(({ s }) => usePublishSessionControls(KEY, s, actions()), {
      initialProps: { s: snapshot() },
    });
    rerender({ s: snapshot({ permissionKey: "plan" }) });
    expect(entry(KEY)?.permissionKey).toBe("plan");
  });

  it("removes its entry on unmount", () => {
    const { unmount } = renderHook(() => usePublishSessionControls(KEY, snapshot(), actions()));
    unmount();
    expect(entry(KEY)).toBeUndefined();
  });

  it("leaves a newer publisher's entry alone when an old one unmounts late", () => {
    const old = renderHook(() => usePublishSessionControls(KEY, snapshot(), actions()));
    renderHook(() =>
      usePublishSessionControls(KEY, snapshot({ model: { ...snapshot().model, label: "Newer" } }), actions()),
    );
    old.unmount();
    expect(entry(KEY)?.model.label).toBe("Newer");
  });

  it("keeps other shells' entries when one unmounts", () => {
    const other = controlsKey("w1", 2);
    renderHook(() => usePublishSessionControls(other, snapshot(), actions()));
    const { unmount } = renderHook(() => usePublishSessionControls(KEY, snapshot(), actions()));
    unmount();
    expect(entry(other)).toBeDefined();
  });
});

describe("useFocusedSessionControls", () => {
  function splitTabs(activePaneId: string): WsTabState {
    return {
      shells: [
        { id: 1, title: "Claude", mode: "general", draft: "", docs: [], attachments: [], paneId: "p1" },
        { id: 2, title: "Codex", mode: "general", draft: "", docs: [], attachments: [], paneId: "p2" },
      ],
      layout: { type: "split", dir: "row", ratio: 0.5, a: leaf("p1"), b: leaf("p2") },
      panes: { p1: { activeId: 1 }, p2: { activeId: 2 } },
      activePaneId,
    };
  }

  it("follows the focused pane of a split", () => {
    renderHook(() => usePublishSessionControls(controlsKey("w1", 1), snapshot(), actions()));
    renderHook(() =>
      usePublishSessionControls(controlsKey("w1", 2), snapshot({ harness: "codex" }), actions()),
    );
    useWorkstreamTabsStore.setState({ byWs: { w1: splitTabs("p1") } });

    const { result } = renderHook(() => useFocusedSessionControls("w1"));
    expect(result.current?.harness).toBe("claude");

    act(() => useWorkstreamTabsStore.setState({ byWs: { w1: splitTabs("p2") } }));
    expect(result.current?.harness).toBe("codex");
  });

  it("reads the default tab when the workstream has no tab state yet", () => {
    const first = defaultTabState().shells[0].id;
    renderHook(() => usePublishSessionControls(controlsKey("w1", first), snapshot(), actions()));
    const { result } = renderHook(() => useFocusedSessionControls("w1"));
    expect(result.current?.model.label).toBe("Opus 5");
  });

  it("is null when the focused tab publishes nothing, or there is no workstream", () => {
    renderHook(() => usePublishSessionControls(controlsKey("w1", 1), snapshot(), actions()));
    useWorkstreamTabsStore.setState({ byWs: { w1: splitTabs("p2") } });
    expect(renderHook(() => useFocusedSessionControls("w1")).result.current).toBeNull();
    expect(renderHook(() => useFocusedSessionControls(null)).result.current).toBeNull();
  });
});
