// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SyncComposerNotice, syncNotice } from "./SyncComposerNotice";
import type { SyncState } from "./transcriptSync";

afterEach(cleanup);

function sync(over: Partial<SyncState> = {}): SyncState {
  return {
    localRev: 41,
    committedRev: 43,
    remoteRev: 43,
    resetRev: 0,
    phase: "behind",
    pendingTurns: [],
    authorDevice: "jons-macbook-pro",
    workingDevice: null,
    flushable: false,
    draft: false,
    idle: false,
    fetching: false,
    retry: null,
    syncedWith: null,
    gone: false,
    ...over,
  };
}


function renderNotice(state: SyncState | undefined, drifted = false, loading = false) {
  const handlers = { onPull: vi.fn(), onSendAnyway: vi.fn(), onLoad: vi.fn(), onDismiss: vi.fn() };
  render(<SyncComposerNotice notice={syncNotice(state, drifted, loading)} {...handlers} />);
  return handlers;
}

const status = () => screen.getByRole("status");

describe("syncNotice", () => {
  it("shows nothing against a server without sync until a lifecycle event drifts the view", () => {
    expect(syncNotice(undefined, false, false)).toBeNull();
    expect(syncNotice(undefined, true, false)).toEqual({ kind: "fallback", loading: false });
  });

  it("shows nothing in sync, and nothing for a deleted session", () => {
    expect(syncNotice(sync({ phase: "inSync", remoteRev: 41 }), false, false)).toBeNull();
    expect(syncNotice(sync({ gone: true }), false, false)).toBeNull();
  });

  it("counts the revisions behind, the number a refusal reports", () => {
    expect(syncNotice(sync(), false, false)).toEqual({ kind: "behind", count: 2 });
  });

  it("names the retry delay after a failed pull", () => {
    const failed = sync({ retry: { attempt: 3, delayMs: 5_000 } });
    expect(syncNotice(failed, false, false)).toEqual({ kind: "retry", count: 2, seconds: 5 });
  });

  it("says syncing only while it reads turns, not on a silent own pull", () => {
    expect(syncNotice(sync({ phase: "syncing" }), false, false)).toEqual({ kind: "syncing", count: 2 });
    expect(syncNotice(sync({ phase: "syncing", remoteRev: 41 }), false, false)).toBeNull();
  });

  it("keeps the guard card through a pull started from it", () => {
    const guard = syncNotice(sync({ phase: "guard", draft: true }), false, false);
    expect(guard).toEqual({ kind: "guard", count: 2, device: "jons-macbook-pro", syncing: false });
    const pulling = syncNotice(sync({ phase: "syncing", draft: true }), false, false);
    expect(pulling).toEqual({ kind: "guard", count: 2, device: "jons-macbook-pro", syncing: true });
  });

  it("falls back to a capitalized device when a refusal named none", () => {
    const anon = syncNotice(sync({ phase: "guard", draft: true, authorDevice: null }), false, false);
    expect(anon).toMatchObject({ kind: "guard", device: "Another device" });
  });

  it("reports the device a finished pull synced with", () => {
    const done = sync({ phase: "inSync", localRev: 43, syncedWith: "jons-macbook-pro" });
    expect(syncNotice(done, false, false)).toEqual({ kind: "synced", device: "jons-macbook-pro" });
  });
});

describe("syncNotice on an ended view", () => {
  it("still offers Load when another device moved a no-sync session on", () => {
    expect(syncNotice(undefined, true, false, true)).toEqual({ kind: "fallback", loading: false });
  });

  it("shows no sync notice, so a deleted or behind session leaves only the ended composer", () => {
    expect(syncNotice(undefined, false, false, true)).toBeNull();
    expect(syncNotice(sync({ gone: true }), false, false, true)).toBeNull();
    expect(syncNotice(sync(), false, false, true)).toBeNull();
    expect(syncNotice(sync({ phase: "guard", draft: true }), false, false, true)).toBeNull();
  });

  it("renders Load on a drifted ended view", () => {
    const handlers = { onPull: vi.fn(), onSendAnyway: vi.fn(), onLoad: vi.fn(), onDismiss: vi.fn() };
    render(<SyncComposerNotice notice={syncNotice(undefined, true, false, true)} {...handlers} />);
    expect(status().textContent).toContain("New turns from another device");
    fireEvent.click(screen.getByRole("button", { name: "Load" }));
    expect(handlers.onLoad).toHaveBeenCalledTimes(1);
  });
});

describe("SyncComposerNotice", () => {
  it("keeps its live region mounted while empty", () => {
    renderNotice(sync({ phase: "inSync", remoteRev: 41 }));
    expect(status().getAttribute("aria-live")).toBe("polite");
    expect(status().textContent).toBe("");
  });

  it("pluralizes the waiting line and pulls on now", () => {
    const one = renderNotice(sync({ remoteRev: 42 }));
    expect(status().textContent).toContain("1 turn waiting · pulls when you're idle");
    fireEvent.click(screen.getByRole("button", { name: "now" }));
    expect(one.onPull).toHaveBeenCalledTimes(1);
  });

  it("renders the guard copy with both actions as buttons", () => {
    const handlers = renderNotice(sync({ phase: "guard", draft: true }));
    expect(status().textContent).toContain("This session moved on from another client");
    expect(status().textContent).toContain(
      "jons-macbook-pro added 2 turns while you were drafting. Pull them in first so your message answers the latest state. Your draft stays.",
    );
    expect(status().textContent).toContain("draft kept · ⏎ send");
    fireEvent.click(screen.getByRole("button", { name: "Pull 2 turns" }));
    fireEvent.click(screen.getByRole("button", { name: "Send anyway" }));
    expect(handlers.onPull).toHaveBeenCalledTimes(1);
    expect(handlers.onSendAnyway).toHaveBeenCalledTimes(1);
  });

  it("holds Pull while a pull runs, but never Send anyway", () => {
    renderNotice(sync({ phase: "syncing", draft: true }));
    expect((screen.getByRole("button", { name: "Pull 2 turns" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Send anyway" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("loads and dismisses the fallback notice", () => {
    const handlers = renderNotice(undefined, true);
    expect(status().textContent).toContain("New turns from another device");
    fireEvent.click(screen.getByRole("button", { name: "Load" }));
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(handlers.onLoad).toHaveBeenCalledTimes(1);
    expect(handlers.onDismiss).toHaveBeenCalledTimes(1);
  });

  it("shows the fallback's loading state without its actions", () => {
    renderNotice(undefined, true, true);
    expect(status().textContent).toContain("Loading turns…");
    expect(screen.queryByRole("button", { name: "Load" })).toBeNull();
  });
});

