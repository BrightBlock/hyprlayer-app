import { create } from "zustand";
import { persist } from "zustand/middleware";
import { getVersion } from "@tauri-apps/api/app";

import { commands, type ConnectionStatus, type Host, type Session } from "@/lib/bindings";
import { hostName } from "@/lib/domain/hosts";
import { onServerReconnected } from "@/lib/events";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { useToast } from "@/lib/toast";
import type { AgentState } from "@/features/multiplexer/engineState";
import { useMultiplexerStore } from "@/features/multiplexer/multiplexerStore";
import { cancelPlannedRestart, planRestart } from "./plannedRestart";
import { retryConnection } from "./reconnect";
import { useReconnectStore, type ReconnectSnapshot } from "./reconnectState";
import { runUpdate, SERVER_OUTDATED } from "./serverInstall";

export const IDLE_MS = 2 * 60_000;
const POLL_MS = 60_000;
// Untouched since launch, nobody is working yet; a lull need only outlast a gap between turns.
// Local only: a host's download and upload outlast that moment.
export const LAUNCH_SETTLE_MS = 2_000;
// A server that comes back still outdated must not be restarted every poll.
const RETRY_MS = 30 * 60_000;
// A server not back by then is an outage, and the bar and toast say so.
export const SETTLE_MS = 30_000;

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
let activeSinceLaunch = false;

/** User input or shell output: the only signal for a command running in a plain terminal. */
export function noteActivity(): void {
  lastActivityAt = Date.now();
  activeSinceLaunch = true;
}

let idleSince: number | null = null;
let lastTickAt = Date.now();
let retryAfter = 0;
let announce: string | null = null;
let appVersion = "";

onServerReconnected(() => {
  if (announce === null) return;
  useToast.getState().show("Server updated", `Updated the server on ${announce}.`);
  announce = null;
});

async function idleNow(fresh: boolean): Promise<boolean> {
  const [active, scripts] = await Promise.all([
    unwrap(commands.listActiveSessions()),
    unwrap(commands.listRunningScripts()),
  ]);
  return isServerIdle({
    sessions: active.map((a) => a.session),
    runningScripts: scripts.length,
    agents: Object.values(useMultiplexerStore.getState().agents).map((a) => a.state),
    quietMs: fresh ? IDLE_MS : Date.now() - lastActivityAt,
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
  const fresh = !activeSinceLaunch && status.host === null;
  if (
    !status.serverOutdated ||
    !isReleasePair(status.serverRelease, appVersion) ||
    !(await idleNow(fresh))
  ) {
    idleSince = null;
    return;
  }
  idleSince ??= Date.now();
  // A session between turns reads idle for a moment; only a sustained lull counts.
  const settle = fresh ? LAUNCH_SETTLE_MS : POLL_MS / 2;
  if (Date.now() - idleSince < settle) {
    if (fresh) window.setTimeout(run, settle);
    return;
  }
  log.info("server-auto-update", fresh ? "the server is idle at launch" : "the server is idle");
  await updateServer(status.host, status.serverRelease).catch(() => {});
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
  if (idle) await updateServer(status.host, activity.serverRelease, false).catch(() => {});
}

/**
 * Settles once the drop has reconnected, the loop needs the user, or `ms` passes. `lostAt`
 * clears only after the reconnect's status refetch lands, so nothing reads a stale `lost`.
 */
function reconnected(dropped: () => boolean, ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(finish, ms);
    const check = (s: ReconnectSnapshot) => {
      if (dropped() && (s.lostAt === null || s.halted !== null || s.keyChanged !== null)) finish();
    };
    const unsubscribe = useReconnectStore.subscribe(check);
    check(useReconnectStore.getState());
    function finish() {
      clearTimeout(timer);
      unsubscribe();
      resolve();
    }
  });
}

/**
 * Replaces the server with this app's build. `updating` holds through the reconnect, so the
 * status bar reads the planned drop as an update rather than an outage.
 */
export async function updateServer(
  host: Host | null,
  from: string | null,
  connected = true,
): Promise<void> {
  if (useServerAutoUpdate.getState().updating) return;
  useServerAutoUpdate.setState({ updating: true });
  retryAfter = Date.now() + RETRY_MS;
  const where = host ? hostName(host) : "this machine";
  log.info("server-update", `updating the server on ${where} from ${from ?? "unknown"}`);
  // Subscribed before the call: its drop can land before the call returns.
  let dropped = false;
  const unwatch = useReconnectStore.subscribe((s) => {
    if (s.lostAt !== null) dropped = true;
  });
  try {
    announce = where;
    planRestart();
    // A live connection drops when the server stops, and the reconnect loop starts this app's build.
    if (host) await runUpdate(host, () => {});
    else await unwrap(commands.restartServer());
    if (connected) await reconnected(() => dropped, SETTLE_MS);
    else await retryConnection();
  } catch (e) {
    announce = null;
    cancelPlannedRestart();
    log.warn("server-update", "update failed", e);
    throw e;
  } finally {
    unwatch();
    useServerAutoUpdate.setState({ updating: false });
    idleSince = null;
  }
}

function run(): void {
  tick().catch((e) => {
    idleSince = null;
    log.info("server-auto-update", "idle check failed", e);
  });
}

/**
 * Updates an outdated server once nothing is running on it, at launch before anyone starts
 * working, or later once the user has stepped away.
 */
export async function startServerAutoUpdate(): Promise<void> {
  // Debug builds flag the server outdated on every rebuild; restarting it would kill the dev server.
  if (import.meta.env.DEV && !import.meta.env.VITE_AUTO_UPDATE_SERVER) return;
  for (const type of ["keydown", "pointerdown", "wheel"]) {
    window.addEventListener(type, noteActivity, { capture: true, passive: true });
  }
  appVersion = await getVersion().catch(() => "");
  run();
  window.setInterval(run, POLL_MS);
}
