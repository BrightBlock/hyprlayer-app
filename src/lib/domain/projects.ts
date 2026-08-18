/**
 * Project-kind predicates shared by every surface that has to decide whether
 * git, PR, and thoughts UI applies.
 *
 * The VOCABULARY is the backend's: [`ProjectSource`] and [`WorktreeMode`] are
 * generated from `store::ProjectSource` / `store::WorktreeMode` via specta, so
 * a variant added in Rust surfaces here rather than as a new string literal.
 */

import type { Project, ProjectSource, WorkstreamView } from "@/lib/bindings";

/** A plain local directory: no git, no branches, no thoughts mapping. */
export function isFolderSource(source: ProjectSource): boolean {
  return source === "folder";
}

/** True for clone/disk projects, the ones with branches, PRs, and thoughts. */
export function isGitProject(project: Pick<Project, "source"> | undefined | null): boolean {
  return project != null && !isFolderSource(project.source);
}

/** A workstream backed by a folder project: no branch, no diff, no Ship. */
export function isFolderWorkstream(view: Pick<WorkstreamView, "item">): boolean {
  return view.item.worktreeMode === "folder";
}

/** Drop the `origin/` remote prefix: the stored base is a git rev (`origin/main`)
 *  but users (and GitHub) think in branch names (`main`). */
export function shortBranch(ref: string): string {
  return ref.replace(/^origin\//, "");
}

/** The checkout sits on a branch other than its merge base. True from creation
 *  for an in-place workstream started off the base branch, the base is detected
 *  (`origin/main`), not copied from the branch picked. */
export function isOffBase(view: Pick<WorkstreamView, "item">): boolean {
  const { branch, baseBranch } = view.item;
  return branch.length > 0 && branch !== shortBranch(baseBranch);
}

/** Mid-rebase/bisect: HEAD points at a commit, not a branch. The stored branch
 *  is the last good one, so surfaces say "detached" rather than showing it. */
export function isDetachedHead(view: Pick<WorkstreamView, "item" | "git">): boolean {
  return !isFolderWorkstream(view) && view.git.exists && view.git.branch == null;
}

/** The project directory is the user's own, not an app-made clone, so
 *  deleting it destroys work the app never created. True for disk and folder. */
export function isUserDirectory(project: Pick<Project, "source"> | undefined | null): boolean {
  return project != null && project.source !== "clone";
}
