import { commands } from "./bindings";
import { errorMessage, unwrap } from "./ipc";
import { log } from "./log";
import { useToast } from "./toast";

/** Versions, the connection, and both machines' log tails, for a bug report. */
export async function copyDiagnostics(): Promise<void> {
  const show = useToast.getState().show;
  try {
    const report = await unwrap(commands.collectDiagnostics());
    await navigator.clipboard.writeText(report);
    show("Diagnostics copied", "Home paths are masked; review it before sharing.", {
      tone: "ok",
    });
  } catch (e) {
    log.warn("diagnostics", "copying diagnostics failed", e);
    show("Couldn't copy diagnostics", errorMessage(e), { tone: "bad" });
  }
}

export function revealLogs(): void {
  void unwrap(commands.revealLogs()).catch((e) => {
    log.warn("diagnostics", "revealing the logs failed", e);
    useToast.getState().show("Couldn't reveal the logs", errorMessage(e), { tone: "bad" });
  });
}
