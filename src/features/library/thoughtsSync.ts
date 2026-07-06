import type { Project } from "@/lib/bindings";

/** Minimum gap between automatic syncs (open/re-scope triggers). The thoughts repo
 *  is shared across projects, so any recent sync satisfies every scope; within this
 *  window auto-triggers are treated as covered. Manual clicks bypass it. */
export const AUTO_SYNC_COOLDOWN_MS = 60_000;

export function shouldAutoSync(
  now: number,
  lastAttemptAt: number,
  cooldownMs = AUTO_SYNC_COOLDOWN_MS,
): boolean {
  return now - lastAttemptAt >= cooldownMs;
}

/** Repo path handed to `hyprlayer thoughts sync`; the CLI resolves from cwd, so it
 *  must be a mapped project. Falls back Library-origin → scoped/last-used → any; the
 *  thoughts repo is shared, so the host repo doesn't change what syncs. */
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
