import { focusManager } from "@tanstack/react-query";

import { commands, events } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";
import { errorMessage, errorRemedy, isAppError, unwrap } from "@/lib/ipc";
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

export function startReconnecting(): void {
  void reconnectLoop();
}

async function reconnectLoop(): Promise<void> {
  if (running) return;
  running = true;
  try {
    for (let attempt = 0; ; attempt++) {
      await wait(DELAYS[attempt] ?? STEADY_DELAY);
      try {
        await unwrap(commands.reconnectServer());
        // The server may have restarted with different state.
        await queryClient.invalidateQueries();
        return;
      } catch (e) {
        // Disagreeing binaries cannot resolve themselves, and a relaunch would
        // not help either; everything else keeps trying.
        if (isAppError(e) && e.code === "protocol_version_mismatch") {
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
