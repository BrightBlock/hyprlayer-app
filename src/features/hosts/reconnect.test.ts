// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppError, ConnectionStatus } from "@/lib/bindings";

const b = vi.hoisted(() => ({
  reconnectServer: vi.fn(),
  paceReconnect: vi.fn(async (_on: boolean) => ({ status: "ok", data: null })),
  listen: vi.fn(() => Promise.resolve(() => {})),
  due: null as (() => void) | null,
}));
vi.mock("@/lib/bindings", () => ({
  commands: { reconnectServer: b.reconnectServer, paceReconnect: b.paceReconnect },
  events: {
    serverDisconnected: { listen: b.listen },
    reconnectDue: {
      listen: (cb: () => void) => {
        b.due = cb;
        return Promise.resolve(() => (b.due = null));
      },
    },
  },
}));

const q = vi.hoisted(() => ({ invalidateQueries: vi.fn() }));
vi.mock("@/lib/queryClient", () => ({ queryClient: q }));

const CONNECTED: ConnectionStatus = {
  state: "connected",
  host: null,
  serverVersion: 1,
  serverRelease: null, serverOutdated: false, transcriptSync: false,
  error: null,
};

const ok = () => ({ status: "ok", data: CONNECTED });
const fails = (error: AppError) => ({ status: "error", error });
const DOWN: AppError = {
  code: "server_disconnected",
  message: "The connection ended.",
  remedy: null,
};
const CANCELLED: AppError = {
  code: "remote_sign_in_cancelled",
  message: "Sign-in was cancelled.",
  remedy: null,
};
const MISMATCH: AppError = {
  code: "protocol_version_mismatch",
  message: "The server speaks protocol 2; this app speaks 1.",
  remedy: "Update hyprlayer-server on Nexus.",
};

type Driver = typeof import("./reconnect");
let driver: Driver;
let focusManager: typeof import("@tanstack/react-query").focusManager;
let useToast: typeof import("@/lib/toast").useToast;
let useReconnectStore: typeof import("./reconnectState").useReconnectStore;
// `focusManager` is a node_modules singleton that survives resetModules, so a
// bridge left subscribed would answer the *next* case's focus too.
let detach: Array<() => void> = [];

beforeEach(async () => {
  vi.useFakeTimers();
  // Module-level loop state, so each case gets its own copy.
  vi.resetModules();
  driver = await import("./reconnect");
  ({ focusManager } = await import("@tanstack/react-query"));
  ({ useToast } = await import("@/lib/toast"));
  ({ useReconnectStore } = await import("./reconnectState"));
  detach = await Promise.all(driver.reconnectBridge());
});

afterEach(() => {
  for (const off of detach) off();
  detach = [];
  vi.useRealTimers();
  vi.clearAllMocks();
});

// The shell's pacing event; no webview timer is advanced, as in a hidden window.
async function due(): Promise<void> {
  b.due?.();
  await vi.advanceTimersByTimeAsync(0);
}

function refocus(): void {
  focusManager.setFocused(false);
  focusManager.setFocused(true);
}

describe("reconnect", () => {
  it("retries on each pacing event from the shell, and never on a webview timer", async () => {
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    driver.startReconnecting();

    await vi.advanceTimersByTimeAsync(3_600_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(0);

    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
    // A host that stays down is never given up on: a relaunch is not the fallback.
    for (let i = 0; i < 7; i++) await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(8);
  });

  it("stops on the first success and invalidates every query", async () => {
    b.reconnectServer.mockResolvedValue(ok());
    driver.startReconnecting();

    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
    expect(q.invalidateQueries).toHaveBeenCalledTimes(1);

    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
  });

  it("retries the moment the window regains focus instead of waiting for the next pacing event", async () => {
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    driver.startReconnecting();
    await vi.advanceTimersByTimeAsync(0);

    refocus();
    await vi.advanceTimersByTimeAsync(0);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
  });

  it("starts no second attempt when a pacing event or focus arrives while one is in flight", async () => {
    let finish!: (value: unknown) => void;
    b.reconnectServer.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    driver.startReconnecting();

    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);

    await due();
    refocus();
    await vi.advanceTimersByTimeAsync(0);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);

    finish(fails(DOWN));
    await vi.advanceTimersByTimeAsync(0);
    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(2);
  });

  it("stops on a protocol mismatch and surfaces its remedy", async () => {
    b.reconnectServer.mockResolvedValue(fails(MISMATCH));
    driver.startReconnecting();

    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
    const toast = useToast.getState();
    expect(toast.visible).toBe(true);
    expect(toast.title).toBe(MISMATCH.message);
    expect(toast.message).toBe(MISMATCH.remedy);

    // Disagreeing binaries cannot resolve themselves, so this is the one
    // failure the loop does not keep trying.
    await due();
    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
  });

  it("starts the shell's schedule over whenever the loop starts, so a resumed loop retries in about 1s", async () => {
    b.reconnectServer.mockResolvedValue(fails(CANCELLED));
    driver.startReconnecting();
    expect(b.paceReconnect.mock.calls).toEqual([[true]]);

    await due();
    expect(useReconnectStore.getState().halted).toBe("signIn");

    // A Sign in that failed some other way resumes dialling.
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    await driver.retryConnection();
    expect(b.paceReconnect.mock.calls).toEqual([[true], [false], [true]]);
  });

  it("stops the shell's pacing while the loop is halted", async () => {
    b.reconnectServer.mockResolvedValue(fails(MISMATCH));
    driver.startReconnecting();

    await due();
    expect(useReconnectStore.getState().halted).toBe("protocol");
    expect(b.paceReconnect.mock.calls).toEqual([[true], [false]]);
  });

  it("adopts a connection that works now, rather than leaving a stale failure up", async () => {
    b.reconnectServer.mockResolvedValue(ok());

    await driver.retryConnection();

    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
    expect(q.invalidateQueries).toHaveBeenCalledTimes(1);
  });

  it("refetches only the reported status when the retry does not connect", async () => {
    b.reconnectServer.mockResolvedValue(fails(DOWN));

    // Swallowed: the caller is a probe or an install, neither of which owns the
    // connection surface's error.
    await expect(driver.retryConnection()).resolves.toBeUndefined();
    expect(q.invalidateQueries).toHaveBeenCalledTimes(1);
    expect(q.invalidateQueries).toHaveBeenCalledWith({ queryKey: ["connection-status"] });
  });

  it("wakes the retry loop instead of dialling beside it", async () => {
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    driver.startReconnecting();
    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);

    // Mid-loop: the loop dials once; a second dialler beside it would make two.
    await driver.retryConnection();
    await vi.advanceTimersByTimeAsync(0);
    expect(b.reconnectServer).toHaveBeenCalledTimes(2);

    await due();
    expect(b.reconnectServer).toHaveBeenCalledTimes(3);
  });
});

