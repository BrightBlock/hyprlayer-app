
import { useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { Button, Card, ContextMenu, ContextMenuItem, Dialog, PageSpinner, TextInput } from "@/components/ui";
import { LayersIcon } from "@/components/icons";
import {
  commands,
  type AgentFile,
  type AgentLibrarySettings,
  type AppSettings,
  type Bootstrap,
  type Project,
} from "@/lib/bindings";
import { errorMessage, errorRemedy, unwrap } from "@/lib/ipc";
import { basename } from "@/lib/path";
import {
  invalidateAgentBundle,
  useAgentBundle,
  useBootstrap,
  useSetAppSettings,
} from "@/lib/queries";
import { startEdgeDrag } from "@/lib/resize";
import { useToast } from "@/lib/toast";
import { SIDE_PANEL_MAX, SIDE_PANEL_MIN, useUiStore } from "@/lib/uiStore";
import { useViewport } from "@/lib/useViewport";
import {
  agentRows,
  bundleChipState,
  DEFAULT_OVERLAY,
  dotColor,
  duplicateAgent,
  folderList,
  folderResolver,
  isManaged,
  isSystem,
  isUnsaved,
  newAgent,
  overlayFor,
  visibleAgents,
  type AgentScope,
  type AgentSection,
  type AgentTab,
  type BundleFailure,
  type BundleReport,
  type EditorTab,
  type Overlay,
} from "./agents";
import { AgentEditor } from "./AgentEditor";
import { AgentUpdateFailed, AgentUpdateReport } from "./AgentBundleDialogs";
import { AgentSidebar } from "./AgentSidebar";
import { FolderCombo, SoonBadge } from "./parts";

type LibraryPatch = Partial<AgentLibrarySettings>;

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
  projects: Project[];
  selectedProject: string | null;
  onSelectProject: (path: string | null) => void;
  selectedPath: string | null;
  onSelect: (path: string | null) => void;
  editorTab: EditorTab;
  onEditorTab: (tab: EditorTab) => void;
  section: AgentSection;
  onSection: (section: AgentSection) => void;
};

const NEW_AGENT_PATH = "new";

