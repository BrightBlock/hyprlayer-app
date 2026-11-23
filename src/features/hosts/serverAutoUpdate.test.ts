// @vitest-environment jsdom
// Failure modes of a quiet server update, written before the code (1-6 and 19-22 here;
// 7-8 and 15 in HostsDialog.test.tsx; 9-10 in statusBarState.test.ts; 10-12 and 16 in
// StatusBar.test.tsx; 13-14 in hosts.test.ts and ServersSettings.test.tsx; 17-18 in
// ServerDisconnectedNotice.test.tsx):
//  1. `updating` clears on the reconnect event, before its status refetch lands, so the
//     bar flashes the stale `lost` status.
//  2. `updating` clears before the drop is observed (the call returns, the disconnect lands later).
//  3. `updating` never clears when the new server never comes back.
//  4. `updating` hides a halted loop or a changed host key, which only the user can clear.
//  5. `updating` sticks after the restart or install call fails, or that failure is swallowed.
//  6. A second update starts while one is in flight.
//  7. The server dialog renders while an update's planned drop reconnects.
//  8. A refused (too old) host's dialog vanishes during its update, leaving no server behind the app.
//  9. The bar stops reading "connected" while connected.
// 10. The bar reads "offline" or "retrying" through an update's planned drop.
// 11. An outdated server offers no update action, or offers it while updating.
// 12. The update action runs without the confirm that names what it stops.
// 13. A server row omits the version its live connection or last probe reported.
// 14. A server row shows a version for a host it cannot reach.
// 15. A dropped connection opens the server dialog instead of leaving it to the toast and the bar.
// 16. While the connection is down, the bar's server item leads nowhere.
// 17. A changed host key, which only the user can clear, surfaces nowhere without the dialog.
// 18. An update whose server never comes back goes quiet once the bar stops reading "updating".
// 19. Untouched since launch with nothing running, the update still waits minutes, so work starts first.
// 20. The launch path skips its re-check and restarts a session caught between two queued turns.
// 21. The launch path outlives input or terminal output, restarting under someone already working.
// 22. The launch path updates a remote host, whose install outlasts the moment nobody is working.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppError, Session } from "@/lib/bindings";
import { IDLE_MS, isReleasePair, isServerIdle, type IdleInputs } from "./serverAutoUpdate";

const b = vi.hoisted(() => ({
  restartServer: vi.fn(),
  connectionStatus: vi.fn(),
  listActiveSessions: vi.fn(),
  listRunningScripts: vi.fn(),
}));
vi.mock("@/lib/bindings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/bindings")>();
  return { ...actual, commands: { ...actual.commands, ...b } };
});
const install = vi.hoisted(() => ({ runUpdate: vi.fn(() => Promise.resolve()) }));
vi.mock("./serverInstall", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./serverInstall")>()),
  runUpdate: install.runUpdate,
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: () => Promise.resolve("0.8.12") }));

const session = (patch: Partial<Session>): Session => ({
  id: "s",
  workstreamId: "w",
  stage: "implement",
  harness: "claude",
  permissionMode: "default",
  claudeSessionId: null,
  cwd: "/tmp",
  state: "running",
  model: null,
  costUsd: null,
  startedAt: null,
  endedAt: null,
  exitSummary: null,
  working: false,
  ...patch,
});

const QUIET: IdleInputs = { sessions: [], runningScripts: 0, agents: [], quietMs: IDLE_MS };

