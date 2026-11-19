import { describe, expect, it } from "vitest";

import type { ArchiveInfo, ArchivedWorkstreamView, WorktreeMode } from "@/lib/bindings";

import {
  filterArchived,
  lossOnDelete,
  mergedPurgeSelection,
  resolveArchiveStatus,
  toArchiveRow,
  type ArchiveRow,
} from "./archiveStatus";

const DAY = 86_400_000;
const RETENTION = 30 * DAY;

function info(over: Partial<ArchiveInfo> = {}): ArchiveInfo {
  return {
    purgeInMs: RETENTION,
    kept: false,
    sizeBytes: 212e6,
    uncommitted: 0,
    unpushed: 0,
    deletesBranch: true,
    pr: { state: "none" },
    ...over,
  };
}

/** An archive `age` days old under the 30-day retention. */
const aged = (age: number, over: Partial<ArchiveInfo> = {}) =>
  info({ purgeInMs: RETENTION - age * DAY, ...over });

const status = (over: Partial<ArchiveInfo>) =>
  resolveArchiveStatus(info(over), null);

function row(
  branch: string,
  project: string,
  over: Partial<ArchiveInfo> = {},
  opts: { mode?: WorktreeMode; exists?: boolean } = {},
): ArchiveRow {
  const view: ArchivedWorkstreamView = {
    item: {
      id: `id-${branch}`,
      projectPath: `/code/${project}`,
      title: branch,
      slug: branch,
      branch,
      worktreePath: `/wt/${branch}`,
      baseBranch: "main",
      worktreeMode: opts.mode ?? "isolated",
      stage: "general",
      status: "archived",
      prNumber: null,
      createdAt: 1,
      updatedAt: 2,
      archivedAt: 3,
    },
    archivePath: `/code/${project}/archived-contexts/${branch}`,
    archiveExists: opts.exists ?? true,
    info: info(over),
  };
  return toArchiveRow(view, project);
}

describe("resolveArchiveStatus", () => {
  it("reads a merged PR as MERGED", () => {
    expect(status({ pr: { state: "merged", number: 38 } })).toMatchObject({
      base: "merged",
      tone: "merged",
      tag: "MERGED",
      prRef: "PR #38",
      purgePending: false,
    });
  });

  it("reads no PR as ABANDONED, without saying so in the meta line", () => {
    expect(status({ pr: { state: "none" } })).toMatchObject({
      base: "abandoned",
      tone: "abandoned",
      tag: "ABANDONED",
      prRef: null,
    });
  });

  it("reads a PR closed unmerged as ABANDONED and names it", () => {
    expect(status({ pr: { state: "closed", number: 12 } })).toMatchObject({
      base: "abandoned",
      tag: "ABANDONED",
      prRef: "PR #12 closed",
    });
  });

  it("gives an open PR no tag and names it open", () => {
    expect(status({ pr: { state: "open", number: 41 } })).toMatchObject({
      base: null,
      tone: "neutral",
      tag: null,
      prRef: "PR #41 open",
    });
  });

  it("claims nothing while the PR state is unknown", () => {
    expect(status({ pr: { state: "unknown" } })).toMatchObject({
      base: null,
      tone: "neutral",
      tag: null,
      prRef: null,
    });
  });

  it("swaps the tag for the countdown in the last week, keeping the base PR ref", () => {
    const pending = resolveArchiveStatus(aged(25, { pr: { state: "merged", number: 38 } }), null);
    expect(pending).toMatchObject({
      base: "merged",
      tone: "purge",
      tag: "PURGES IN 5d",
      prRef: "PR #38",
      purgePending: true,
    });
  });

  it("flags uncommitted work only on a row about to purge", () => {
    expect(resolveArchiveStatus(aged(25, { uncommitted: 2 }), null).uncommittedWork).toBe(true);
    expect(resolveArchiveStatus(aged(25, { uncommitted: 0 }), null).uncommittedWork).toBe(false);
    expect(resolveArchiveStatus(aged(3, { uncommitted: 2 }), null).uncommittedWork).toBe(false);
  });

  it("shows no countdown when the server sent no deadline", () => {
    expect(status({ purgeInMs: null }).purgePending).toBe(false);
  });
});

