import { useState } from "react";

import type { CommitEntry } from "@/lib/bindings";
import { ErrorNote, rowClass, Spinner } from "@/components/ui";
import { ChevronDownIcon, ChevronRightIcon } from "@/components/icons";
import { FileTypeIcon } from "@/components/fileIcon";
import { formatRelativeTime, shortSha } from "@/lib/format";
import { useCommitFiles } from "@/lib/queries";
import { GLYPH } from "./changeGlyph";
import { renamedLabel } from "./renamedLabel";

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
        className={rowClass({ tier: "compact", className: "flex w-full gap-2 pr-1 text-left" })}
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
            const label = renamedLabel(entry.path, entry.oldPath);
            return (
              <button
                key={entry.path}
                type="button"
                onClick={() => onOpenDiff(entry.path, false, commit.sha)}
                className={rowClass({
                  tier: "compact",
                  className: "flex w-full gap-2 pl-[26px] pr-1 text-left",
                })}
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
