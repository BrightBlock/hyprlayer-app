import type { Project } from "../../lib/bindings";

/** Minimum gap between automatic syncs. Opening the Library or re-scoping it
 *  opportunistically syncs the thoughts repo; within this window those
 *  triggers count as already covered — the repo is shared across projects, so
 *  any recent sync satisfies every scope. Manual clicks bypass this. */
export const AUTO_SYNC_COOLDOWN_MS = 60_000;

export function shouldAutoSync(
  now: number,
  lastAttemptAt: number,
  cooldownMs = AUTO_SYNC_COOLDOWN_MS,
): boolean {
  return now - lastAttemptAt >= cooldownMs;
}

/** Repo path handed to `hyprlayer thoughts sync` — the CLI resolves everything
 *  from its cwd, so it must be a mapped project. Prefer the repo the Library
 *  was opened from, then the scoped (or last-used) project, then any project:
 *  the thoughts repo itself is shared, so which mapped repo hosts the sync
 *  doesn't change what syncs. */
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