describe("isServerIdle", () => {
  it("is idle with nothing running and no recent input", () => {
    expect(isServerIdle(QUIET)).toBe(true);
  });

  it("treats a live harness between turns as idle", () => {
    expect(isServerIdle({ ...QUIET, sessions: [session({ state: "running", working: false })] })).toBe(true);
  });

  it("is busy while a session is mid-turn", () => {
    expect(isServerIdle({ ...QUIET, sessions: [session({ working: true })] })).toBe(false);
  });

  it.each(["starting", "awaiting_permission", "awaiting_question"] as const)(
    "is busy while a session is %s",
    (state) => {
      expect(isServerIdle({ ...QUIET, sessions: [session({ state, working: false })] })).toBe(false);
    },
  );

  it("is busy while a script runs", () => {
    expect(isServerIdle({ ...QUIET, runningScripts: 1 })).toBe(false);
  });

  it.each(["working", "blocked"] as const)("is busy while a terminal agent is %s", (state) => {
    expect(isServerIdle({ ...QUIET, agents: ["idle", state] })).toBe(false);
  });

  it("is busy while the user has touched the app recently", () => {
    expect(isServerIdle({ ...QUIET, quietMs: IDLE_MS - 1 })).toBe(false);
  });
});

describe("isReleasePair", () => {
  it("allows a release app over a release server", () => {
    expect(isReleasePair("0.8.6", "0.40.0")).toBe(true);
  });

  it("never replaces a dev-installed server", () => {
    expect(isReleasePair("0.8.7+dev.abc123", "0.40.0")).toBe(false);
  });

  it("never pushes a dev-installed app's server", () => {
    expect(isReleasePair("0.8.6", "0.40.0+dev.abc123")).toBe(false);
  });

  it("holds off until both versions are known", () => {
    expect(isReleasePair(null, "0.40.0")).toBe(false);
    expect(isReleasePair("0.8.6", "")).toBe(false);
  });
});

const KEY_CHANGED: AppError = {
  code: "remote_host_key_changed",
  message: "Nexus presented a different host key.",
  remedy: null,
};

