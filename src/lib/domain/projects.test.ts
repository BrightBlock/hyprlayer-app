import { describe, expect, it } from "vitest";

import type { WorkstreamView } from "@/lib/bindings";
import { isDetachedHead, isOffBase, shortBranch } from "./projects";

type Item = WorkstreamView["item"];

function view(item: Partial<Item>, head: string | null = "main"): WorkstreamView {
  return {
    item: { branch: "main", baseBranch: "origin/main", worktreeMode: "inplace", ...item } as Item,
    git: { exists: true, branch: head } as WorkstreamView["git"],
    seed: null,
  };
}

describe("shortBranch", () => {
  it("strips the remote prefix so a base compares against a branch name", () => {
    expect(shortBranch("origin/main")).toBe("main");
    expect(shortBranch("main")).toBe("main");
    expect(shortBranch("feature/origin/thing")).toBe("feature/origin/thing");
  });
});

describe("isOffBase", () => {
  it("is false while the checkout sits on its merge target", () => {
    expect(isOffBase(view({ branch: "main", baseBranch: "origin/main" }))).toBe(false);
    expect(isOffBase(view({ branch: "develop", baseBranch: "origin/develop" }))).toBe(false);
    expect(isOffBase(view({ branch: "main", baseBranch: "main" }))).toBe(false);
  });

  it("is true once the checkout branches away", () => {
    expect(isOffBase(view({ branch: "feature/x", baseBranch: "origin/main" }))).toBe(true);
  });

  it("is false for a folder workstream, which has no branch", () => {
    expect(isOffBase(view({ branch: "", baseBranch: "", worktreeMode: "folder" }))).toBe(false);
  });
});

describe("isDetachedHead", () => {
  it("is true only when a live git checkout has no branch", () => {
    expect(isDetachedHead(view({}, null))).toBe(true);
    expect(isDetachedHead(view({}, "main"))).toBe(false);
  });

  it("is false for a folder workstream, which never has a branch", () => {
    expect(isDetachedHead(view({ worktreeMode: "folder" }, null))).toBe(false);
  });

  it("is false for a worktree missing on disk, that is a different failure", () => {
    const missing = view({}, null);
    missing.git.exists = false;
    expect(isDetachedHead(missing)).toBe(false);
  });
});
