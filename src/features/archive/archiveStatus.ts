import type { ArchiveInfo, ArchivedWorkstreamView } from "@/lib/bindings";
import { appOwnsWorktree } from "@/lib/domain/projects";

const PURGE_WARNING_DAYS = 7;

const DAY_MS = 86_400_000;

type ArchiveBase = "merged" | "superseded" | "abandoned";
export type ArchiveTone = ArchiveBase | "purge" | "neutral";

type ArchiveStatus = {
  /** Null for an open or unknown PR: no base tag is claimed. */
  base: ArchiveBase | null;
  tone: ArchiveTone;
  tag: string | null;
  prRef: string | null;
  purgePending: boolean;
  uncommittedWork: boolean;
};

export type ArchiveRow = {
  view: ArchivedWorkstreamView;
  id: string;
  /** The branch, or the title of a folder workstream, which has none. */
  branch: string;
  project: string;
  status: ArchiveStatus;
};

export type DeleteLoss = { uncommitted: number; unpushed: number; unchecked: boolean };

function daysUntilPurge(purgeInMs: number): number {
  return Math.max(0, Math.ceil(purgeInMs / DAY_MS));
}

function baseOf(pr: ArchiveInfo["pr"], supersededBy: string | null): ArchiveBase | null {
  if (pr.state === "merged") return "merged";
  if (supersededBy) return "superseded";
  if (pr.state === "none" || pr.state === "closed") return "abandoned";
  return null;
}

function prRefOf(pr: ArchiveInfo["pr"], base: ArchiveBase | null, supersededBy: string | null) {
  if (base === "superseded") return `replaced by ${supersededBy}`;
  switch (pr.state) {
    case "merged":
      return `PR #${pr.number}`;
    case "closed":
      return `PR #${pr.number} closed`;
    case "open":
      return `PR #${pr.number} open`;
    case "none":
    case "unknown":
      return null;
  }
}

export function resolveArchiveStatus(info: ArchiveInfo, supersededBy: string | null): ArchiveStatus {
  const base = baseOf(info.pr, supersededBy);
  const days = info.purgeInMs === null ? null : daysUntilPurge(info.purgeInMs);
  const purgePending = !info.kept && days !== null && days <= PURGE_WARNING_DAYS;
  const baseTag = base === null ? null : base.toUpperCase();
  return {
    base,
    tone: purgePending ? "purge" : (base ?? "neutral"),
    tag: purgePending ? (days === 0 ? "PURGES TODAY" : `PURGES IN ${days}d`) : baseTag,
    prRef: prRefOf(info.pr, base, supersededBy),
    purgePending,
    uncommittedWork: purgePending && (info.uncommitted ?? 0) > 0,
  };
}

export function toArchiveRow(view: ArchivedWorkstreamView, project: string): ArchiveRow {
  return {
    view,
    id: view.item.id,
    branch: view.item.branch || view.item.title,
    project,
    // TODO(archive): nothing records which branch replaced an archive, so SUPERSEDED never shows.
    status: resolveArchiveStatus(view.info, null),
  };
}

export function filterArchived(rows: ArchiveRow[], query: string, mergedOnly: boolean): ArchiveRow[] {
  const q = query.trim().toLowerCase();
  return rows.filter(
    (row) =>
      (!mergedOnly || row.status.base === "merged") &&
      (!q || row.branch.toLowerCase().includes(q) || row.project.toLowerCase().includes(q)),
  );
}

/**
 * What deleting the archive would destroy, or null when nothing. A squash-merged
 * branch always reads as ahead of its base, so its commits are not counted.
 */
export function lossOnDelete(row: ArchiveRow): DeleteLoss | null {
  const { item, info, archiveExists } = row.view;
  const ownFiles = appOwnsWorktree(item);
  if (!ownFiles || !archiveExists) return null;
  const unchecked = info.uncommitted === null;
  const uncommitted = info.uncommitted ?? 0;
  const unpushed =
    info.deletesBranch && row.status.base !== "merged" ? (info.unpushed ?? 0) : 0;
  return unchecked || uncommitted > 0 || unpushed > 0 ? { uncommitted, unpushed, unchecked } : null;
}

export function mergedPurgeSelection(rows: ArchiveRow[]): ArchiveRow[] {
  return rows.filter((row) => row.status.base === "merged" && !row.view.info.kept);
}

export function totalBytes(rows: ArchiveRow[]): number {
  return rows.reduce((sum, row) => sum + (row.view.info.sizeBytes ?? 0), 0);
}
