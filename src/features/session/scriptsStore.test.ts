// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const mem = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => void mem.set(k, v),
    removeItem: (k: string) => void mem.delete(k),
    clear: () => mem.clear(),
    key: (i: number) => [...mem.keys()][i] ?? null,
    get length() {
      return mem.size;
    },
  });
});
vi.mock("@/lib/bindings", () => ({ events: { scriptRunChanged: { listen: vi.fn() } } }));

import {
  addDockTerminal,
  closeDockTerminal,
  dockStateFor,
  setDockTermId,
  useScriptsStore,
} from "./scriptsStore";

const WS = "ws-1";
const KEY = "hyprlayer:scripts-dock";

const persisted = () =>
  (JSON.parse(localStorage.getItem(KEY) ?? "{}") as { state: { dock: Record<string, unknown> } })
    .state;

beforeEach(() => {
  localStorage.clear();
  useScriptsStore.setState({ dock: {}, running: {}, pendingRun: {}, activeWorkstream: null });
});

describe("dock terminal persistence", () => {
  it("keeps terminal ids across a rehydrate so a relaunch can reattach", async () => {
    const term = addDockTerminal(WS);
    setDockTermId(WS, term, "tid-abc");
    expect((persisted().dock[WS] as { termIds: Record<number, string> }).termIds[term]).toBe(
      "tid-abc",
    );

    // A relaunch: the in-memory store starts empty and storage still holds the snapshot.
    const snapshot = localStorage.getItem(KEY) ?? "";
    useScriptsStore.setState({ dock: {} });
    localStorage.setItem(KEY, snapshot);
    await useScriptsStore.persist.rehydrate();
    const dock = dockStateFor(WS);
    expect(dock.termIds[term]).toBe("tid-abc");
    expect(dock.terminals).toEqual([1, term]);
    expect(dock.tab).toEqual({ term });
    expect(dock.open).toBe(true);
  });

  it("forgets a closed terminal's id", () => {
    const term = addDockTerminal(WS);
    setDockTermId(WS, term, "tid-abc");
    expect(closeDockTerminal(WS, term)).toBe("tid-abc");
    expect((persisted().dock[WS] as { termIds: Record<number, string> }).termIds).toEqual({});
  });
});