describe("the purge countdown", () => {
  const tagAt = (age: number) => resolveArchiveStatus(aged(age), null).tag;

  it("stays hidden until the last seven days", () => {
    expect(tagAt(22)).toBe("ABANDONED");
    expect(tagAt(22.5)).toBe("ABANDONED");
  });

  it("counts whole days left, rounding a part day up", () => {
    expect(tagAt(23)).toBe("PURGES IN 7d");
    expect(tagAt(23.5)).toBe("PURGES IN 7d");
    expect(tagAt(24)).toBe("PURGES IN 6d");
    expect(tagAt(29)).toBe("PURGES IN 1d");
    expect(tagAt(29.9)).toBe("PURGES IN 1d");
  });

  it("reads due on day 30 and after, until the sweep runs", () => {
    expect(tagAt(30)).toBe("PURGES TODAY");
    expect(tagAt(31)).toBe("PURGES TODAY");
  });

});

describe("filterArchived", () => {
  const rows = [
    row("og-image-generator", "acme-web", { pr: { state: "merged", number: 38 } }),
    row("fix-login", "billing-api", { pr: { state: "none" } }),
    row("OG-Card", "acme-web", { purgeInMs: RETENTION - 26 * DAY, pr: { state: "merged", number: 7 } }),
    row("spike-cache", "Acme-Infra", { pr: { state: "open", number: 9 } }),
  ];
  const branches = (list: ArchiveRow[]) => list.map((r) => r.branch);

  it("keeps everything for an empty query", () => {
    expect(branches(filterArchived(rows, "  ", false))).toEqual(branches(rows));
  });

  it("matches the branch, ignoring case", () => {
    expect(branches(filterArchived(rows, "og-", false))).toEqual(["og-image-generator", "OG-Card"]);
  });

  it("matches the project name, ignoring case", () => {
    expect(branches(filterArchived(rows, "ACME", false))).toEqual([
      "og-image-generator",
      "OG-Card",
      "spike-cache",
    ]);
  });

  it("keeps only merged rows under the Merged toggle, including ones about to purge", () => {
    expect(branches(filterArchived(rows, "", true))).toEqual(["og-image-generator", "OG-Card"]);
  });

  it("applies the query and the toggle together", () => {
    expect(branches(filterArchived(rows, "card", true))).toEqual(["OG-Card"]);
    expect(filterArchived(rows, "billing", true)).toEqual([]);
  });
});

describe("lossOnDelete", () => {

  it("counts uncommitted files", () => {
    expect(lossOnDelete(row("dirty", "p", { uncommitted: 2 }))).toEqual({
      uncommitted: 2,
      unpushed: 0,
      unchecked: false,
    });
  });

  it("counts unpushed commits when the delete takes the branch", () => {
    expect(lossOnDelete(row("ahead", "p", { unpushed: 3 }))).toEqual({
      uncommitted: 0,
      unpushed: 3,
      unchecked: false,
    });
  });

  it("ignores commits a merged branch only appears to be ahead by", () => {
    expect(lossOnDelete(row("squashed", "p", { unpushed: 3, pr: { state: "merged", number: 1 } }))).toBeNull();
  });

  it("ignores commits when the branch survives the delete", () => {
    expect(lossOnDelete(row("kept-branch", "p", { unpushed: 3, deletesBranch: false }))).toBeNull();
  });

  it("risks no files where none were archived", () => {
    expect(lossOnDelete(row("inplace", "p", { uncommitted: null }, { mode: "inplace" }))).toBeNull();
    expect(lossOnDelete(row("gone", "p", { uncommitted: null }, { exists: false }))).toBeNull();
  });
});

describe("mergedPurgeSelection", () => {
  it("takes merged archives and leaves kept ones alone", () => {
    const rows = [
      row("merged", "p", { pr: { state: "merged", number: 1 } }),
      row("merged-kept", "p", { kept: true, pr: { state: "merged", number: 2 } }),
      row("merged-soon", "p", { purgeInMs: 2 * DAY, pr: { state: "merged", number: 3 } }),
      row("abandoned", "p", { pr: { state: "none" } }),
      row("open", "p", { pr: { state: "open", number: 4 } }),
    ];
    expect(mergedPurgeSelection(rows).map((r) => r.branch)).toEqual(["merged", "merged-soon"]);
  });
});

describe("toArchiveRow", () => {
  it("labels a folder workstream, which has no branch, by its title", () => {
    const { view } = row("", "notes", {}, { mode: "folder" });
    expect(toArchiveRow({ ...view, item: { ...view.item, title: "Reading" } }, "notes").branch).toBe(
      "Reading",
    );
  });
});
