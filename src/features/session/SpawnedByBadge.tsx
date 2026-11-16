import { useQuery } from "@tanstack/react-query";

import { Icon } from "@/components/Icon";
import type { Session } from "@/lib/bindings";
import { stageLabel } from "@/lib/domain/stages";
import { useWorkstreamSessions, workstreamSessionsQuery } from "@/lib/queries";

const NONE: string[] = [];
const agentSpawnedIds = (rows: Session[]) => rows.filter((s) => s.origin === "agent").map((s) => s.id);

/** The workstream's agent-spawned session ids; structural sharing re-renders only when that list changes. */
export function useAgentSpawnedSessionIds(workstreamId: string | null): readonly string[] {
  const { data } = useQuery({
    ...workstreamSessionsQuery(workstreamId ?? ""),
    enabled: !!workstreamId,
    select: agentSpawnedIds,
  });
  return data ?? NONE;
}

/** Names the agent that spawned this session; a parent in another workstream reads as "an agent". */
export function SpawnedByBadge({
  workstreamId,
  sessionId,
}: {
  workstreamId: string;
  sessionId?: string | null;
}) {
  const { data: sessions } = useWorkstreamSessions(workstreamId, !!sessionId);
  const session = sessions?.find((s) => s.id === sessionId);
  if (session?.origin !== "agent") return null;
  const parent = sessions?.find((s) => s.id === session.parentSessionId);
  const name = parent ? (parent.title ?? parent.preview ?? stageLabel(parent.stage)) : "an agent";
  return (
    <span className="flex min-w-0 shrink-0 items-center gap-1.5" title={`Spawned by ${name}`}>
      <Icon name="agents" size="xs" className="text-ink-dim" />
      <span className="max-w-[160px] truncate text-meta text-ink-dim">from {name}</span>
    </span>
  );
}
