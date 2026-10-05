import { create } from "zustand";
import { persist } from "zustand/middleware";
import { getVersion } from "@tauri-apps/api/app";

import { commands, type ConnectionStatus, type Host, type Session } from "@/lib/bindings";
import { onServerReconnected } from "@/lib/events";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { useToast } from "@/lib/toast";
import type { AgentState } from "@/features/multiplexer/engineState";
import { useMultiplexerStore } from "@/features/multiplexer/multiplexerStore";
import { cancelPlannedRestart, planRestart } from "./plannedRestart";
import { retryConnection } from "./reconnect";
import { useReconnectStore } from "./reconnectState";
import { runUpdate, SERVER_OUTDATED } from "./serverInstall";

export const IDLE_MS = 2 * 60_000;
const POLL_MS = 60_000;
// A server that comes back still outdated must not be restarted every poll.
const RETRY_MS = 30 * 60_000;

type AutoUpdate = { enabled: boolean; updating: boolean };

// Machine-local: an outdated server is the one that would have to store it.
export const useServerAutoUpdate = create<AutoUpdate>()(
  persist((): AutoUpdate => ({ enabled: true, updating: false }), {
    name: "hyprlayer:server-auto-update",
    version: 1,
    partialize: (s) => ({ enabled: s.enabled }),
  }),
);

export function setServerAutoUpdate(enabled: boolean): void {
  useServerAutoUpdate.setState({ enabled });
}

export type IdleInputs = {
  sessions: Session[];
  runningScripts: number;
  agents: AgentState[];
  quietMs: number;
};

// The awaiting states hold a prompt the restart would drop.
const HELD_STATES = new Set<Session["state"]>(["starting", "awaiting_permission", "awaiting_question"]);

export function isServerIdle({ sessions, runningScripts, agents, quietMs }: IdleInputs): boolean {
  return (
    quietMs >= IDLE_MS &&
    runningScripts === 0 &&
    !agents.some((a) => a === "working" || a === "blocked") &&
    !sessions.some((s) => s.working === true || HELD_STATES.has(s.state))
  );
}

/** `serverOutdated` also flags a different dev build; only a release pair is a real update. */
export function isReleasePair(serverRelease: string | null, appVersion: string): boolean {
  if (!serverRelease || !appVersion) return false;
  return !serverRelease.includes("+") && !appVersion.includes("+");
}

let lastActivityAt = Date.now();

/** User input or shell output: the only signal for a command running in a plain terminal. */
export function noteActivity(): void {
  lastActivityAt = Date.now();
}

let idleSince: number | null = null;
let lastTickAt = Date.now();
let retryAfter = 0;
let announce: string | null = null;
let appVersion = "";

async function idleNow(): Promise<boolean> {
  const [active, scripts] = await Promise.all([
    unwrap(commands.listActiveSessions()),
    unwrap(commands.listRunningScripts()),
  ]);
  return isServerIdle({
    sessions: active.map((a) => a.session),
    runningScripts: scripts.length,
    agents: Object.values(useMultiplexerStore.getState().agents).map((a) => a.state),
    quietMs: Date.now() - lastActivityAt,
  });
}

async function tick(): Promise<void> {
  const reconnect = useReconnectStore.getState();
  // Intervals stall while the machine sleeps; a lull spanning that gap was never observed.
  if (Date.now() - lastTickAt > 2 * POLL_MS) idleSince = null;
  lastTickAt = Date.now();
  if (useServerAutoUpdate.getState().updating || Date.now() < retryAfter) return;
  if (!useServerAutoUpdate.getState().enabled || reconnect.lostAt !== null || reconnect.attempting) {
    idleSince = null;
    return;
  }
  const status = await unwrap(commands.connectionStatus());
  if (status.state !== "connected") return updateRefusedHost(status);
  if (
    !status.serverOutdated ||
    !isReleasePair(status.serverRelease, appVersion) ||
    !(await idleNow())
  ) {
    idleSince = null;
    return;
  }
  idleSince ??= Date.now();
  // A session between turns reads idle for a moment; only a sustained lull counts.
  if (Date.now() - idleSince < POLL_MS / 2) return;
  await update(status.host, status.serverRelease, true);
}

/** A host below this app's minimum leaves the app unusable, so it updates as soon as it idles. */
async function updateRefusedHost(status: ConnectionStatus): Promise<void> {
  if (!status.host || status.error?.code !== SERVER_OUTDATED) return;
  const activity = await unwrap(commands.inspectOutdatedHost(status.host));
  if (!isReleasePair(activity.serverRelease, appVersion)) return;
  const idle = isServerIdle({
    sessions: activity.sessions,
    runningScripts: activity.runningScripts.length,
    // Never connected, so nothing on this client can be busy.
    agents: [],
    quietMs: IDLE_MS,
  });
  if (idle) await update(status.host, activity.serverRelease, false);
}

async function update(host: Host | null, from: string | null, connected: boolean): Promise<void> {
  useServerAutoUpdate.setState({ updating: true });
  retryAfter = Date.now() + RETRY_MS;
  const where = host ? host.label || host.destination : "this machine";
  log.info("server-auto-update", `idle; updating the server on ${where} from ${from ?? "unknown"}`);
  try {
    announce = where;
    planRestart();
    // A live connection drops when the server stops, and the reconnect loop starts this app's build.
    if (host) await runUpdate(host, () => {});
    else await unwrap(commands.restartServer());
    if (!connected) await retryConnection();
  } catch (e) {
    announce = null;
    cancelPlannedRestart();
    log.warn("server-auto-update", "update failed", e);
  } finally {
    useServerAutoUpdate.setState({ updating: false });
    idleSince = null;
  }
}

/** Updates an outdated server once nothing is running on it and the user has stepped away. */
export async function startServerAutoUpdate(): Promise<void> {
  // Debug builds flag the server outdated on every rebuild; restarting it would kill the dev server.
  if (import.meta.env.DEV && !import.meta.env.VITE_AUTO_UPDATE_SERVER) return;
  for (const type of ["keydown", "pointerdown", "wheel"]) {
    window.addEventListener(type, noteActivity, { capture: true, passive: true });
  }
  onServerReconnected(() => {
    if (announce === null) return;
    useToast.getState().show("Server updated", `Updated the server on ${announce}.`);
    announce = null;
  });
  appVersion = await getVersion().catch(() => "");
  const run = () =>
    tick().catch((e) => {
      idleSince = null;
      log.info("server-auto-update", "idle check failed", e);
    });
  void run();
  window.setInterval(run, POLL_MS);
}
