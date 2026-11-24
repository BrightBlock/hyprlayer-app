// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";

import { LEAD_DOCK_MAX, LEAD_DOCK_MIN, useUiStore } from "./uiStore";

const KEY = "hyprlayer:ui";

function persisted(): Record<string, unknown> {
  return JSON.parse(localStorage.getItem(KEY) ?? "{}").state ?? {};
}

async function rehydrateWith(state: Record<string, unknown>) {
  localStorage.setItem(KEY, JSON.stringify({ state, version: 1 }));
  await useUiStore.persist.rehydrate();
}

beforeEach(() => {
  localStorage.clear();
  useUiStore.setState({ leadDockWidth: 300, leadPanelHidden: false });
});

describe("Lead panel width", () => {
  it("clamps to its bounds when set", () => {
    useUiStore.getState().setLeadDockWidth(5000);
    expect(useUiStore.getState().leadDockWidth).toBe(LEAD_DOCK_MAX);
    useUiStore.getState().setLeadDockWidth(10);
    expect(useUiStore.getState().leadDockWidth).toBe(LEAD_DOCK_MIN);
  });

  it("persists across a reload", async () => {
    useUiStore.getState().setLeadDockWidth(380);
    expect(persisted()).toMatchObject({ leadDockWidth: 380 });

    const saved = localStorage.getItem(KEY)!;
    useUiStore.setState({ leadDockWidth: 300 });
    localStorage.setItem(KEY, saved);
    await useUiStore.persist.rehydrate();
    expect(useUiStore.getState().leadDockWidth).toBe(380);
  });

  it("clamps a stored width that is out of bounds", async () => {
    await rehydrateWith({ leadDockWidth: 99999 });
    expect(useUiStore.getState().leadDockWidth).toBe(LEAD_DOCK_MAX);
  });

  it("falls back to the default when nothing usable is stored", async () => {
    await rehydrateWith({ leadDockWidth: "wide" });
    expect(useUiStore.getState().leadDockWidth).toBe(300);
  });
});

describe("Lead panel collapse", () => {
  it("persists across a reload and ignores a stored non-boolean", async () => {
    useUiStore.getState().toggleLeadPanel();
    expect(persisted()).toMatchObject({ leadPanelHidden: true });

    await rehydrateWith({ leadPanelHidden: "yes" });
    expect(useUiStore.getState().leadPanelHidden).toBe(false);
  });
});
