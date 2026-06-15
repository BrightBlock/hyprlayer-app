import { memo, useMemo, useState } from "react";

import type { FileDiff } from "../../lib/bindings";
import { Badge, ErrorNote, IconButton, Spinner, type BadgeTone } from "../../components/ui";
import { ChevronRightIcon, DocIcon } from "../../components/icons";
import { useWorkstreamDocs } from "../session/sessionStore";
import type { DocKind } from "../session/toolDisplay";
import { useWorktreeDiff } from "../../lib/queries";
import { countPatchStats, parsePatchLines, type PatchLineKind } from "./diff";

function PanelRightIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path
        d="M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-13Z"
        stroke="currentColor"
        strokeWidth="1.8"
      />
      <path d="M14 4v16" stroke="currentColor" strokeWidth="1.8" />
    </svg>
  );
}

function RefreshIcon() {
  return (
    <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none">
      <path
        d="M20 12a8 8 0 1 1-2.34-5.66M20 4v4h-4"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

const CHANGE_TONES: Record<string, BadgeTone> = {
  added: "ok",
  modified: "accent",
  deleted: "bad",
  renamed: "warn",
  other: "dim",
};

const LINE_CLASSES: Record<PatchLineKind, string> = {
  add: "bg-emerald-400/10 text-emerald-300",
  del: "bg-rose-400/10 text-rose-300",
  hunk: "text-sky-300",
  meta: "text-ink-dim/70",
  context: "text-ink-dim",
};

// Memoized: patch parsing is linear in diff size and the parent re-renders on
// every pointermove while the sidebar is being resized.
const FileDiffSection = memo(function FileDiffSection({ file }: { file: FileDiff }) {
  const [open, setOpen] = useState(false);
  const stats = useMemo(() => countPatchStats(file.patch), [file.patch]);
  const lines = useMemo(() => (open ? parsePatchLines(file.patch) : []), [open, file.patch]);

  return (
    <div className="overflow-hidden rounded-lg border border-edge bg-panel-2">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-edge/40"
      >
        <ChevronRightIcon
          size={12}
          className={`shrink-0 transition-transform ${open ? "rotate-90" : ""}`}
        />
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink" title={file.path}>
          {file.path}
        </span>
        {stats.additions > 0 && (
          <span className="shrink-0 font-mono text-xs text-emerald-300">+{stats.additions}</span>
        )}
        {stats.deletions > 0 && (
          <span className="shrink-0 font-mono text-xs text-rose-300">-{stats.deletions}</span>
        )}
        <Badge tone={CHANGE_TONES[file.change] ?? "dim"}>{file.change}</Badge>
      </button>
      {open && (
        <pre className="selectable max-h-96 overflow-auto border-t border-edge p-2 font-mono text-[11px] leading-relaxed">
          {lines.map((line, index) => (
            <div key={index} className={`whitespace-pre px-1 ${LINE_CLASSES[line.kind]}`}>
              {line.text || " "}
            </div>
          ))}
        </pre>
      )}
    </div>
  );
});

type Props = {
  /** Active workstream, or null when there are no workstreams. */
  workstreamId: string | null;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  /** Open an agent-created doc as an in-place preview tab. */
  onOpenDoc: (path: string, title: string) => void;
};

const DOC_TONE: Record<DocKind, string> = {
  research: "bg-sky-400/10 text-sky-300",
  plan: "bg-amber-400/10 text-amber-300",
};

/** Research/plan docs the agent wrote — click to review in a preview tab. */
function DocsSection({
  workstreamId,
  onOpenDoc,
}: {
  workstreamId: string;
  onOpenDoc: (path: string, title: string) => void;
}) {
  const docs = useWorkstreamDocs(workstreamId);
  if (docs.length === 0) return null;
  return (
    <div className="rounded-lg border border-edge bg-panel-2 p-2">
      <h3 className="px-1 pb-1.5 text-xs font-semibold text-ink-dim">Documents</h3>
      <div className="space-y-0.5">
        {docs.map((doc) => (
          <button
            key={doc.path}
            type="button"
            onClick={() => onOpenDoc(doc.path, doc.title)}
            title={doc.path}
            className="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left hover:bg-edge/50"
          >
            <DocIcon size={13} />
            <span className="min-w-0 flex-1 truncate text-xs text-ink">{doc.title}</span>
            <span className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${DOC_TONE[doc.kind]}`}>
              {doc.kind}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * Contents of the right diff sidebar (header + body). The diff query only
 * runs while the sidebar is expanded and a workstream is active.
 */
export function DiffSidebar({ workstreamId, collapsed, onToggleCollapsed, onOpenDoc }: Props) {
  const diff = useWorktreeDiff(workstreamId ?? "", !collapsed);
  const fileCount = diff.data?.length ?? 0;

  return (
    <>
      <div className="flex h-[61px] items-center border-b border-edge px-3">
        <div className={`flex w-full items-center gap-3 ${collapsed ? "justify-center" : ""}`}>
          <IconButton
            onClick={onToggleCollapsed}
            title={collapsed ? "Expand diff sidebar" : "Collapse diff sidebar"}
            aria-label={collapsed ? "Expand diff sidebar" : "Collapse diff sidebar"}
          >
            <PanelRightIcon />
          </IconButton>
          {!collapsed && (
            <div className="flex min-w-0 flex-1 items-center justify-between gap-3">
              <h2 className="text-sm font-semibold">Diff</h2>
              <div className="flex items-center gap-1.5">
                <Badge tone="dim">{fileCount} {fileCount === 1 ? "file" : "files"}</Badge>
                {workstreamId !== null && (
                  <IconButton
                    onClick={() => void diff.refetch()}
                    disabled={diff.isFetching}
                    title="Refresh diff"
                    aria-label="Refresh diff"
                  >
                    {diff.isFetching ? <Spinner size={14} /> : <RefreshIcon />}
                  </IconButton>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
      {collapsed ? (
        <div className="flex justify-center p-3">
          <Badge tone="dim">{fileCount}</Badge>
        </div>
      ) : (
        <div className="h-[calc(100%-61px)] space-y-2 overflow-y-auto p-4">
          {workstreamId !== null && <DocsSection workstreamId={workstreamId} onOpenDoc={onOpenDoc} />}
          {workstreamId === null ? (
            <div className="rounded-lg border border-edge bg-panel-2 p-3">
              <p className="text-sm font-medium">No workstream selected</p>
              <p className="mt-1 text-xs leading-relaxed text-ink-dim">
                Create a workstream to see its worktree diff here.
              </p>
            </div>
          ) : diff.isLoading ? (
            <div className="flex justify-center p-6">
              <Spinner />
            </div>
          ) : diff.isError ? (
            <ErrorNote error={diff.error} />
          ) : fileCount === 0 ? (
            <div className="rounded-lg border border-edge bg-panel-2 p-3">
              <p className="text-sm font-medium">No changes yet</p>
              <p className="mt-1 text-xs leading-relaxed text-ink-dim">
                Edits in this workstream&apos;s worktree will show up here.
              </p>
            </div>
          ) : (
            diff.data!.map((file) => <FileDiffSection key={file.path} file={file} />)
          )}
        </div>
      )}
    </>
  );
}
