import { memo, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { GitFileEntry } from "@/lib/bindings";
import {
  Badge,
  ConfirmDialog,
  ContextMenu,
  ContextMenuItem,
  ErrorNote,
  IconButton,
  rowClass,
  Spinner,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
import { FileTypeIcon } from "@/components/fileIcon";
import { formatCompactAge } from "@/lib/format";
import { treeIndent } from "@/lib/layout";
import { basename } from "@/lib/path";
import { queryClient } from "@/lib/queryClient";
import {
  useDiscardFiles,
  useProjectMemory,
  useStageFiles,
  useUnstageFiles,
  useWorkstreamCommits,
  useWorktreeDir,
  useWorktreeGitStatus,
} from "@/lib/queries";
import { useUiStore } from "@/lib/uiStore";
import { useWorkstreamArtifacts, useWorkstreamDocs } from "@/features/session/sessionStore";
import type { DocKind } from "@/lib/domain/docs";
import { glyphForStatus } from "@/lib/domain/review";
import { CommitsPanel } from "./CommitsPanel";
import { renamedLabel } from "./renamedLabel";

type RowMenu = { entry: GitFileEntry; staged: boolean; x: number; y: number };

type FileRowProps = {
  entry: GitFileEntry;
  staged: boolean;
  depth: number;
  label: string;
  onOpen: () => void;
  onToggle: () => void;
  onMenu: (x: number, y: number) => void;
};

const FileRow = memo(function FileRow({
  entry,
  staged,
  depth,
  label,
  onOpen,
  onToggle,
  onMenu,
}: FileRowProps) {
  const glyph = glyphForStatus(entry.change);
  return (
    <div
      className={rowClass({ tier: "compact", className: "group flex gap-2 pr-1 text-left" })}
      style={{ paddingLeft: treeIndent(depth, 6) }}
      onClick={onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(e.clientX, e.clientY);
      }}
      role="button"
    >
      <span
        className={`w-3 shrink-0 text-center text-meta font-semibold ${glyph.cls}`}
        title={glyph.label}
      >
        {glyph.ch}
      </span>
      <FileTypeIcon name={entry.path} />
      <span
        className="min-w-0 flex-1 truncate text-meta text-ink"
        title={renamedLabel(entry.path, entry.oldPath)}
      >
        {label}
      </span>
      <button
        type="button"
        title={staged ? "Unstage" : "Stage"}
        aria-label={staged ? "Unstage" : "Stage"}
        onClick={(e) => {
          e.stopPropagation();
          onToggle();
        }}
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-ink-dim opacity-0 hover:bg-edge/70 hover:text-ink group-hover:opacity-100"
      >
        <Icon name={staged ? "arrowUp" : "arrowDown"} size="sm" />
      </button>
    </div>
  );
});


export type TreeNode = {
  name: string;
  path: string;
  entry?: GitFileEntry;
  children: Map<string, TreeNode>;
};

export function buildTree(entries: GitFileEntry[]): TreeNode {
  const root: TreeNode = { name: "", path: "", children: new Map() };
  for (const entry of entries) {
    const parts = entry.path.split("/").filter(Boolean);
    if (parts.length === 0) continue;
    let node = root;
    parts.forEach((part, i) => {
      const path = parts.slice(0, i + 1).join("/");
      let child = node.children.get(part);
      if (!child) {
        child = { name: part, path, children: new Map() };
        node.children.set(part, child);
      }
      if (i === parts.length - 1) child.entry = entry;
      node = child;
    });
  }
  return root;
}

function TreeLevel({
  node,
  depth,
  staged,
  collapsed,
  toggleCollapsed,
  onOpen,
  onToggle,
  onMenu,
}: {
  node: TreeNode;
  depth: number;
  staged: boolean;
  collapsed: Set<string>;
  toggleCollapsed: (path: string) => void;
  onOpen: (entry: GitFileEntry) => void;
  onToggle: (entry: GitFileEntry) => void;
  onMenu: (entry: GitFileEntry, x: number, y: number) => void;
}) {
  const kids = [...node.children.values()].sort((a, b) => {
    const af = a.entry ? 1 : 0;
    const bf = b.entry ? 1 : 0;
    return af - bf || a.name.localeCompare(b.name);
  });
  return (
    <>
      {kids.map((child) =>
        child.entry && child.children.size === 0 ? (
          <FileRow
            key={child.path}
            entry={child.entry}
            staged={staged}
            depth={depth}
            label={renamedLabel(child.entry.path, child.entry.oldPath, true)}
            onOpen={() => onOpen(child.entry!)}
            onToggle={() => onToggle(child.entry!)}
            onMenu={(x, y) => onMenu(child.entry!, x, y)}
          />
        ) : (
          <div key={child.path}>
            <button
              type="button"
              onClick={() => toggleCollapsed(child.path)}
              className={rowClass({ tier: "compact", className: "flex w-full gap-1 text-left" })}
              style={{ paddingLeft: treeIndent(depth, 6) }}
            >
              <Icon name={collapsed.has(child.path) ? "chevronRight" : "chevronDown"} size="xs" />
              <span className="truncate text-ui text-ink-dim">{child.name}</span>
            </button>
            {!collapsed.has(child.path) && (
              <TreeLevel
                node={child}
                depth={depth + 1}
                staged={staged}
                collapsed={collapsed}
                toggleCollapsed={toggleCollapsed}
                onOpen={onOpen}
                onToggle={onToggle}
                onMenu={onMenu}
              />
            )}
          </div>
        ),
      )}
    </>
  );
}


function ChangeSection({
  title,
  entries,
  staged,
  view,
  actionLabel,
  onActionAll,
  onOpen,
  onToggle,
  onMenu,
}: {
  title: string;
  entries: GitFileEntry[];
  staged: boolean;
  view: "list" | "tree";
  actionLabel: string;
  onActionAll: () => void;
  onOpen: (entry: GitFileEntry) => void;
  onToggle: (entry: GitFileEntry) => void;
  onMenu: (entry: GitFileEntry, staged: boolean, x: number, y: number) => void;
}) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const tree = useMemo(() => (view === "tree" ? buildTree(entries) : null), [view, entries]);
  const toggleCollapsed = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  return (
    <div>
      <div className="mb-1 flex items-center justify-between px-1">
        <span className="text-label text-ink-dim">
          {title} {entries.length}
        </span>
        {entries.length > 0 && (
          <button
            type="button"
            onClick={onActionAll}
            className="rounded px-1.5 py-0.5 text-ui text-ink-dim hover:bg-edge/60 hover:text-ink"
          >
            {actionLabel}
          </button>
        )}
      </div>
      {entries.length === 0 ? (
        <p className="px-1 pb-1 text-ui text-ink-dim/70">Nothing here.</p>
      ) : tree ? (
        <TreeLevel
          node={tree}
          depth={0}
          staged={staged}
          collapsed={collapsed}
          toggleCollapsed={toggleCollapsed}
          onOpen={onOpen}
          onToggle={onToggle}
          onMenu={(entry, x, y) => onMenu(entry, staged, x, y)}
        />
      ) : (
        entries.map((entry) => (
          <FileRow
            key={entry.path}
            entry={entry}
            staged={staged}
            depth={0}
            label={renamedLabel(entry.path, entry.oldPath)}
            onOpen={() => onOpen(entry)}
            onToggle={() => onToggle(entry)}
            onMenu={(x, y) => onMenu(entry, staged, x, y)}
          />
        ))
      )}
    </div>
  );
}


