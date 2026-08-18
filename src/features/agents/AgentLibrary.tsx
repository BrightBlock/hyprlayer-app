/**
 * The Agents view's state owner: lists the `.claude/agents/*.md` definitions,
 * keeps the editor's working copy, and writes each save back to its own file.
 *
 * The files ARE the agents, so every definition change is a filesystem write
 * followed by a refetch; the launch settings claude has no frontmatter key for
 * ride in `AppSettings.agents`, keyed by the same path.
 */

import { useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { Button, ContextMenu, ContextMenuItem, Dialog, PageSpinner, TextInput } from "@/components/ui";
import {
  commands,
  type AgentFile,
  type AgentLibrarySettings,
  type AppSettings,
  type Bootstrap,
} from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { basename } from "@/lib/path";
import { useBootstrap, useSetAppSettings } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import {
  agentRows,
  DEFAULT_OVERLAY,
  dotColor,
  duplicateAgent,
  folderList,
  folderResolver,
  isSystem,
  isUnsaved,
  newAgent,
  overlayFor,
  visibleAgents,
  type AgentScope,
  type AgentTab,
  type EditorTab,
  type Overlay,
} from "./agents";
import { AgentEditor } from "./AgentEditor";
import { AgentList } from "./AgentList";
import { FolderCombo } from "./parts";

/** What `writeLibrary` accepts: any subset of the library's own fields. */
type LibraryPatch = Partial<AgentLibrarySettings>;

/** A move that gave up partway. Carries the overlay map as far as it got, so
 *  the plugin agents that did move aren't rolled back with the failure. */
class MoveFailed extends Error {
  constructor(
    message: string,
    readonly overlays: NonNullable<AgentLibrarySettings["overlays"]>,
  ) {
    super(message);
    this.name = "MoveFailed";
  }
}

type Props = {
  /** The project whose `.claude/agents` is in scope, or null. */
  projectPath: string | null;
  /** Path of the open agent (`?agent=`), or null for the list. */
  selectedPath: string | null;
  onSelect: (path: string | null) => void;
  editorTab: EditorTab;
  onEditorTab: (tab: EditorTab) => void;
};

/** The unsaved-new-agent sentinel, so the editor can open before a file exists. */
const NEW_AGENT_PATH = "new";

export function AgentLibrary({
  projectPath,
  selectedPath,
  onSelect,
  editorTab,
  onEditorTab,
}: Props) {
  const queryClient = useQueryClient();
  const bootstrap = useBootstrap();
  const setSettings = useSetAppSettings();
  const settings = bootstrap.data?.settings;
  const library = settings?.agents;

  const agents = useQuery({
    queryKey: ["agent-files", projectPath],
    queryFn: () => unwrap(commands.listAgentFiles(projectPath)),
  });
  const contextFiles = useQuery({
    queryKey: ["agent-context-files", projectPath],
    queryFn: () => unwrap(commands.agentContextFiles(projectPath)),
  });
  const mcpServers = useQuery({
    queryKey: ["agent-mcp-servers", projectPath],
    queryFn: () => unwrap(commands.agentMcpServers(projectPath)),
  });
  const skills = useQuery({
    queryKey: ["agent-skills", projectPath],
    queryFn: () => unwrap(commands.agentSkills(projectPath)),
  });

  const [tab, setTab] = useState<AgentTab>("yours");
  const [scope, setScope] = useState<AgentScope>("global");
  const [search, setSearch] = useState("");
  const [menu, setMenu] = useState<{ agent: AgentFile; x: number; y: number } | null>(null);
  const [dialog, setDialog] = useState<"folder" | "move" | "delete" | null>(null);
  const [folderName, setFolderName] = useState("");
  const [moveTarget, setMoveTarget] = useState("");
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  // The editor's working copy plus the baseline it diverged from.
  const [draft, setDraft] = useState<{
    base: { agent: AgentFile; overlay: Overlay };
    agent: AgentFile;
    overlay: Overlay;
  } | null>(null);

  /** The last settings payload dispatched, until it settles, see `writeLibrary`. */
  const queued = useRef<AppSettings | null>(null);

  const projectName = projectPath ? basename(projectPath) : null;
  const all = useMemo(() => agents.data ?? [], [agents.data]);
  const groupUncategorized = library?.groupUncategorized ?? true;
  // Everything this tab + scope admits, before the search box narrows it. The
  // folder pickers list from here: a search filters which ROWS you see, never
  // which folders you may file into.
  const inScope = useMemo(
    () => visibleAgents(all, { tab, scope, search: "" }),
    [all, tab, scope],
  );
  const visible = useMemo(
    () => (search.trim() ? visibleAgents(all, { tab, scope, search }) : inScope),
    [all, tab, scope, search, inScope],
  );
  const folderOf = useMemo(
    () => folderResolver(library, groupUncategorized),
    [library, groupUncategorized],
  );
  const folders = useMemo(
    () => folderList(inScope, library, groupUncategorized),
    [inScope, library, groupUncategorized],
  );
  // A folder absent here, including one that appears later, renders open,
  // so the list arrives expanded rather than needing every folder clicked
  // open on first visit.
  const closedFolders = useMemo(() => new Set(library?.closedFolders ?? []), [library]);
  const rows = useMemo(
    () => agentRows({ agents: visible, folders, closedFolders, folderOf }),
    [visible, folders, closedFolders, folderOf],
  );
  const chosen = useMemo(() => visible.filter((a) => selected.has(a.path)), [visible, selected]);

  const stored = selectedPath ? (all.find((a) => a.path === selectedPath) ?? null) : null;
  const live =
    draft && (draft.agent.path === selectedPath || selectedPath === NEW_AGENT_PATH) ? draft : null;
  const working = live?.agent ?? stored ?? null;
  const workingOverlay = live?.overlay ?? (stored ? overlayFor(library, stored.path) : null);
  const baseline = live?.base ?? (stored ? { agent: stored, overlay: overlayFor(library, stored.path) } : null);

  const flash = (message: string) => useToast.getState().show("Agents", message);
  const fail = (title: string, e: unknown) =>
    useToast.getState().show(title, String(e), { tone: "bad" });
  const refetch = () => queryClient.invalidateQueries({ queryKey: ["agent-files"] });

  /**
   * Fold a change into the persisted agent-library settings.
   *
   * `patch` may be a function of the CURRENT library rather than a literal.
   * Take that form whenever the new value is derived from the old one: two
   * clicks in one tick share a render, so a literal patch built from the
   * `library` in scope would compute both from the same pre-click state and
   * the second would silently revert the first. The updater reads the query
   * cache at call time, which `useSetAppSettings` keeps current from
   * `onMutate`, before the round-trip, not after it.
   */
  // Only `library.folders` is ever written back, so a derived phase name (or a
  // plugin's) never gets baked into settings as if the user had created it.
  const writeLibrary = (
    patch: LibraryPatch | ((current: AgentLibrarySettings) => LibraryPatch),
  ) => {
    // `queued` is the synchronous part. `useSetAppSettings` is scoped, so its
    // mutations QUEUE, `onMutate` for the second does not run until the first
    // has landed, which means neither React state nor the query cache has
    // moved when two clicks arrive in one tick. Both would read the same base
    // and the later would revert the earlier. The ref is the only thing that
    // updates in time, so each call folds onto the last payload dispatched.
    const base = queued.current ?? queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings ?? settings;
    if (!base) return;
    const current: AgentLibrarySettings = {
      folders: base.agents?.folders ?? [],
      overlays: base.agents?.overlays ?? {},
      defaultAgent: base.agents?.defaultAgent ?? null,
      groupUncategorized: base.agents?.groupUncategorized ?? true,
      closedFolders: base.agents?.closedFolders ?? [],
    };
    const next = {
      ...base,
      agents: { ...current, ...(typeof patch === "function" ? patch(current) : patch) },
    };
    queued.current = next;
    setSettings.mutate(next, {
      // Only the last write in a burst clears the baton; an earlier one
      // settling must not drop a payload still queued behind it.
      onSettled: () => {
        if (queued.current === next) queued.current = null;
      },
    });
  };

  const save = useMutation({
    mutationFn: async (next: { agent: AgentFile; overlay: Overlay; isDefault: boolean }) => {
      // A new agent, or the first edit of a plugin's, has no file of its own
      // yet; ask the backend where its name and scope put it.
      let path = next.agent.path;
      if (!path || (isSystem(next.agent) && !next.agent.modified)) {
        const resolved = await unwrap(
          commands.newAgentFilePath(
            next.agent.name,
            next.agent.source === "project" ? "project" : "user",
            projectPath,
          ),
        );
        if (!resolved) throw new Error("No .claude/agents directory for that scope.");
        path = resolved;
      }
      const saved = await unwrap(commands.saveAgentFile({ ...next.agent, path }));
      // The old file goes only when the scope toggle moved the agent.
      if (next.agent.path && next.agent.path !== path && !isSystem(next.agent)) {
        await unwrap(commands.deleteAgentFile(next.agent.path));
      }
      return { saved, overlay: next.overlay, isDefault: next.isDefault, from: next.agent.path };
    },
    onSuccess: ({ saved, overlay, isDefault, from }) => {
      const overlays = { ...(library?.overlays ?? {}) };
      if (from && from !== saved.path) delete overlays[from];
      overlays[saved.path] = overlay;
      const wasDefault = library?.defaultAgent;
      writeLibrary({
        overlays,
        defaultAgent: isDefault ? saved.path : wasDefault === from || wasDefault === saved.path ? null : wasDefault,
      });
      setDraft(null);
      void refetch();
      flash(`“${saved.name}” saved`);
      onSelect(null);
    },
    onError: (e) => fail("Save failed", e),
  });

  const remove = useMutation({
    mutationFn: async (batch: AgentFile[]) => {
      // Deleting a modified system agent removes the override, not the plugin's
      // own file, the shipped version comes back.
      for (const agent of batch) await unwrap(commands.deleteAgentFile(agent.path));
      return batch;
    },
    onSuccess: (batch) => {
      const overlays = { ...(library?.overlays ?? {}) };
      for (const a of batch) delete overlays[a.path];
      writeLibrary({ overlays });
      setDialog(null);
      setSelected(new Set());
      void refetch();
      if (batch.some((a) => a.path === selectedPath)) onSelect(null);
      flash(`Deleted ${batch.length} ${batch.length === 1 ? "agent" : "agents"}`);
    },
    onError: (e) => fail("Delete failed", e),
  });

  const duplicate = useMutation({
    mutationFn: async (batch: AgentFile[]) => {
      for (const agent of batch) {
        const copy = duplicateAgent(agent);
        const path = await unwrap(
          commands.newAgentFilePath(copy.name, copy.source, projectPath),
        );
        if (!path) throw new Error("No .claude/agents directory for that scope.");
        await unwrap(commands.saveAgentFile({ ...copy, path }));
      }
      return batch.length;
    },
    onSuccess: (count) => {
      setSelected(new Set());
      void refetch();
      flash(`Duplicated ${count} ${count === 1 ? "agent" : "agents"}`);
    },
    onError: (e) => fail("Duplicate failed", e),
  });

  const moveToFolder = useMutation({
    mutationFn: async ({ batch, folder }: { batch: AgentFile[]; folder: string }) => {
      // An owned agent's folder is its file's own `category`, a real write,
      // one IPC call per agent. A plugin agent has no file of its own to
      // carry it, so it keeps moving via the overlay, app-side only.
      const overlays = { ...(library?.overlays ?? {}) };
      try {
        for (const agent of batch) {
          if (isSystem(agent)) {
            overlays[agent.path] = { ...overlayFor(library, agent.path), folder };
          } else {
            await unwrap(commands.saveAgentFile({ ...agent, category: folder.trim() || null }));
            // `category` outranks `overlay.folder`, so a stale overlay left
            // behind would silently win back the moment the category is
            // cleared, and moving to "no folder" clears it. Retire it here,
            // touching settings only for an agent that actually has one.
            const legacy = library?.overlays?.[agent.path];
            if (legacy?.folder) overlays[agent.path] = { ...legacy, folder: "" };
          }
        }
      } catch (e) {
        // Keep the overlays for whatever already moved: half a batch is on
        // disk, and dropping the app-side half would desync the two.
        throw new MoveFailed(String(e), overlays);
      }
      return overlays;
    },
    onSuccess: (overlays, { folder }) => {
      writeLibrary({ overlays });
      flash(`Moved to ${folder.trim() || "no folder"}`);
    },
    onError: (e) => {
      if (e instanceof MoveFailed) writeLibrary({ overlays: e.overlays });
      fail("Move failed", e instanceof MoveFailed ? e.message : e);
    },
    // Whether or not the batch finished, some files may already be on disk,
    // the list has to be re-read rather than left showing the old folders.
    onSettled: () => {
      setSelected(new Set());
      setDialog(null);
      void refetch();
    },
  });

  const exportAgents = useMutation({
    mutationFn: async (batch: AgentFile[]) => {
      const paths: string[] = [];
      for (const agent of batch) {
        paths.push(await unwrap(commands.exportAgentFile(agent, overlayFor(library, agent.path))));
      }
      return paths;
    },
    onSuccess: (paths) => {
      setSelected(new Set());
      flash(paths.length === 1 ? `Exported to ${paths[0]}` : `Exported ${paths.length} agents`);
    },
    onError: (e) => fail("Export failed", e),
  });

  const openDraft = (agent: AgentFile, overlay: Overlay, path: string) => {
    setDraft({ base: { agent, overlay }, agent, overlay });
    onSelect(path);
    onEditorTab("setup");
  };

  // One functional update behind both patches: a handler that touches the file
  // and the overlay together (the Skills card) would otherwise have the second
  // call overwrite the first with this render's stale copy.
  const patchDraft = (p: { agent?: Partial<AgentFile>; overlay?: Partial<Overlay> }) =>
    setDraft((prev) => {
      const from =
        prev && (prev.agent.path === selectedPath || selectedPath === NEW_AGENT_PATH)
          ? prev
          : working && workingOverlay && baseline
            ? { base: baseline, agent: working, overlay: workingOverlay }
            : null;
      if (!from) return prev;
      return {
        base: from.base,
        agent: p.agent ? { ...from.agent, ...p.agent } : from.agent,
        overlay: p.overlay ? { ...from.overlay, ...p.overlay } : from.overlay,
      };
    });

  const patch = (p: Partial<AgentFile>) => patchDraft({ agent: p });
  const patchOverlay = (p: Partial<Overlay>) => patchDraft({ overlay: p });

  // "Default" is library-level (only one agent holds it), so the editor toggle
  // tracks a local intent until save rather than patching the draft.
  const [defaultIntent, setDefaultIntent] = useState<boolean | null>(null);
  const isDefault = defaultIntent ?? (working ? library?.defaultAgent === working.path : false);

  if (agents.isLoading || bootstrap.isLoading) return <PageSpinner />;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {working && workingOverlay && baseline ? (
        <AgentEditor
          draft={working}
          overlay={workingOverlay}
          isDefault={isDefault}
          patch={patch}
          patchOverlay={patchOverlay}
          onDefault={setDefaultIntent}
          dirty={
            JSON.stringify({ a: working, o: workingOverlay }) !==
              JSON.stringify({ a: baseline.agent, o: baseline.overlay }) ||
            isUnsaved(working) ||
            (defaultIntent !== null && defaultIntent !== (library?.defaultAgent === working.path))
          }
          tab={editorTab}
          onTab={onEditorTab}
          onBack={() => {
            setDraft(null);
            setDefaultIntent(null);
            onSelect(null);
          }}
          onSave={() => save.mutate({ agent: working, overlay: workingOverlay, isDefault })}
          onPublish={() => patchOverlay({ status: "published" })}
          onReset={() => {
            if (working.modified) remove.mutate([working]);
            else flash("This agent is already the shipped version");
          }}
          onExport={() => exportAgents.mutate([working])}
          onCopyPath={() => {
            void navigator.clipboard.writeText(working.path);
            flash("Agent path copied");
          }}
          saving={save.isPending}
          projectName={projectName}
          folders={folders}
          contextFiles={contextFiles.data ?? []}
          mcpServers={mcpServers.data ?? []}
          skills={skills.data ?? []}
          peers={all.filter((a) => a.path !== working.path)}
        />
      ) : (
        <AgentList
          rows={rows}
          agents={visible}
          library={library}
          defaultAgent={library?.defaultAgent ?? null}
          tab={tab}
          onTab={(next) => {
            setTab(next);
            setSelected(new Set());
          }}
          scope={scope}
          onScope={setScope}
          projectName={projectName}
          search={search}
          onSearch={setSearch}
          selected={selected}
          onToggleSelect={(path) =>
            setSelected((prev) => {
              const next = new Set(prev);
              if (!next.delete(path)) next.add(path);
              return next;
            })
          }
          onToggleAll={() =>
            setSelected((prev) =>
              visible.every((a) => prev.has(a.path))
                ? new Set()
                : new Set(visible.map((a) => a.path)),
            )
          }
          onClearSelection={() => setSelected(new Set())}
          onToggleFolder={(folder) =>
            writeLibrary((current) => {
              const next = new Set(current.closedFolders ?? []);
              if (!next.delete(folder)) next.add(folder);
              return { closedFolders: [...next] };
            })
          }
          loading={agents.isFetching}
          dotColorOf={(agent) => dotColor(agent, overlayFor(library, agent.path), folderOf)}
          groupUncategorized={groupUncategorized}
          onGroupUncategorized={(on) => writeLibrary({ groupUncategorized: on })}
          onOpen={(agent) => {
            setDefaultIntent(null);
            onSelect(agent.path);
          }}
          onMenu={(agent, at) => setMenu({ agent, ...at })}
          onCreate={() => {
            setDefaultIntent(null);
            openDraft(
              newAgent(scope === "project" && projectName ? "project" : "user"),
              { ...DEFAULT_OVERLAY, status: "draft" },
              NEW_AGENT_PATH,
            );
          }}
          onNewFolder={() => {
            setFolderName("");
            setDialog("folder");
          }}
          bulk={{
            duplicate: () => duplicate.mutate(chosen),
            move: () => {
              setMoveTarget("");
              setDialog("move");
            },
            export: () => exportAgents.mutate(chosen),
            remove: () => setDialog("delete"),
          }}
        />
      )}

      {menu && (
        <ContextMenu position={{ x: menu.x, y: menu.y }} onClose={() => setMenu(null)}>
          <ContextMenuItem
            onClick={() => {
              setMenu(null);
              onSelect(menu.agent.path);
            }}
          >
            Open
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setMenu(null);
              duplicate.mutate([menu.agent]);
            }}
          >
            Duplicate
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setMenu(null);
              writeLibrary({ defaultAgent: menu.agent.path });
              flash("Default agent updated");
            }}
          >
            Set as default
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setSelected(new Set([menu.agent.path]));
              setMoveTarget("");
              setMenu(null);
              setDialog("move");
            }}
          >
            Move to folder…
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setMenu(null);
              exportAgents.mutate([menu.agent]);
            }}
          >
            Export .agent.json
          </ContextMenuItem>
          <ContextMenuItem
            danger
            onClick={() => {
              setSelected(new Set([menu.agent.path]));
              setMenu(null);
              setDialog("delete");
            }}
          >
            {menu.agent.modified ? "Restore shipped version" : "Delete agent"}
          </ContextMenuItem>
        </ContextMenu>
      )}

      <Dialog open={dialog === "folder"} onClose={() => setDialog(null)} title="New folder">
        <div className="flex flex-col gap-3">
          <TextInput
            value={folderName}
            onChange={(e) => setFolderName(e.target.value)}
            placeholder="e.g. Review"
            aria-label="Folder name"
          />
          <div className="flex justify-end gap-2">
            <Button onClick={() => setDialog(null)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={!folderName.trim() || folders.includes(folderName.trim())}
              onClick={() => {
                const name = folderName.trim();
                // No need to also touch closedFolders, a folder absent from
                // it already renders open, and this one is brand new.
                writeLibrary((current) => ({ folders: [...(current.folders ?? []), name] }));
                setDialog(null);
                flash(`Folder “${name}” created`);
              }}
            >
              Create folder
            </Button>
          </div>
        </div>
      </Dialog>

      <Dialog open={dialog === "move"} onClose={() => setDialog(null)} title="Move to folder">
        <div className="flex flex-col gap-3">
          <FolderCombo
            id="move-target"
            value={moveTarget}
            onChange={setMoveTarget}
            folders={folders}
            placeholder="No folder"
            aria-label="Target folder"
          />
          <div className="flex justify-end gap-2">
            <Button onClick={() => setDialog(null)}>Cancel</Button>
            <Button
              variant="primary"
              busy={moveToFolder.isPending}
              onClick={() => moveToFolder.mutate({ batch: chosen, folder: moveTarget })}
            >
              Move
            </Button>
          </div>
        </div>
      </Dialog>

      <Dialog open={dialog === "delete"} onClose={() => setDialog(null)} title="Delete agents">
        <div className="flex flex-col gap-3.5">
          <p className="text-sm leading-relaxed text-ink-dim">
            {chosen.length === 1
              ? "This deletes the file from disk:"
              : `This deletes ${chosen.length} files from disk:`}
          </p>
          <ul className="flex max-h-40 flex-col gap-1 overflow-y-auto">
            {chosen.map((a) => (
              <li key={a.path} className="truncate font-mono text-xs text-ink">
                {a.path}
              </li>
            ))}
          </ul>
          <p className="text-xs text-ink-dim">
            Sessions already running are unaffected; a modified system agent goes back to the
            version its plugin ships.
          </p>
          <div className="flex justify-end gap-2">
            <Button onClick={() => setDialog(null)}>Cancel</Button>
            <Button
              variant="danger"
              busy={remove.isPending}
              onClick={() => remove.mutate(chosen)}
            >
              Delete
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}