describe("reconnect state", () => {
  const DROP = Date.UTC(2026, 8, 24, 12, 0, 0);

  it("records the drop time when the loop starts, and a new drop clears an old halt", () => {
    vi.setSystemTime(DROP);
    useReconnectStore.setState({ halted: "protocol" });
    b.reconnectServer.mockResolvedValue(fails(DOWN));

    driver.startReconnecting();

    expect(useReconnectStore.getState()).toEqual({
      lostAt: DROP,
      attempting: false,
      halted: null,
      keyChanged: null,
    });
  });

  it("marks an attempt only while its call is pending", async () => {
    let finish!: (value: unknown) => void;
    b.reconnectServer.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));
    driver.startReconnecting();

    await vi.advanceTimersByTimeAsync(0);
    expect(useReconnectStore.getState().attempting).toBe(false);
    await due();
    expect(useReconnectStore.getState().attempting).toBe(true);

    finish(fails(DOWN));
    await vi.advanceTimersByTimeAsync(0);
    expect(useReconnectStore.getState().attempting).toBe(false);
  });

  it("marks a one-off retry while it is pending", async () => {
    let finish!: (value: unknown) => void;
    b.reconnectServer.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));

    const retry = driver.retryConnection();
    expect(useReconnectStore.getState().attempting).toBe(true);

    finish(fails(DOWN));
    await retry;
    expect(useReconnectStore.getState().attempting).toBe(false);
  });

  it("keeps the drop time through the post-reconnect refetch, then clears it", async () => {
    vi.setSystemTime(DROP);
    let refetched!: () => void;
    q.invalidateQueries.mockReturnValueOnce(new Promise<void>((resolve) => (refetched = resolve)));
    b.reconnectServer.mockResolvedValue(ok());
    driver.startReconnecting();

    await due();
    // The status query still reads `lost` until the refetch lands.
    expect(useReconnectStore.getState().lostAt).toBe(DROP);

    refetched();
    await vi.advanceTimersByTimeAsync(0);
    expect(useReconnectStore.getState()).toEqual({
      lostAt: null,
      attempting: false,
      halted: null,
      keyChanged: null,
    });
  });

  it("halts on a protocol mismatch and keeps the drop time", async () => {
    vi.setSystemTime(DROP);
    b.reconnectServer.mockResolvedValue(fails(MISMATCH));
    driver.startReconnecting();

    await due();
    expect(useReconnectStore.getState()).toEqual({
      lostAt: DROP,
      attempting: false,
      halted: "protocol",
      keyChanged: null,
    });
  });

  it("clears a halted drop once a retry connects", async () => {
    b.reconnectServer.mockResolvedValue(fails(MISMATCH));
    driver.startReconnecting();
    await due();
    expect(useReconnectStore.getState().halted).toBe("protocol");

    b.reconnectServer.mockResolvedValue(ok());
    await driver.retryConnection();

    expect(useReconnectStore.getState()).toEqual({
      lostAt: null,
      attempting: false,
      halted: null,
      keyChanged: null,
    });
  });

  it("leaves the drop alone when a retry does not connect", async () => {
    vi.setSystemTime(DROP);
    b.reconnectServer.mockResolvedValue(fails(MISMATCH));
    driver.startReconnecting();
    await due();

    b.reconnectServer.mockResolvedValue(fails(DOWN));
    await driver.retryConnection();

    expect(useReconnectStore.getState()).toEqual({
      lostAt: DROP,
      attempting: false,
      halted: "protocol",
      keyChanged: null,
    });
  });
});
