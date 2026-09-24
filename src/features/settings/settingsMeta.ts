const plural = (n: number, noun: string) => `${n} ${n === 1 ? noun : `${noun}s`}`;

export type ServerCounts = { total: number; unreachable: number };

export function workshopMeta(agents: number | null, skills: number | null): string {
  return agents == null || skills == null
    ? "—"
    : `${plural(agents, "agent")} · ${plural(skills, "skill")}`;
}

export function serversMeta({ total, unreachable }: ServerCounts): string {
  return unreachable > 0 ? `${total} · ${unreachable} offline` : `${total}`;
}

export function serversHeadline({ total, unreachable }: ServerCounts): string {
  const head = plural(total, "server");
  return unreachable > 0 ? `${head} · ${unreachable} unreachable` : head;
}
