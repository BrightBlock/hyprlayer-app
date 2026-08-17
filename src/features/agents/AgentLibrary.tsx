/**
 * The Agents view's state owner: lists the `.claude/agents/*.md` definitions,
 * keeps the editor's working copy, and writes each save back to its own file.
 *
 * The files ARE the agents, so every definition change is a filesystem write
 * followed by a refetch; the launch settings claude has no frontmatter key for
 * ride in `AppSettings.agents`, keyed by the same path.
 */

import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  Button,
  ContextMenu,
  ContextMenuItem,
  Dialog,
  DropdownSelect,
  PageSpinner,
  TextInput,
} from "@/components/ui";
import { commands, type AgentFile } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { basename } from "@/lib/path";
import { useBootstrap, useSetAppSettings } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import {
  agentRows,
  DEFAULT_OVERLAY,
  duplicateAgent,
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

  const [tab, setTab] = useState<AgentTab>("yours");
  const [scope, setScope] = useState<AgentScope>("global");
  const [search, setSearch] = useState("");
  const [openFolders, setOpenFolders] = useState<ReadonlySet<string>>(new Set());
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

  const projectName = projectPath ? basename(projectPath) : null;
  const all = useMemo(() => agents.data ?? [], [agents.data]);
  const folders = useMemo(() => library?.folders ?? [], [library]);
  const visible = useMemo(
    () => visibleAgents(all, { tab, scope, search }),
    [all, tab, scope, search],
  );
  const rows = useMemo(
    () => agentRows(visible, library, folders, openFolders),
    [visible, library, folders, openFolders],
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

  /** Fold a change into the persisted agent-library settings. */
  const writeLibrary = (patch: Partial<NonNullable<typeof library>>) => {
    if (!settings) return;
    setSettings.mutate({
      ...settings,
      agents: {
        folders,
        overlays: library?.overlays ?? {},
        defaultAgent: library?.defaultAgent ?? null,
        ...patch,
      },
    });
  };

  const save = useMutation({
    mutationFn: async (next: { agent: AgentFile; overlay: Overlay; isDefault: boolean }) => {
      // A new agent — or the first edit of a plugin's — has no file of its own
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
      // own file — the shipped version comes back.
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

  const patch = (p: Partial<AgentFile>) =>
    working && workingOverlay && baseline
      ? setDraft({ base: baseline, agent: { ...working, ...p }, overlay: workingOverlay })
      : undefined;
  const patchOverlay = (p: Partial<Overlay>) =>
    working && workingOverlay && baseline
      ? setDraft({ base: baseline, agent: working, overlay: { ...workingOverlay, ...p } })
      : undefined;

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
            setOpenFolders((prev) => {
              const next = new Set(prev);
              if (!next.delete(folder)) next.add(folder);
              return next;
            })
          }
          loading={agents.isFetching}
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
              setMoveTarget(folders[0] ?? "");
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
              setMoveTarget(folders[0] ?? "");
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
                writeLibrary({ folders: [...folders, name] });
                setOpenFolders((prev) => new Set(prev).add(name));
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
          <DropdownSelect
            value={moveTarget}
            options={[
              { value: "", label: "No folder" },
              ...folders.map((f) => ({ value: f, label: f })),
            ]}
            onChange={setMoveTarget}
            aria-label="Target folder"
          />
          <div className="flex justify-end gap-2">
            <Button onClick={() => setDialog(null)}>Cancel</Button>
            <Button
              variant="primary"
              onClick={() => {
                const overlays = { ...(library?.overlays ?? {}) };
                for (const a of chosen) {
                  overlays[a.path] = { ...overlayFor(library, a.path), folder: moveTarget };
                }
                writeLibrary({ overlays });
                setSelected(new Set());
                setDialog(null);
                flash(`Moved to ${moveTarget || "no folder"}`);
              }}
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
