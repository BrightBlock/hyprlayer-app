import { useState } from "react";

import type { CommitEntry } from "@/lib/bindings";
import { ErrorNote, Spinner } from "@/components/ui";
import { ChevronDownIcon, ChevronRightIcon } from "@/components/icons";
import { FileTypeIcon } from "@/components/fileIcon";
import { formatRelativeTime, shortSha } from "@/lib/format";
import { useCommitFiles } from "@/lib/queries";
import { GLYPH } from "./changeGlyph";
import { renamedLabel } from "./renamedLabel";

/** One commit on the workstream's branch, expanding in place to the files it
 *  touched. Fetches its file list lazily, only once expanded, and never
 *  refetches on a second expansion: `useCommitFiles` is `staleTime: Infinity`. */
function CommitRow({
  id,
  commit,
  onOpenDiff,
}: {
  id: string;
  commit: CommitEntry;
  onOpenDiff: (path: string, staged: boolean, sha?: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const files = useCommitFiles(id, commit.sha, open);

  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex h-7 w-full items-center gap-2 rounded-md pr-1 text-left hover:bg-edge/40"
        style={{ paddingLeft: 6 }}
      >
        {open ? <ChevronDownIcon size={12} /> : <ChevronRightIcon size={12} />}
        <span className="shrink-0 font-mono text-xs text-ink-dim">{shortSha(commit.sha)}</span>
        <span className="min-w-0 flex-1 truncate text-xs text-ink" title={commit.subject}>
          {commit.subject}
        </span>
      </button>
      <div className="flex items-center gap-1 pb-1 pl-[26px] text-[11px] text-ink-dim">
        <span>{commit.author}</span>
        <span>·</span>
        <span>{formatRelativeTime(commit.timeMs)}</span>
      </div>
      {open &&
        (files.isLoading ? (
          <div className="py-1 pl-[26px]">
            <Spinner size={12} />
          </div>
        ) : files.isError ? (
          <div className="px-1 py-1 pl-[26px]">
            <ErrorNote error={files.error} />
          </div>
        ) : (files.data ?? []).length === 0 ? (
          <p className="px-1 pb-1 pl-[26px] text-xs text-ink-dim/70">No files changed.</p>
        ) : (
          (files.data ?? []).map((entry) => {
            const glyph = GLYPH[entry.change] ?? {
              ch: "?",
              cls: "text-ink-dim",
              label: entry.change,
            };
            // A rename shows both names, but opens under the new one: the sides
            // read re-derives the old path from the commit's own diff, so
            // nothing downstream needs a second path.
            const label = renamedLabel(entry.path, entry.oldPath);
            return (
              <button
                key={entry.path}
                type="button"
                onClick={() => onOpenDiff(entry.path, false, commit.sha)}
                className="flex h-7 w-full items-center gap-2 rounded-md pl-[26px] pr-1 text-left hover:bg-edge/40"
              >
                <span
                  className={`w-3 shrink-0 text-center font-mono text-xs font-semibold ${glyph.cls}`}
                  title={glyph.label}
                >
                  {glyph.ch}
                </span>
                <FileTypeIcon name={entry.path} size={13} />
                <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink" title={label}>
                  {label}
                </span>
              </button>
            );
          })
        ))}
    </div>
  );
}

/** The Commits tab body: this workstream's own commits, newest first, each
 *  expanding in place to the files it changed. The header owns the commit
 *  list query (its refresh button needs a `refetch` handle); this component
 *  owns only per-commit expansion state. */
export function CommitsPanel({
  id,
  commits,
  onOpenDiff,
}: {
  id: string;
  commits: CommitEntry[];
  onOpenDiff: (path: string, staged: boolean, sha?: string) => void;
}) {
  return (
    <div>
      {commits.map((commit) => (
        <CommitRow key={commit.sha} id={id} commit={commit} onOpenDiff={onOpenDiff} />
      ))}
    </div>
  );
}
