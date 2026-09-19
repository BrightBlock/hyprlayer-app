import { focusManager } from "@tanstack/react-query";

import { commands, events } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";
import { errorMessage, errorRemedy, isAppError, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";
import { useToast } from "@/lib/toast";

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
    await unwrap(commands.reconnectServer());
    await queryClient.invalidateQueries();
  } catch (e) {
    // The status already on screen is still the best answer; the log keeps why.
    log.info("reconnect", "retry failed", e);
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
  let lastCode: string | null = null;
  log.info("reconnect", "server connection lost; reconnecting");
  try {
    for (let attempt = 1; ; attempt++) {
      await wait(DELAYS[attempt - 1] ?? STEADY_DELAY);
      try {
        await unwrap(commands.reconnectServer());
        log.info(
          "reconnect",
          `reconnected on attempt ${attempt}, ${Math.round((Date.now() - lostAt) / 1000)}s after the drop`,
        );
        // The server may have restarted with different state.
        await queryClient.invalidateQueries();
        return;
      } catch (e) {
        // Logged when the reason changes, not on every retry.
        const code = isAppError(e) ? e.code : "unknown";
        if (code !== lastCode) {
          lastCode = code;
          log.warn("reconnect", `attempt ${attempt} failed`, e);
        }
        // Disagreeing binaries cannot resolve themselves, and a relaunch would
        // not help either; everything else keeps trying.
        if (isAppError(e) && e.code === "protocol_version_mismatch") {
          log.error("reconnect", "stopped: the app and server speak different protocols");
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
