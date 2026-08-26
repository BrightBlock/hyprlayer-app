
import type { Project, ProjectSource, WorkstreamView } from "@/lib/bindings";

export function isFolderSource(source: ProjectSource): boolean {
  return source === "folder";
}

export function isGitProject(project: Pick<Project, "source"> | undefined | null): boolean {
  return project != null && !isFolderSource(project.source);
}

export function isFolderWorkstream(view: Pick<WorkstreamView, "item">): boolean {
  return view.item.worktreeMode === "folder";
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
