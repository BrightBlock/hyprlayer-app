import { commands, type Session } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";

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
  }
}
