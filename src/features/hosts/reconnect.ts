import { focusManager } from "@tanstack/react-query";

import { commands, events } from "@/lib/bindings";
import { notifyServerReconnected, type EventBridge } from "@/lib/events";
import { errorMessage, errorRemedy, isAppError, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";
import { useToast } from "@/lib/toast";
import { useReconnectStore } from "./reconnectState";

// Quick at first, then settled: the loop never gives up, because relaunching is
// the annoyance this reconnect exists to remove (plan Phase 8 §5). A host asleep
// for three hours should find its session waiting.
const DELAYS = [1_000, 2_000, 5_000, 15_000];
const STEADY_DELAY = 30_000;

let running = false;
// Resolves the wait between attempts. Non-null only while waiting, which is what
// makes the loop single-flight: a focus during an attempt wakes nothing.
let wake: (() => void) | null = null;

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(done, ms);
    wake = done;
    function done() {
      clearTimeout(timer);
      wake = null;
      resolve();
    }
  });
}

/** Jumps the timer — the user coming back is the strongest signal a retry is worth making. */
export function reconnectNow(): void {
  wake?.();
}

async function attempt(): Promise<void> {
  useReconnectStore.setState({ attempting: true });
  try {
    await unwrap(commands.reconnectServer());
  } finally {
    useReconnectStore.setState({ attempting: false });
  }
}

/**
 * One attempt, for when something the user just did disproves the failure the
 * connection surface is still reporting — a probe that answered, an install
 * that landed. Shares the loop's flag rather than dialling beside it.
 */
export async function retryConnection(): Promise<void> {
  if (running) {
    reconnectNow();
    return;
  }
  running = true;
  try {
    await attempt();
    notifyServerReconnected();
    await queryClient.invalidateQueries();
    useReconnectStore.setState({ lostAt: null, halted: false });
  } catch (e) {
    log.info("reconnect", "retry failed", e);
    // The failure may have changed, e.g. installed but the running server is too old.
    void queryClient.invalidateQueries({ queryKey: ["connection-status"] });
  } finally {
    running = false;
  }
}

export function startReconnecting(): void {
  void reconnectLoop();
}

async function reconnectLoop(): Promise<void> {
  if (running) return;
  running = true;
  const lostAt = Date.now();
  useReconnectStore.setState({ lostAt, halted: false });
  let lastCode: string | null = null;
  log.info("reconnect", "server connection lost; reconnecting");
  try {
    for (let n = 1; ; n++) {
      await wait(DELAYS[n - 1] ?? STEADY_DELAY);
      try {
        await attempt();
        log.info(
          "reconnect",
          `reconnected on attempt ${n}, ${Math.round((Date.now() - lostAt) / 1000)}s after the drop`,
        );
        notifyServerReconnected();
        // The server may have restarted with different state.
        await queryClient.invalidateQueries();
        // Only now: the status reads `lost` until the refetch lands, and its counter must hold.
        useReconnectStore.setState({ lostAt: null });
        return;
      } catch (e) {
        // Logged when the reason changes, not on every retry.
        const code = isAppError(e) ? e.code : "unknown";
        if (code !== lastCode) {
          lastCode = code;
          log.warn("reconnect", `attempt ${n} failed`, e);
        }
        // Disagreeing binaries cannot resolve themselves, and a relaunch would
        // not help either; everything else keeps trying.
        if (isAppError(e) && e.code === "protocol_version_mismatch") {
          log.error("reconnect", "stopped: the app and server speak different protocols");
          useReconnectStore.setState({ halted: true });
          useToast.getState().show(errorMessage(e), errorRemedy(e) ?? "", { tone: "bad" });
          return;
        }
      }
    }
  } finally {
    running = false;
  }
}

export const reconnectBridge: EventBridge = () => [
  events.serverDisconnected.listen(() => startReconnecting()),
  // The same focus signal `lib/queryFocus.ts` already feeds from Tauri's window
  // event, rather than a second listener over the same thing.
  Promise.resolve(
    focusManager.subscribe((focused) => {
      if (focused) reconnectNow();
    }),
  ),
];