const DOC_TONE: Record<DocKind, string> = {
  research: "bg-research/10 text-research",
  plan: "bg-plan/10 text-plan",
  handoff: "bg-handoff/10 text-handoff",
};

function DocsSection({
  workstreamId,
  onOpenDoc,
}: {
  workstreamId: string;
  onOpenDoc: (path: string, title: string) => void;
}) {
  const docs = useWorkstreamDocs(workstreamId);
  const collapsed = useUiStore((s) => s.docsCollapsed);
  const toggle = useUiStore((s) => s.toggleDocs);
  if (docs.length === 0) return null;
  const open = !collapsed;
  const label = `${open ? "Hide" : "Show"} documents, ${docs.length}`;
  return (
    <div className="rounded-lg border border-edge bg-panel-2 p-2">
      <button
        type="button"
        onClick={toggle}
        title={label}
        aria-label={label}
        aria-expanded={open}
        className={`flex w-full items-center px-1 text-left text-ink-dim hover:text-ink ${
          open ? "pb-1.5" : ""
        }`}
      >
        <span className="text-ui font-semibold">Documents</span>
        <span className="ml-auto flex w-3 shrink-0 justify-center">
          <Icon name={open ? "chevronDown" : "chevronRight"} size="xs" />
        </span>
      </button>
      {open && (
        <div className="space-y-0.5">
          {docs.map((doc) => (
            <button
              key={doc.path}
              type="button"
              onClick={() => onOpenDoc(doc.path, doc.title)}
              title={doc.path}
              className="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left hover:bg-edge/50"
            >
              <Icon name="doc" size="xs" />
              <span className="min-w-0 flex-1 truncate text-ui text-ink">{doc.title}</span>
              <span
                className={`shrink-0 rounded px-1.5 py-0.5 text-ui-sm font-medium ${DOC_TONE[doc.kind]}`}
              >
                {doc.kind}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}


function ArtifactsSection({ workstreamId }: { workstreamId: string }) {
  const artifacts = useWorkstreamArtifacts(workstreamId);
  if (artifacts.length === 0) return null;
  return (
    <div className="rounded-lg border border-edge bg-panel-2 p-2">
      <h3 className="px-1 pb-1.5 text-ui font-semibold text-ink-dim">Artifacts</h3>
      <div className="space-y-0.5">
        {artifacts.map((a) => (
          <button
            key={a.url}
            type="button"
            onClick={() => void openUrl(a.url)}
            title={a.description ? `${a.description}\n${a.url}` : a.url}
            className="flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left hover:bg-edge/50"
          >
            <Icon name="globe" size="xs" />
            <span className="min-w-0 flex-1 truncate text-ui text-ink">{a.title}</span>
            <span className="shrink-0 rounded bg-brand/10 px-1.5 py-0.5 text-ui-sm font-medium text-brand">
              artifact
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function MemoryList({
  memory,
  onOpenDoc,
}: {
  memory: ReturnType<typeof useProjectMemory>;
  onOpenDoc: (path: string, title: string) => void;
}) {
  if (memory.isError) return <ErrorNote error={memory.error} />;
  if (!memory.data) {
    return (
      <div className="flex justify-center p-6">
        <Spinner />
      </div>
    );
  }
  const { dir, keyCheck, files } = memory.data;
  if (keyCheck === "mismatch") {
    return (
      <div className="rounded-lg border border-edge bg-panel-2 p-3">
        <p className="text-body font-medium text-warn-ink">Memory unavailable</p>
        <p className="mt-1 text-ui leading-prose text-ink-dim">
          This Claude Code version keeps project files somewhere hyprlayer does not compute, so
          memory is hidden here and codex tabs get none. Claude tabs still read and write it.
        </p>
      </div>
    );
  }
  if (files.length === 0) {
    return (
      <div className="rounded-lg border border-edge bg-panel-2 p-3">
        <p className="text-body font-medium">No memory yet</p>
        <p className="mt-1 text-ui leading-prose text-ink-dim">
          When auto-memory is on, Claude Code writes notes to{" "}
          <span className="break-all font-mono">{dir}</span>, and codex tabs read the index when
          they start.
        </p>
      </div>
    );
  }
  return (
    <div>
      {files.map((file) => (
        <button
          key={file.path}
          type="button"
          onClick={() => onOpenDoc(file.path, file.name)}
          title={file.path}
          className={rowClass({ tier: "compact", className: "flex w-full gap-2 px-2 text-left" })}
        >
          <Icon name="doc" size="xs" />
          <span className="min-w-0 flex-1 truncate text-ui text-ink">{file.name}</span>
          <span className="shrink-0 text-ui-sm text-ink-dim">{formatCompactAge(file.mtimeMs)}</span>
        </button>
      ))}
    </div>
  );
}


function FilesTree({ id, onOpenFile }: { id: string; onOpenFile: (path: string) => void }) {
  return <DirLevel id={id} relPath="" depth={0} onOpenFile={onOpenFile} />;
}

function DirLevel({
  id,
  relPath,
  depth,
  onOpenFile,
}: {
  id: string;
  relPath: string;
  depth: number;
  onOpenFile: (path: string) => void;
}) {
  const dir = useWorktreeDir(id, relPath);
  if (dir.isLoading) {
    return (
      <div className="py-1" style={{ paddingLeft: treeIndent(depth, 8) }}>
        <Spinner size="xs" />
      </div>
    );
  }
  if (dir.isError) {
    return (
      <div className="px-1 py-1">
        <ErrorNote error={dir.error} />
      </div>
    );
  }
  return (
    <>
      {(dir.data ?? []).map((entry) =>
        entry.isDir ? (
          <DirNode key={entry.path} id={id} entry={entry} depth={depth} onOpenFile={onOpenFile} />
        ) : (
          <button
            key={entry.path}
            type="button"
            onClick={() => onOpenFile(entry.path)}
            title={entry.path}
            className={rowClass({ tier: "compact", className: "flex w-full gap-2 text-left" })}
            style={{ paddingLeft: treeIndent(depth + 1, 8) }}
          >
            <FileTypeIcon name={entry.name} />
            <span className="min-w-0 flex-1 truncate text-meta text-ink">{entry.name}</span>
          </button>
        ),
      )}
    </>
  );
}

function DirNode({
  id,
  entry,
  depth,
  onOpenFile,
}: {
  id: string;
  entry: { name: string; path: string };
  depth: number;
  onOpenFile: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={rowClass({ tier: "compact", className: "flex w-full gap-1.5 text-left" })}
        style={{ paddingLeft: treeIndent(depth, 8) }}
      >
        <Icon name={open ? "chevronDown" : "chevronRight"} size="xs" />
        <span className="text-brand">
          <Icon name="folder" size="xs" />
        </span>
        <span className="truncate text-ui text-ink-dim">{entry.name}</span>
      </button>
      {open && <DirLevel id={id} relPath={entry.path} depth={depth + 1} onOpenFile={onOpenFile} />}
    </div>
  );
}


type Props = {
  workstreamId: string | null;
  onOpenDoc: (path: string, title: string) => void;
  onOpenDiff: (path: string, staged: boolean, sha?: string) => void;
  onOpenFile: (path: string) => void;
  onOpenReview?: () => void;
  filesOnly?: boolean;
  baseBranch: string | null;
};

export const ChangesPanel = memo(function ChangesPanel({
  workstreamId,
  onOpenDoc,
  onOpenDiff,
  onOpenFile,
  onOpenReview,
  filesOnly = false,
  baseBranch,
}: Props) {
  const [rawTab, setTab] = useState<"changes" | "commits" | "files" | "memory">("changes");
  const tab = filesOnly && rawTab !== "memory" ? "files" : rawTab;
  const tabs = filesOnly ? (["files"] as const) : (["changes", "commits", "files"] as const);
  const [view, setView] = useState<"list" | "tree">("list");
  const [menu, setMenu] = useState<RowMenu | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState<GitFileEntry | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const id = workstreamId ?? "";
  const status = useWorktreeGitStatus(id, workstreamId !== null && !filesOnly);
  const stage = useStageFiles(id);
  const unstage = useUnstageFiles(id);
  const discard = useDiscardFiles(id);
  const commits = useWorkstreamCommits(id, workstreamId !== null && !filesOnly && tab === "commits");
  const memory = useProjectMemory(id, workstreamId !== null && tab === "memory");
  const memoryMismatch = memory.data?.keyCheck === "mismatch";

  const staged = status.data?.staged ?? [];
  const unstaged = status.data?.unstaged ?? [];
  const changedCount = staged.length + unstaged.length;

  const openMenu = (entry: GitFileEntry, isStaged: boolean, x: number, y: number) =>
    setMenu({ entry, staged: isStaged, x, y });

  return (
    <>
      <div className="flex h-9 items-center justify-between gap-2 border-b border-edge px-3">
        <div className="flex items-center gap-1">
          {tabs.map((t) => (
            <button
              key={t}
              type="button"
              onClick={() => setTab(t)}
              className={`rounded-md px-2 py-1 text-ui font-medium capitalize ${
                tab === t ? "bg-panel-2 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"
              }`}
            >
              {t}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-2.5">
          {tab === "changes" && <Badge tone="dim">{changedCount}</Badge>}
          {tab === "memory" && memory.data && (
            <Badge tone={memoryMismatch ? "warn" : "dim"}>
              {memoryMismatch ? "!" : memory.data.files.length}
            </Badge>
          )}
          {workstreamId !== null && (
            <div className="flex items-center">
              {!filesOnly && onOpenReview && (
                <IconButton onClick={onOpenReview} title="Review changes" aria-label="Review changes">
                  <Icon name="review" size="sm" />
                </IconButton>
              )}
              <IconButton
                active={tab === "memory"}
                onClick={() => setTab("memory")}
                title="Project memory"
                aria-label="Project memory"
              >
                <Icon name="tip" size="sm" />
              </IconButton>
              <IconButton
                onClick={() => {
                  if (tab === "changes") {
                    setRefreshing(true);
                    void status.refetch().finally(() => setRefreshing(false));
                  } else if (tab === "commits") {
                    setRefreshing(true);
                    void commits.refetch().finally(() => setRefreshing(false));
                  } else if (tab === "memory") {
                    setRefreshing(true);
                    void memory.refetch().finally(() => setRefreshing(false));
                  } else {
                    void queryClient.invalidateQueries({ queryKey: ["workstream", id, "dir"] });
                  }
                }}
                disabled={refreshing}
                title="Refresh"
                aria-label="Refresh"
              >
                {refreshing ? <Spinner size="sm" /> : <Icon name="sync" size="sm" />}
              </IconButton>
            </div>
          )}
        </div>
      </div>

      <div className="h-[calc(100%-36px)] overflow-y-auto p-3">
        {workstreamId === null ? (
          <div className="rounded-lg border border-edge bg-panel-2 p-3">
            <p className="text-body font-medium">No workstream selected</p>
            <p className="mt-1 text-ui leading-prose text-ink-dim">
              Create a workstream to see its changes here.
            </p>
          </div>
        ) : tab === "commits" ? (
          commits.isLoading ? (
            <div className="flex justify-center p-6">
              <Spinner />
            </div>
          ) : commits.isError ? (
            <ErrorNote error={commits.error} />
          ) : (commits.data ?? []).length === 0 ? (
            <div className="rounded-lg border border-edge bg-panel-2 p-3">
              <p className="text-body font-medium">No commits yet</p>
              <p className="mt-1 text-ui leading-prose text-ink-dim">
                Commits this workstream adds on top of{" "}
                <span className="font-mono">{baseBranch ?? "its base branch"}</span> show up here.
              </p>
            </div>
          ) : (
            <CommitsPanel id={id} commits={commits.data ?? []} onOpenDiff={onOpenDiff} />
          )
        ) : tab === "memory" ? (
          <MemoryList memory={memory} onOpenDoc={onOpenDoc} />
        ) : tab === "files" ? (
          <FilesTree id={id} onOpenFile={onOpenFile} />
        ) : (
          <div className="space-y-3">
            <ArtifactsSection workstreamId={id} />
            <DocsSection workstreamId={id} onOpenDoc={onOpenDoc} />
            {status.isLoading ? (
              <div className="flex justify-center p-6">
                <Spinner />
              </div>
            ) : status.isError ? (
              <ErrorNote error={status.error} />
            ) : changedCount === 0 ? (
              <div className="rounded-lg border border-edge bg-panel-2 p-3">
                <p className="text-body font-medium">No changes yet</p>
                <p className="mt-1 text-ui leading-prose text-ink-dim">
                  Edits in this workstream&apos;s worktree show up here.
                </p>
              </div>
            ) : (
              <>
                <div className="flex justify-end">
                  <div className="flex overflow-hidden rounded-md border border-edge">
                    {(["list", "tree"] as const).map((v) => (
                      <button
                        key={v}
                        type="button"
                        onClick={() => setView(v)}
                        className={`px-2 py-0.5 text-ui font-medium capitalize ${
                          view === v ? "bg-panel-2 text-ink" : "text-ink-dim hover:bg-panel-2"
                        }`}
                      >
                        {v}
                      </button>
                    ))}
                  </div>
                </div>
                <ChangeSection
                  title="Unstaged"
                  entries={unstaged}
                  staged={false}
                  view={view}
                  actionLabel="Stage all"
                  onActionAll={() => stage.mutate(unstaged.map((e) => e.path))}
                  onOpen={(e) => onOpenDiff(e.path, false)}
                  onToggle={(e) => stage.mutate([e.path])}
                  onMenu={openMenu}
                />
                <ChangeSection
                  title="Staged"
                  entries={staged}
                  staged={true}
                  view={view}
                  actionLabel="Unstage all"
                  onActionAll={() => unstage.mutate(staged.map((e) => e.path))}
                  onOpen={(e) => onOpenDiff(e.path, true)}
                  onToggle={(e) => unstage.mutate([e.path])}
                  onMenu={openMenu}
                />
              </>
            )}
          </div>
        )}
      </div>

      {menu && (
        <ContextMenu position={{ x: menu.x, y: menu.y }} onClose={() => setMenu(null)}>
          {menu.staged ? (
            <ContextMenuItem onClick={() => unstage.mutate([menu.entry.path])}>
              Unstage
            </ContextMenuItem>
          ) : (
            <ContextMenuItem onClick={() => stage.mutate([menu.entry.path])}>Stage</ContextMenuItem>
          )}
          <ContextMenuItem onClick={() => onOpenDiff(menu.entry.path, menu.staged)}>
            Open diff
          </ContextMenuItem>
          {!menu.staged && (
            <ContextMenuItem danger onClick={() => setConfirmDiscard(menu.entry)}>
              Discard changes…
            </ContextMenuItem>
          )}
        </ContextMenu>
      )}

      <ConfirmDialog
        open={confirmDiscard !== null}
        onClose={() => {
          if (discard.isPending) return;
          setConfirmDiscard(null);
        }}
        title="Discard changes?"
        confirmLabel="Discard"
        confirmVariant="danger"
        busy={discard.isPending}
        error={discard.isError ? discard.error : null}
        onDismissError={() => discard.reset()}
        onConfirm={() => {
          if (!confirmDiscard) return;
          discard.mutate([confirmDiscard.path], { onSettled: () => setConfirmDiscard(null) });
        }}
      >
        <p className="text-body text-ink-dim">
          Discard working-tree changes to{" "}
          <span className="font-mono text-ink">
            {confirmDiscard ? basename(confirmDiscard.path) : ""}
          </span>
          ? This cannot be undone.
        </p>
      </ConfirmDialog>
    </>
  );
});
