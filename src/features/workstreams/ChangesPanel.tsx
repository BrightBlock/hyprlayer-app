import { memo, useMemo, useState } from "react";

import type { GitFileEntry } from "@/lib/bindings";
import {
  Badge,
  Button,
  ContextMenu,
  ContextMenuItem,
  Dialog,
  ErrorNote,
  IconButton,
  Spinner,
} from "@/components/ui";
import { ChevronDownIcon, ChevronRightIcon, DocIcon, FolderIcon } from "@/components/icons";
import { FileTypeIcon } from "@/components/fileIcon";
import { basename } from "@/lib/path";
import { queryClient } from "@/lib/queryClient";
import {
  useDiscardFiles,
  useStageFiles,
  useUnstageFiles,
  useWorktreeDir,
  useWorktreeGitStatus,
} from "@/lib/queries";
import { useWorkstreamDocs } from "@/features/session/sessionStore";
import type { DocKind } from "@/lib/domain/docs";

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

/** Down arrow → stage; up arrow → unstage. */
function MoveIcon({ dir }: { dir: "down" | "up" }) {
  return (
    <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none">
      <path
        d={dir === "down" ? "M12 5v14M6 13l6 6 6-6" : "M12 19V5M6 11l6-6 6 6"}
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

// Status glyph + color per change kind (the colored badge before each path).
const GLYPH: Record<string, { ch: string; cls: string; label: string }> = {
  added: { ch: "+", cls: "text-ok", label: "added" },
  untracked: { ch: "+", cls: "text-ok", label: "untracked" },
  modified: { ch: "M", cls: "text-accent", label: "modified" },
  deleted: { ch: "−", cls: "text-bad", label: "deleted" },
  renamed: { ch: "R", cls: "text-warn", label: "renamed" },
  typechange: { ch: "T", cls: "text-warn", label: "typechange" },
  conflict: { ch: "!", cls: "text-bad", label: "conflict" },
};

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
  const glyph = GLYPH[entry.change] ?? { ch: "?", cls: "text-ink-dim", label: entry.change };
  return (
    <div
      className="group flex h-7 items-center gap-2 rounded-md pr-1 text-left hover:bg-edge/40"
      style={{ paddingLeft: 6 + depth * 14 }}
      onClick={onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu(e.clientX, e.clientY);
      }}
      role="button"
    >
      <span
        className={`w-3 shrink-0 text-center font-mono text-xs font-semibold ${glyph.cls}`}
        title={glyph.label}
      >
        {glyph.ch}
      </span>
      <FileTypeIcon name={entry.path} size={13} />
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink" title={entry.path}>
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
        <MoveIcon dir={staged ? "up" : "down"} />
      </button>
    </div>
  );
});

// ── Tree mode ──────────────────────────────────────────────────────────────

export type TreeNode = {
  name: string;
  path: string;
  entry?: GitFileEntry; // leaf
  children: Map<string, TreeNode>;
};

export function buildTree(entries: GitFileEntry[]): TreeNode {
  const root: TreeNode = { name: "", path: "", children: new Map() };
  for (const entry of entries) {
    // Drop empty segments so a trailing slash (e.g. a collapsed untracked-dir
    // entry `crates/`) never spawns a nameless phantom leaf under the folder.
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
  // Folders first, then files; each alphabetical.
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
            label={child.name}
            onOpen={() => onOpen(child.entry!)}
            onToggle={() => onToggle(child.entry!)}
            onMenu={(x, y) => onMenu(child.entry!, x, y)}
          />
        ) : (
          <div key={child.path}>
            <button
              type="button"
              onClick={() => toggleCollapsed(child.path)}
              className="flex h-7 w-full items-center gap-1 rounded-md text-left hover:bg-edge/40"
              style={{ paddingLeft: 6 + depth * 14 }}
            >
              {collapsed.has(child.path) ? (
                <ChevronRightIcon size={12} />
              ) : (
                <ChevronDownIcon size={12} />
              )}
              <span className="truncate text-xs text-ink-dim">{child.name}</span>
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

// ── Changes-list section (one of unstaged / staged) ──────────────────────────

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
        <span className="text-xs font-semibold uppercase tracking-wide text-ink-dim">
          {title} {entries.length}
        </span>
        {entries.length > 0 && (
          <button
            type="button"
            onClick={onActionAll}
            className="rounded px-1.5 py-0.5 text-xs text-ink-dim hover:bg-edge/60 hover:text-ink"
          >
            {actionLabel}
          </button>
        )}
      </div>
      {entries.length === 0 ? (
        <p className="px-1 pb-1 text-xs text-ink-dim/70">Nothing here.</p>
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
            label={entry.path}
            onOpen={() => onOpen(entry)}
            onToggle={() => onToggle(entry)}
            onMenu={(x, y) => onMenu(entry, staged, x, y)}
          />
        ))
      )}
    </div>
  );
}