describe("updateServer", () => {
  let update: typeof import("./serverAutoUpdate");
  let reconnect: typeof import("./reconnectState").useReconnectStore;
  let notifyServerReconnected: () => void;

  const updating = () => update.useServerAutoUpdate.getState().updating;
  const drop = () => reconnect.setState({ lostAt: Date.now() });
  // The reconnect loop clears `lostAt` only once its status refetch has landed.
  const refetched = () => reconnect.setState({ lostAt: null });

  beforeEach(async () => {
    vi.useFakeTimers();
    // Module-level update state, so each case gets its own copy.
    vi.resetModules();
    update = await import("./serverAutoUpdate");
    reconnect = (await import("./reconnectState")).useReconnectStore;
    ({ notifyServerReconnected } = await import("@/lib/events"));
    b.restartServer.mockResolvedValue({ status: "ok", data: null });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("holds `updating` past the reconnect event until its status refetch lands", async () => {
    const run = update.updateServer(null, "0.8.10");
    await vi.advanceTimersByTimeAsync(0);
    expect(updating()).toBe(true);
    drop();
    notifyServerReconnected();
    expect(updating()).toBe(true);
    refetched();
    await run;
    expect(updating()).toBe(false);
  });

  it("waits for a drop that lands after the call returns", async () => {
    const run = update.updateServer(null, "0.8.10");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(updating()).toBe(true);
    drop();
    expect(updating()).toBe(true);
    refetched();
    await run;
    expect(updating()).toBe(false);
  });

  it("counts a drop that lands before the call returns", async () => {
    b.restartServer.mockImplementation(async () => {
      drop();
      return { status: "ok", data: null };
    });
    const run = update.updateServer(null, "0.8.10");
    await vi.advanceTimersByTimeAsync(0);
    expect(updating()).toBe(true);
    refetched();
    await run;
    expect(updating()).toBe(false);
  });

  it("hands a server that never comes back to the server dialog", async () => {
    const run = update.updateServer(null, "0.8.10");
    await vi.advanceTimersByTimeAsync(0);
    drop();
    await vi.advanceTimersByTimeAsync(update.SETTLE_MS - 1);
    expect(updating()).toBe(true);
    await vi.advanceTimersByTimeAsync(1);
    await run;
    expect(updating()).toBe(false);
  });

  it.each([
    ["a halted loop", { halted: "protocol" as const }],
    ["a changed host key", { keyChanged: KEY_CHANGED }],
  ])("lets go at once for %s, which only the user can clear", async (_, patch) => {
    const run = update.updateServer(null, "0.8.10");
    await vi.advanceTimersByTimeAsync(0);
    drop();
    reconnect.setState(patch);
    await run;
    expect(updating()).toBe(false);
  });

  it("clears `updating` and rethrows when the restart fails", async () => {
    b.restartServer.mockResolvedValue({
      status: "error",
      error: { code: "server_unavailable", message: "The server would not stop.", remedy: null },
    });
    await expect(update.updateServer(null, "0.8.10")).rejects.toMatchObject({
      code: "server_unavailable",
    });
    expect(updating()).toBe(false);
  });

  it("never starts a second update while one runs", async () => {
    const first = update.updateServer(null, "0.8.10");
    await update.updateServer(null, "0.8.10");
    expect(b.restartServer).toHaveBeenCalledTimes(1);
    drop();
    refetched();
    await first;
  });
});

describe("startServerAutoUpdate", () => {
  let update: typeof import("./serverAutoUpdate");
  const ok = <T,>(data: T) => ({ status: "ok", data });
  const outdated = {
    state: "connected",
    host: null,
    serverVersion: 1,
    serverRelease: "0.8.10",
    serverOutdated: true,
    transcriptSync: false,
    error: null,
  };

  beforeEach(async () => {
    vi.useFakeTimers();
    vi.stubEnv("VITE_AUTO_UPDATE_SERVER", "1");
    vi.resetModules();
    update = await import("./serverAutoUpdate");
    b.connectionStatus.mockResolvedValue(ok(outdated));
    b.listActiveSessions.mockResolvedValue(ok([]));
    b.listRunningScripts.mockResolvedValue(ok([]));
    b.restartServer.mockResolvedValue(ok(null));
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllEnvs();
    vi.clearAllMocks();
  });

  it("updates within seconds of launch when nothing runs and nobody has touched the app", async () => {
    await update.startServerAutoUpdate();
    await vi.advanceTimersByTimeAsync(update.LAUNCH_SETTLE_MS - 1);
    expect(b.restartServer).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(b.restartServer).toHaveBeenCalledTimes(1);
  });

  it("re-checks before updating, sparing a session caught between two queued turns", async () => {
    await update.startServerAutoUpdate();
    await vi.advanceTimersByTimeAsync(0);
    expect(b.listActiveSessions).toHaveBeenCalledTimes(1);
    expect(b.restartServer).not.toHaveBeenCalled();
    b.listActiveSessions.mockResolvedValue(ok([{ session: session({ working: true }) }]));
    await vi.advanceTimersByTimeAsync(update.LAUNCH_SETTLE_MS);
    expect(b.listActiveSessions).toHaveBeenCalledTimes(2);
    expect(b.restartServer).not.toHaveBeenCalled();
  });

  it("leaves a remote host to the long quiet window, since its install takes far longer", async () => {
    b.connectionStatus.mockResolvedValue(ok({ ...outdated, host: { destination: "nexus", label: "Nexus" } }));
    await update.startServerAutoUpdate();
    await vi.advanceTimersByTimeAsync(update.LAUNCH_SETTLE_MS * 5);
    expect(install.runUpdate).not.toHaveBeenCalled();
  });

  it.each([
    ["input", () => window.dispatchEvent(new KeyboardEvent("keydown"))],
    ["terminal output", () => update.noteActivity()],
  ])("waits for the long quiet window once %s arrives after launch", async (_, act) => {
    await update.startServerAutoUpdate();
    act();
    await vi.advanceTimersByTimeAsync(update.LAUNCH_SETTLE_MS * 5);
    expect(b.restartServer).not.toHaveBeenCalled();
  });
});
