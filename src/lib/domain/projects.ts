
import type { Project, ProjectSource, Workstream, WorkstreamView } from "@/lib/bindings";

function isFolderSource(source: ProjectSource): boolean {
  return source === "folder";
}

export function isGitProject(project: Pick<Project, "source"> | undefined | null): boolean {
  return project != null && !isFolderSource(project.source);
}

export function isFolderWorkstream(view: Pick<WorkstreamView, "item">): boolean {
  return view.item.worktreeMode === "folder";
}

/** Mirrors core's `app_owns_worktree`: only these folders are moved on archive and deleted on purge. */
export function appOwnsWorktree(item: Pick<Workstream, "worktreeMode">): boolean {
  return item.worktreeMode === "isolated" || item.worktreeMode === "existing";
}

export function runsInProjectDir(item: Pick<Workstream, "worktreeMode">): boolean {
  return item.worktreeMode === "inplace" || item.worktreeMode === "folder";
}

export function shortBranch(ref: string): string {
  return ref.replace(/^origin\//, "");
}

export function isOffBase(view: Pick<WorkstreamView, "item">): boolean {
  const { branch, baseBranch } = view.item;
  return branch.length > 0 && branch !== shortBranch(baseBranch);
}

export function isDetachedHead(view: Pick<WorkstreamView, "item" | "git">): boolean {
  return !isFolderWorkstream(view) && view.git.exists && view.git.branch == null;
}

export function isUserDirectory(project: Pick<Project, "source"> | undefined | null): boolean {
  return project != null && project.source !== "clone";
}