// ── Documents section (workstream research/plan docs) ────────────────────────

// Doc-type accent hues bind themed tokens (research/plan share the matching
// pipeline-stage tokens; handoff its own) so the chips recolor per theme and
// stay legible on the light surface instead of washing out. See src/index.css.
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
            <span
              className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${DOC_TONE[doc.kind]}`}
            >
              {doc.kind}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

// ── Files tree tab (lazy, whole worktree) ────────────────────────────────────

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
      <div className="py-1" style={{ paddingLeft: 8 + depth * 14 }}>
        <Spinner size={12} />
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
            className="flex h-7 w-full items-center gap-2 rounded-md text-left hover:bg-edge/40"
            style={{ paddingLeft: 8 + depth * 14 + 14 }}
          >
            <FileTypeIcon name={entry.name} size={13} />
            <span className="min-w-0 flex-1 truncate font-mono text-xs text-ink">{entry.name}</span>
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
        className="flex h-7 w-full items-center gap-1.5 rounded-md text-left hover:bg-edge/40"
        style={{ paddingLeft: 8 + depth * 14 }}
      >
        {open ? <ChevronDownIcon size={12} /> : <ChevronRightIcon size={12} />}
        <span className="text-brand">
          <FolderIcon size={13} />
        </span>
        <span className="truncate text-xs text-ink-dim">{entry.name}</span>
      </button>
      {open && <DirLevel id={id} relPath={entry.path} depth={depth + 1} onOpenFile={onOpenFile} />}
    </div>
  );
}

// ── Panel ────────────────────────────────────────────────────────────────────

type Props = {
  workstreamId: string | null;
  onOpenDoc: (path: string, title: string) => void;
  onOpenDiff: (path: string, staged: boolean) => void;
  onOpenFile: (path: string) => void;
  /** Folder project: no git, so only the file tree, no Changes tab, no status query. */
  filesOnly?: boolean;
};

/** Right-hand source-control panel: a Changes tab (split staged/unstaged with
 *  stage / unstage / discard actions) and a Files tab (lazy worktree tree).
 *  With `filesOnly` it collapses to just the tree, the browse-and-edit surface
 *  a folder project still needs. */
export function ChangesPanel({
  workstreamId,
  onOpenDoc,
  onOpenDiff,
  onOpenFile,
  filesOnly = false,
}: Props) {
  const [rawTab, setTab] = useState<"changes" | "files">("changes");
  const tab = filesOnly ? "files" : rawTab;
  const [view, setView] = useState<"list" | "tree">("list");
  const [menu, setMenu] = useState<RowMenu | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState<GitFileEntry | null>(null);
  // Whether the USER asked for this refresh. Deliberately NOT `status.isFetching`:
  // the backend's worktree watcher invalidates git-status on every file change
  // (see lib/events → worktreeChanged), so during an agent turn or a build that
  // flag strobes true/false several times a second, and the button visibly
  // flickered between its glyph and the spinner, dimming on each disabled edge.
  // Background refreshes now update the list silently, as they should.
  const [refreshing, setRefreshing] = useState(false);

  const id = workstreamId ?? "";
  // Never ask git about a folder project, there's no repo to answer.
  const status = useWorktreeGitStatus(id, workstreamId !== null && !filesOnly);
  const stage = useStageFiles(id);
  const unstage = useUnstageFiles(id);
  const discard = useDiscardFiles(id);

  const staged = status.data?.staged ?? [];
  const unstaged = status.data?.unstaged ?? [];
  const changedCount = staged.length + unstaged.length;

  const openMenu = (entry: GitFileEntry, isStaged: boolean, x: number, y: number) =>
    setMenu({ entry, staged: isStaged, x, y });

  return (
    <>
      <div className="flex h-9 items-center justify-between gap-2 border-b border-edge px-3">
        <div className="flex items-center gap-1">
          {filesOnly ? (
            <span className="px-2 py-1 text-xs font-medium text-ink">Files</span>
          ) : (
            (["changes", "files"] as const).map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => setTab(t)}
                className={`rounded-md px-2 py-1 text-xs font-medium capitalize ${
                  tab === t ? "bg-panel-2 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"
                }`}
              >
                {t}
              </button>
            ))
          )}
        </div>
        <div className="flex items-center gap-1.5">
          {tab === "changes" && <Badge tone="dim">{changedCount}</Badge>}
          {workstreamId !== null && (
            <IconButton
              onClick={() => {
                if (tab === "changes") {
                  setRefreshing(true);
                  void status.refetch().finally(() => setRefreshing(false));
                } else {
                  // The tree is lazy per directory level; drop them all so
                  // reopening re-reads from disk.
                  void queryClient.invalidateQueries({ queryKey: ["workstream", id, "dir"] });
                }
              }}
              disabled={refreshing}
              title="Refresh"
              aria-label="Refresh"
            >
              {refreshing ? <Spinner size={14} /> : <RefreshIcon />}
            </IconButton>
          )}
        </div>
      </div>

      <div className="h-[calc(100%-36px)] overflow-y-auto p-3">
        {workstreamId === null ? (
          <div className="rounded-lg border border-edge bg-panel-2 p-3">
            <p className="text-sm font-medium">No workstream selected</p>
            <p className="mt-1 text-xs leading-relaxed text-ink-dim">
              Create a workstream to see its changes here.
            </p>
          </div>
        ) : tab === "files" ? (
          <FilesTree id={id} onOpenFile={onOpenFile} />
        ) : (
          <div className="space-y-3">
            <DocsSection workstreamId={id} onOpenDoc={onOpenDoc} />
            {status.isLoading ? (
              <div className="flex justify-center p-6">
                <Spinner />
              </div>
            ) : status.isError ? (
              <ErrorNote error={status.error} />
            ) : changedCount === 0 ? (
              <div className="rounded-lg border border-edge bg-panel-2 p-3">
                <p className="text-sm font-medium">No changes yet</p>
                <p className="mt-1 text-xs leading-relaxed text-ink-dim">
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
                        className={`px-2 py-0.5 text-xs font-medium capitalize ${
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

      <Dialog
        open={confirmDiscard !== null}
        onClose={() => setConfirmDiscard(null)}
        title="Discard changes?"
      >
        <p className="text-sm text-ink-dim">
          Discard working-tree changes to{" "}
          <span className="font-mono text-ink">
            {confirmDiscard ? basename(confirmDiscard.path) : ""}
          </span>
          ? This cannot be undone.
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <Button onClick={() => setConfirmDiscard(null)}>Cancel</Button>
          <Button
            variant="danger"
            onClick={() => {
              if (confirmDiscard) discard.mutate([confirmDiscard.path]);
              setConfirmDiscard(null);
            }}
          >
            Discard
          </Button>
        </div>
      </Dialog>
    </>
  );
}