export function AgentLibrary({
  projects,
  selectedProject,
  onSelectProject,
  selectedPath,
  onSelect,
  editorTab,
  onEditorTab,
  section,
  onSection,
}: Props) {
  const queryClient = useQueryClient();
  const bootstrap = useBootstrap();
  const setSettings = useSetAppSettings();
  const settings = bootstrap.data?.settings;
  const library = settings?.agents;
  const agentsWidth = useUiStore((s) => s.agentsWidth);
  const setAgentsWidth = useUiStore((s) => s.setAgentsWidth);
  const { narrow } = useViewport();
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const sidebarWidth = dragWidth ?? agentsWidth;
  const agentsCollapsed = useUiStore((s) => s.agentsCollapsed);

  const projectPath = selectedProject;
  const scope: AgentScope = selectedProject ? "project" : "global";

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
  const agentBundle = useAgentBundle();

  const [search, setSearch] = useState("");
  const [menu, setMenu] = useState<{ agent: AgentFile; x: number; y: number } | null>(null);
  const [folderMenu, setFolderMenu] = useState<{ folder: string; x: number; y: number } | null>(
    null,
  );
  const [dialog, setDialog] = useState<"folder" | "move" | "delete" | null>(null);
  const [folderName, setFolderName] = useState("");
  const [moveTarget, setMoveTarget] = useState("");
  const [bundleReport, setBundleReport] = useState<BundleReport | null>(null);
  const [bundleFailure, setBundleFailure] = useState<BundleFailure | null>(null);
  const [actionTarget, setActionTarget] = useState<AgentFile | null>(null);
  const [draft, setDraft] = useState<{
    base: { agent: AgentFile; overlay: Overlay };
    agent: AgentFile;
    overlay: Overlay;
  } | null>(null);

  const queued = useRef<AppSettings | null>(null);

  const projectName = projectPath ? basename(projectPath) : null;
  const all = useMemo(() => agents.data ?? [], [agents.data]);
  const groupUncategorized = library?.groupUncategorized ?? true;

  const stored = selectedPath ? (all.find((a) => a.path === selectedPath) ?? null) : null;
  const live =
    draft && (draft.agent.path === selectedPath || selectedPath === NEW_AGENT_PATH) ? draft : null;
  const working = live?.agent ?? stored ?? null;
  const workingOverlay = live?.overlay ?? (stored ? overlayFor(library, stored.path) : null);
  const baseline = live?.base ?? (stored ? { agent: stored, overlay: overlayFor(library, stored.path) } : null);

  const effectiveSection: AgentSection = working ? (isSystem(working) ? "system" : "agents") : section;
  const tab: AgentTab = effectiveSection === "system" ? "system" : "yours";

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
  const closedFolders = useMemo(() => new Set(library?.closedFolders ?? []), [library]);
  const keepEmpty = useMemo(
    () =>
      search.trim() || tab !== "yours" ? new Set<string>() : new Set(library?.folders ?? []),
    [search, tab, library],
  );
  const rows = useMemo(
    () => agentRows({ agents: visible, folders, closedFolders, folderOf, keepEmpty }),
    [visible, folders, closedFolders, folderOf, keepEmpty],
  );
  const agentsCount = useMemo(
    () => visibleAgents(all, { tab: "yours", scope, search: "" }).length,
    [all, scope],
  );

  const folderCount = (folder: string) => all.filter((a) => folderOf(a) === folder).length;

  const flash = (message: string) => useToast.getState().show("Agents", message);
  const fail = (title: string, e: unknown) => {
    const remedy = errorRemedy(e);
    const message = errorMessage(e);
    useToast
      .getState()
      .show(title, remedy ? `${message} — ${remedy}` : message, { tone: "bad" });
  };
  const refetch = () => queryClient.invalidateQueries({ queryKey: ["agent-files"] });

  const writeLibrary = (
    patch: LibraryPatch | ((current: AgentLibrarySettings) => LibraryPatch),
  ) => {
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
      onSettled: () => {
        if (queued.current === next) queued.current = null;
      },
    });
  };

  const save = useMutation({
    mutationFn: async (next: { agent: AgentFile; overlay: Overlay; isDefault: boolean }) => {
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
      for (const agent of batch) await unwrap(commands.deleteAgentFile(agent.path));
      return batch;
    },
    onSuccess: (batch) => {
      const overlays = { ...(library?.overlays ?? {}) };
      for (const a of batch) delete overlays[a.path];
      writeLibrary({ overlays });
      setDialog(null);
      setActionTarget(null);
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
      void refetch();
      flash(`Duplicated ${count} ${count === 1 ? "agent" : "agents"}`);
    },
    onError: (e) => fail("Duplicate failed", e),
  });

  const moveToFolder = useMutation({
    mutationFn: async ({ batch, folder }: { batch: AgentFile[]; folder: string }) => {
      const overlays = { ...(library?.overlays ?? {}) };
      try {
        for (const agent of batch) {
          if (isSystem(agent)) {
            overlays[agent.path] = { ...overlayFor(library, agent.path), folder };
          } else {
            await unwrap(commands.saveAgentFile({ ...agent, category: folder.trim() || null }));
            const legacy = library?.overlays?.[agent.path];
            if (legacy?.folder) overlays[agent.path] = { ...legacy, folder: "" };
          }
        }
      } catch (e) {
        throw new MoveFailed(errorMessage(e), overlays);
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
    onSettled: () => {
      setActionTarget(null);
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
      flash(paths.length === 1 ? `Exported to ${paths[0]}` : `Exported ${paths.length} agents`);
    },
    onError: (e) => fail("Export failed", e),
  });

  const updateBundle = useMutation({
    mutationFn: (_vars: { from: string | null; to: string | null }) =>
      unwrap(commands.updateAgentBundle()),
    onSuccess: (result, { from, to }) => {
      invalidateAgentBundle();
      setBundleReport({ ...result, from, to });
    },
    onError: (e, { from }) => {
      invalidateAgentBundle();
      setBundleFailure({ message: errorMessage(e), stillOn: from });
    },
  });

  const bundleChip = bundleChipState(agentBundle.data, {
    loading: agentBundle.isLoading,
    error: agentBundle.isError,
  });

  const startUpdate = () => {
    updateBundle.mutate(
      bundleChip.kind === "updateAvailable"
        ? { from: bundleChip.from, to: bundleChip.to }
        : { from: bundleChip.kind === "upToDate" ? bundleChip.label : null, to: null },
    );
  };

  const openDraft = (agent: AgentFile, overlay: Overlay, path: string) => {
    setDraft({ base: { agent, overlay }, agent, overlay });
    onSelect(path);
    onEditorTab("setup");
  };

  const createAgent = (folder?: string) => {
    setDefaultIntent(null);
    onSection("agents");
    const blank = newAgent(scope === "project" && projectName ? "project" : "user");
    openDraft(
      folder ? { ...blank, category: folder } : blank,
      { ...DEFAULT_OVERLAY },
      NEW_AGENT_PATH,
    );
  };

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

  const [defaultIntent, setDefaultIntent] = useState<boolean | null>(null);
  const isDefault = defaultIntent ?? (working ? library?.defaultAgent === working.path : false);

  const onBack = () => {
    setDraft(null);
    setDefaultIntent(null);
    onSelect(null);
  };

  const workingFolder = stored ? folderOf(stored) : "";
  const breadcrumb = {
    folder: workingFolder,
    onAgents: () => {
      onBack();
      onSection("agents");
    },
    onFolder: () => {
      onBack();
      if (workingFolder) {
        writeLibrary((current) => {
          const next = new Set(current.closedFolders ?? []);
          next.delete(workingFolder);
          return { closedFolders: [...next] };
        });
      }
    },
  };

  if (agents.isLoading || bootstrap.isLoading) return <PageSpinner />;

  return (
    <div className="flex h-full min-h-0">
      <AgentSidebar
        width={sidebarWidth}
        collapsed={agentsCollapsed}
        section={effectiveSection}
        onSection={onSection}
        selectedAgent={working}
        onBack={onBack}
        editorTab={editorTab}
        onEditorTab={onEditorTab}
        rows={rows}
        agentsCount={agentsCount}
        library={library}
        projects={projects}
        selectedProject={selectedProject}
        onSelectProject={onSelectProject}
        search={search}
        onSearch={setSearch}
        onToggleFolder={(folder) =>
          writeLibrary((current) => {
            const next = new Set(current.closedFolders ?? []);
            if (!next.delete(folder)) next.add(folder);
            return { closedFolders: [...next] };
          })
        }
        onOpen={(agent) => {
          setDefaultIntent(null);
          onSelect(agent.path);
        }}
        onMenu={(agent, at) => setMenu({ agent, ...at })}
        onFolderMenu={(folder, at) => setFolderMenu({ folder, ...at })}
        dotColorOf={(agent) => dotColor(agent, overlayFor(library, agent.path), folderOf)}
        loading={agents.isFetching}
        onCreate={() => createAgent()}
        onNewFolder={() => {
          onSection("agents");
          setFolderName("");
          setDialog("folder");
        }}
        groupUncategorized={groupUncategorized}
        onGroupUncategorized={(on) => writeLibrary({ groupUncategorized: on })}
        bundle={{
          state: bundleChip,
          updating: updateBundle.isPending,
          checking: agentBundle.isFetching,
          onRefresh: () => void agentBundle.refetch(),
          onUpdate: startUpdate,
        }}
      />

      {!narrow && !agentsCollapsed && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize agents sidebar"
          className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
          onPointerDown={(e) =>
            startEdgeDrag(e, {
              startWidth: agentsWidth,
              min: SIDE_PANEL_MIN,
              max: SIDE_PANEL_MAX,
              direction: 1,
              onResize: setDragWidth,
              onEnd: () =>
                setDragWidth((live) => {
                  if (live !== null) setAgentsWidth(live);
                  return null;
                }),
            })
          }
        />
      )}

      <section className="min-w-0 flex-1 overflow-y-auto">
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
            onBack={onBack}
            breadcrumb={breadcrumb}
            onSave={() => save.mutate({ agent: working, overlay: workingOverlay, isDefault })}
            onStatus={(status) => patchOverlay({ status })}
            onReset={() => {
              if (!isManaged(working) && working.modified) remove.mutate([working]);
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
          <AgentsCta section={effectiveSection} onCreate={() => createAgent()} />
        )}
      </section>

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
          {!isManaged(menu.agent) && (
            <ContextMenuItem
              onClick={() => {
                setActionTarget(menu.agent);
                setMoveTarget("");
                setMenu(null);
                setDialog("move");
              }}
            >
              Move to folder…
            </ContextMenuItem>
          )}
          <ContextMenuItem
            onClick={() => {
              setMenu(null);
              exportAgents.mutate([menu.agent]);
            }}
          >
            Export .agent.json
          </ContextMenuItem>
          {!isManaged(menu.agent) && (
            <ContextMenuItem
              danger
              onClick={() => {
                setActionTarget(menu.agent);
                setMenu(null);
                setDialog("delete");
              }}
            >
              {menu.agent.modified ? "Restore shipped version" : "Delete agent"}
            </ContextMenuItem>
          )}
        </ContextMenu>
      )}

      {folderMenu && (
        <ContextMenu
          position={{ x: folderMenu.x, y: folderMenu.y }}
          onClose={() => setFolderMenu(null)}
        >
          <ContextMenuItem
            onClick={() => {
              const { folder } = folderMenu;
              setFolderMenu(null);
              createAgent(folder);
            }}
          >
            New agent in “{folderMenu.folder}”
          </ContextMenuItem>
          {(library?.folders ?? []).includes(folderMenu.folder) && (
            <ContextMenuItem
              danger
              disabled={folderCount(folderMenu.folder) > 0}
              hint={
                folderCount(folderMenu.folder) > 0 ? "Move its agents out first." : undefined
              }
              onClick={() => {
                const { folder } = folderMenu;
                setFolderMenu(null);
                writeLibrary((current) => ({
                  folders: (current.folders ?? []).filter((f) => f !== folder),
                  closedFolders: (current.closedFolders ?? []).filter((f) => f !== folder),
                }));
                flash(`Folder “${folder}” deleted`);
              }}
            >
              Delete folder
            </ContextMenuItem>
          )}
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
              disabled={!actionTarget}
              onClick={() => actionTarget && moveToFolder.mutate({ batch: [actionTarget], folder: moveTarget })}
            >
              Move
            </Button>
          </div>
        </div>
      </Dialog>

      <Dialog open={dialog === "delete"} onClose={() => setDialog(null)} title="Delete agent">
        <div className="flex flex-col gap-3.5">
          <p className="text-sm leading-relaxed text-ink-dim">This deletes the file from disk:</p>
          {actionTarget && (
            <ul className="flex max-h-40 flex-col gap-1 overflow-y-auto">
              <li className="truncate font-mono text-xs text-ink">{actionTarget.path}</li>
            </ul>
          )}
          <p className="text-xs text-ink-dim">
            Sessions already running are unaffected; a modified system agent goes back to the
            version its plugin ships.
          </p>
          <div className="flex justify-end gap-2">
            <Button onClick={() => setDialog(null)}>Cancel</Button>
            <Button
              variant="danger"
              busy={remove.isPending}
              disabled={!actionTarget}
              onClick={() => actionTarget && remove.mutate([actionTarget])}
            >
              Delete
            </Button>
          </div>
        </div>
      </Dialog>

      <AgentUpdateReport
        report={bundleReport}
        open={bundleReport !== null}
        onClose={() => setBundleReport(null)}
      />
      <AgentUpdateFailed
        failure={bundleFailure}
        open={bundleFailure !== null}
        onClose={() => setBundleFailure(null)}
        onRetry={() => {
          setBundleFailure(null);
          startUpdate();
        }}
      />
    </div>
  );
}

