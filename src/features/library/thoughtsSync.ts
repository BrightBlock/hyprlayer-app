import type { Project } from "@/lib/bindings";

export const AUTO_SYNC_COOLDOWN_MS = 60_000;

export function shouldAutoSync(
  now: number,
  lastAttemptAt: number,
  cooldownMs = AUTO_SYNC_COOLDOWN_MS,
): boolean {
  return now - lastAttemptAt >= cooldownMs;
}

export function resolveSyncProjectPath(
  search: { project?: string; from?: string },
  lastProject: string | null,
  projects: Project[],
): string | undefined {
  const fromRepo = search.from ? projects.find((p) => p.path === search.from) : undefined;
  if (fromRepo) return fromRepo.path;
  const key = search.project ?? lastProject;
  const scoped = key ? projects.find((p) => p.thoughtsProject === key) : undefined;
  return (scoped ?? projects[0])?.path;
}
