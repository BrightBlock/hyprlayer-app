import { focusManager } from "@tanstack/react-query";

import { commands, events } from "@/lib/bindings";
import { notifyServerReconnected, type EventBridge } from "@/lib/events";
import { errorMessage, errorRemedy, isAppError, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";
import { useToast } from "@/lib/toast";
import { HOST_KEY_CHANGED } from "./failures";
import { useReconnectStore } from "./reconnectState";

let running = false;
// Resolves the wait between attempts. Non-null only while waiting, which is what
// makes the loop single-flight: a focus during an attempt wakes nothing.
let wake: (() => void) | null = null;

// Paced by the shell's `reconnectDue`: a hidden window's own timers stall for minutes.
function nextAttempt(): Promise<void> {
  return new Promise((resolve) => {
    wake = () => {
      wake = null;
      resolve();
    };
  });
}

/** Jumps the timer — the user coming back is the strongest signal a retry is worth making. */
export function reconnectNow(): void {
  wake?.();
}

// The shell's schedule restarts with each loop, so a resumed one retries in about 1s.
function pace(on: boolean): void {
  void commands.paceReconnect(on).catch((e) => log.warn("reconnect", "pacing the retries failed", e));
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
  let resume = false;
  try {
    await attempt();
    notifyServerReconnected();
    await queryClient.invalidateQueries();
    useReconnectStore.setState({ lostAt: null, halted: null, keyChanged: null });
  } catch (e) {
    log.info("reconnect", "retry failed", e);
    // The failure may have changed, e.g. installed but the running server is too old.
    void queryClient.invalidateQueries({ queryKey: ["connection-status"] });
    // A Sign in that failed some other way left the user nothing to answer: dial again.
    resume =
      useReconnectStore.getState().halted === "signIn" &&
      !(isAppError(e) && e.code === "remote_sign_in_cancelled");
  } finally {
    running = false;
  }
  if (resume) startReconnecting();
}

let launchSignIn = false;

/** Once per launch: the launch connect cannot ask for a password, the window can. */
export function signInAfterLaunch(): void {
  if (launchSignIn) return;
  launchSignIn = true;
  void retryConnection();
}

export function startReconnecting(): void {
  void reconnectLoop();
}

async function reconnectLoop(): Promise<void> {
  if (running) return;
  running = true;
  const lostAt = Date.now();
  useReconnectStore.setState({ lostAt, halted: null, keyChanged: null });
  let lastCode: string | null = null;
  log.info("reconnect", "server connection lost; reconnecting");
  pace(true);
  try {
    for (let n = 1; ; n++) {
      await nextAttempt();
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
        useReconnectStore.setState({ lostAt: null, keyChanged: null });
        return;
      } catch (e) {
        // Logged when the reason changes, not on every retry.
        const code = isAppError(e) ? e.code : "unknown";
        if (code !== lastCode) {
          lastCode = code;
          log.warn("reconnect", `attempt ${n} failed`, e);
        }
        // Dialling goes on, but only the user can clear this; the server dialog says how.
        useReconnectStore.setState({
          keyChanged: isAppError(e) && e.code === HOST_KEY_CHANGED ? e : null,
        });
        // Disagreeing binaries cannot resolve themselves, and a relaunch would
        // not help either; everything else keeps trying.
        if (isAppError(e) && e.code === "protocol_version_mismatch") {
          log.error("reconnect", "stopped: the app and server speak different protocols");
          pace(false);
          useReconnectStore.setState({ halted: "protocol" });
          useToast.getState().show(errorMessage(e), errorRemedy(e) ?? "", { tone: "bad" });
          return;
        }
        // Asking again every 30s would overrule the user's answer; the server dialog offers Sign in.
        if (isAppError(e) && e.code === "remote_sign_in_cancelled") {
          log.info("reconnect", "stopped: sign-in was cancelled");
          pace(false);
          useReconnectStore.setState({ halted: "signIn" });
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
  events.reconnectDue.listen(() => reconnectNow()),
  // The same focus signal `lib/queryFocus.ts` already feeds from Tauri's window
  // event, rather than a second listener over the same thing.
  Promise.resolve(
    focusManager.subscribe((focused) => {
      if (focused) reconnectNow();
    }),
  ),
];