function AgentsCta({ section, onCreate }: { section: AgentSection; onCreate: () => void }) {
  if (section === "skills") {
    return (
      <div className="mx-auto w-full max-w-5xl px-7 py-8">
        <div className="mb-4 flex items-center gap-2">
          <h1 className="text-lg font-semibold tracking-tight text-ink">Skills</h1>
          <SoonBadge />
        </div>
        <Card>
          <h3 className="text-sm font-semibold text-ink">Not shipped yet</h3>
          <p className="mt-1 text-xs text-ink-dim">
            Skills will live beside agents on this page and drill in the same way. Reserving the
            section now is what keeps the nav from being rebuilt when they land.
          </p>
        </Card>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 p-12 text-center">
      <span
        aria-hidden="true"
        className="mb-2 grid h-14 w-14 place-items-center rounded-full border border-edge bg-panel text-ink-dim"
      >
        <LayersIcon size={24} strokeWidth={1.5} full />
      </span>
      <p className="text-base font-medium text-ink">No agent selected</p>
      <div className="max-w-md text-sm text-ink-dim">
        Pick one from the sidebar to edit its prompt, model, tools and permissions — or create a
        new agent so a workstream can start the same way every time.
      </div>
      <div className="mt-2.5">
        <Button variant="primary" onClick={onCreate}>
          Create Agent
        </Button>
      </div>
    </div>
  );
}
