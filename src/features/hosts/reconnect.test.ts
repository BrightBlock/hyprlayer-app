// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppError, ConnectionStatus } from "@/lib/bindings";

const b = vi.hoisted(() => ({
  reconnectServer: vi.fn(),
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock("@/lib/bindings", () => ({
  commands: { reconnectServer: b.reconnectServer },
  events: { serverDisconnected: { listen: b.listen } },
}));

const q = vi.hoisted(() => ({ invalidateQueries: vi.fn() }));
vi.mock("@/lib/queryClient", () => ({ queryClient: q }));

const CONNECTED: ConnectionStatus = {
  state: "connected",
  host: null,
  serverVersion: 1,
  error: null,
};

const ok = () => ({ status: "ok", data: CONNECTED });
const fails = (error: AppError) => ({ status: "error", error });
const DOWN: AppError = {
  code: "server_disconnected",
  message: "The connection ended.",
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
// `focusManager` is a node_modules singleton that survives resetModules, so a
// bridge left subscribed would answer the *next* case's focus too.
let detach: Array<() => void> = [];

async function subscribeBridge(): Promise<void> {
  detach.push(...(await Promise.all(driver.reconnectBridge())));
}

beforeEach(async () => {
  vi.useFakeTimers();
  // Module-level loop state, so each case gets its own copy.
  vi.resetModules();
  driver = await import("./reconnect");
  ({ focusManager } = await import("@tanstack/react-query"));
  ({ useToast } = await import("@/lib/toast"));
});

afterEach(() => {
  for (const off of detach) off();
  detach = [];
  vi.useRealTimers();
  vi.clearAllMocks();
});

function refocus(): void {
  focusManager.setFocused(false);
  focusManager.setFocused(true);
}

describe("reconnect", () => {
  it("retries at 1s, 2s, 5s, 15s and then every 30s, without ever giving up", async () => {
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    driver.startReconnecting();

    await vi.advanceTimersByTimeAsync(999);
    expect(b.reconnectServer).toHaveBeenCalledTimes(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);

    for (const delay of [2_000, 5_000, 15_000]) {
      await vi.advanceTimersByTimeAsync(delay - 1);
      const before = b.reconnectServer.mock.calls.length;
      await vi.advanceTimersByTimeAsync(1);
      expect(b.reconnectServer).toHaveBeenCalledTimes(before + 1);
    }
    expect(b.reconnectServer).toHaveBeenCalledTimes(4);

    // Past the fourth step the interval is capped, and past the fifth attempt
    // the loop keeps going — a relaunch is not the fallback for a host that
    // stays down.
    for (let i = 0; i < 4; i++) await vi.advanceTimersByTimeAsync(30_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(8);
  });

  it("stops on the first success and invalidates every query", async () => {
    b.reconnectServer.mockResolvedValue(ok());
    driver.startReconnecting();

    await vi.advanceTimersByTimeAsync(1_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
    expect(q.invalidateQueries).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(120_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
  });

  it("retries the moment the window regains focus instead of waiting out the interval", async () => {
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    await subscribeBridge();
    driver.startReconnecting();
    await vi.advanceTimersByTimeAsync(0);

    refocus();
    await vi.advanceTimersByTimeAsync(0);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
  });

  it("starts no second attempt when focus arrives while one is in flight", async () => {
    let finish!: (value: unknown) => void;
    b.reconnectServer.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));
    b.reconnectServer.mockResolvedValue(fails(DOWN));
    await subscribeBridge();
    driver.startReconnecting();

    await vi.advanceTimersByTimeAsync(1_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);

    refocus();
    await vi.advanceTimersByTimeAsync(0);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);

    finish(fails(DOWN));
    await vi.advanceTimersByTimeAsync(2_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(2);
  });

  it("stops on a protocol mismatch and surfaces its remedy", async () => {
    b.reconnectServer.mockResolvedValue(fails(MISMATCH));
    driver.startReconnecting();

    await vi.advanceTimersByTimeAsync(1_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
    const toast = useToast.getState();
    expect(toast.visible).toBe(true);
    expect(toast.title).toBe(MISMATCH.message);
    expect(toast.message).toBe(MISMATCH.remedy);

    // Disagreeing binaries cannot resolve themselves, so this is the one
    // failure the loop does not keep trying.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(b.reconnectServer).toHaveBeenCalledTimes(1);
  });
});
