import type { GoalCheck } from "@/lib/bindings";

/** The plan phases that list a check; the first one owns it. */
export function checkPhases(check: GoalCheck): string[] {
  if (check.kind === "command") return check.phases ?? [];
  return check.kind === "plan_boxes" && check.phase ? [check.phase] : [];
}

/** A check runs while any phase that lists it is on; one no phase lists always runs. */
export function phaseAllows(check: GoalCheck, off: ReadonlySet<string>): boolean {
  const phases = checkPhases(check);
  return phases.length === 0 || phases.some((id) => !off.has(id));
}

// "2a" sorts after "2" and before "10".
export function comparePhaseIds(a: string, b: string): number {
  const [na, nb] = [parseInt(a, 10), parseInt(b, 10)];
  if (!Number.isNaN(na) && !Number.isNaN(nb) && na !== nb) return na - nb;
  return a.localeCompare(b);
}

/** `./app/` → `app`, and the worktree root is null, as the server's `normalize_dir` reads a plan. */
export function normalizeDir(raw: string | null | undefined): string | null {
  let dir = (raw ?? "").trim();
  while (dir.startsWith("./")) dir = dir.slice(2);
  dir = dir.replace(/\/+$/, "");
  return dir && dir !== "." ? dir : null;
}

/** Relative and free of `..`; the server refuses any other directory. */
export function dirInside(dir: string): boolean {
  return !/^([A-Za-z]:|[\\/])/.test(dir) && dir.split(/[\\/]/).every((part) => part !== "..");
}
