import { useEffect, useMemo } from "react";

import { harnessLabel } from "@/lib/domain/harnessing";
import { permissionModeOption } from "@/lib/domain/permissions";
import { useWorkstreamSessions } from "@/lib/queries";
import { restoreSessionIfNeeded, useSession } from "@/features/session/sessionStore";
import { sessionPermissionMode } from "@/features/session/workstreamTabs";
import { newestImplementSession } from "@/features/workstreams/useGitActions";

// Loads the implement transcript even when the user didn't come from its chat tab.
export function useImplementSession(workstreamId: string) {
  const sessions = useWorkstreamSessions(workstreamId);
  const session = useMemo(() => newestImplementSession(sessions.data ?? []), [sessions.data]);
  const id = session?.id;
  useEffect(() => {
    if (!id) return;
    const mode = permissionModeOption(sessionPermissionMode(workstreamId, id)).flag;
    void restoreSessionIfNeeded(id, workstreamId, "implement", mode);
  }, [id, workstreamId]);
  const entry = useSession(id);
  return { session, entry, agentLabel: harnessLabel(session?.harness) };
}
