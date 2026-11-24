import { commands, type Session } from "@/lib/bindings";
import { isAppError, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";
import { showFailure } from "@/lib/toast";

// The server refuses a mid-turn archive, so the UI never offers one.
export function canArchiveSession(session: Pick<Session, "working">): boolean {
  return !session.working;
}

const refresh = (workstreamId: string) =>
  queryClient.invalidateQueries({ queryKey: ["workstream-sessions", workstreamId] });

export async function setSessionArchived(
  workstreamId: string,
  sessionId: string,
  archived: boolean,
): Promise<void> {
  try {
    await unwrap(commands.setSessionArchived(sessionId, archived));
    await refresh(workstreamId);
  } catch (e) {
    log.warn("session", `${archived ? "archiving" : "unarchiving"} ${sessionId} failed`, e);
    showFailure(archived ? "Couldn't archive the agent" : "Couldn't restore the agent")(e);
  }
}

const BUSY_RETRY_MS = 250;
const BUSY_RETRY_LIMIT_MS = 10_000;

// A killed harness reports its exit asynchronously, and the server refuses the archive until then.
export async function archiveStoppedSession(workstreamId: string, sessionId: string): Promise<void> {
  for (let waited = 0; ; waited += BUSY_RETRY_MS) {
    try {
      await unwrap(commands.setSessionArchived(sessionId, true));
      await refresh(workstreamId);
      return;
    } catch (e) {
      if (isAppError(e) && e.code === "session_busy" && waited < BUSY_RETRY_LIMIT_MS) {
        await new Promise((resolve) => setTimeout(resolve, BUSY_RETRY_MS));
        continue;
      }
      log.warn("session", `archiving stopped ${sessionId} failed`, e);
      showFailure("Couldn't archive the agent")(e);
      return;
    }
  }
}

// A tab archived before server archiving existed is archived here first.
export async function deleteServerSession(workstreamId: string, sessionId: string): Promise<void> {
  try {
    await unwrap(commands.setSessionArchived(sessionId, true));
    await unwrap(commands.deleteSession(sessionId));
    await refresh(workstreamId);
  } catch (e) {
    log.warn("session", `deleting ${sessionId} failed`, e);
    showFailure("Couldn't delete the agent")(e);
  }
}
