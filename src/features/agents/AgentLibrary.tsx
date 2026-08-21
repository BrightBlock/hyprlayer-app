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
import { useAgentBundle, useBootstrap, useSetAppSettings } from "@/lib/queries";
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
  /** Every mapped project, for the sidebar's scope picker. */
  projects: Project[];
  /** Path of the project the list is scoped to (`?project=`), or null for the
   *  global library. Doubles as the `projectPath` every agent query and write
   *  is keyed by, so picking a project here is what makes "create an agent for
   *  this project" land in that project's `.claude/agents`. */
  selectedProject: string | null;
  onSelectProject: (path: string | null) => void;
  /** Path of the open agent (`?agent=`), or null for the list. */
  selectedPath: string | null;
  onSelect: (path: string | null) => void;
  editorTab: EditorTab;
  onEditorTab: (tab: EditorTab) => void;
  /** The sidebar's root nav choice (`?section=`). */
  section: AgentSection;
  onSection: (section: AgentSection) => void;
};

/** The unsaved-new-agent sentinel, so the editor can open before a file exists. */
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
  // The live drag, held locally and committed to the store once on pointer-up.
  // `agentsWidth` is persisted (unlike the other panel widths), so letting the
  // drag write straight through would hit localStorage on every pointermove —
  // the cost `uiStore`'s `partialize` comment exists to avoid.
  const [dragWidth, setDragWidth] = useState<number | null>(null);
  const sidebarWidth = dragWidth ?? agentsWidth;
  const agentsCollapsed = useUiStore((s) => s.agentsCollapsed);

  // One selection drives both halves of what used to be a Global|Project
  // segmented control plus a route-supplied project: which directory every
  // query below reads, and which agents `visibleAgents` admits.
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
  // The agent an in-flight Move/Delete dialog acts on — always exactly one,
  // the context menu's own row. No multi-select survives decision 1; this
  // replaces the old `Set<string>` staging a checkbox column used to fill.
  const [actionTarget, setActionTarget] = useState<AgentFile | null>(null);
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

  const stored = selectedPath ? (all.find((a) => a.path === selectedPath) ?? null) : null;
  const live =
    draft && (draft.agent.path === selectedPath || selectedPath === NEW_AGENT_PATH) ? draft : null;
  const working = live?.agent ?? stored ?? null;
  const workingOverlay = live?.overlay ?? (stored ? overlayFor(library, stored.path) : null);
  const baseline = live?.base ?? (stored ? { agent: stored, overlay: overlayFor(library, stored.path) } : null);

  // Section and agent are not independent: an open agent pins the section to
  // its own kind, overriding a stale or mismatched `?section=` (a deep link,
  // or one left over from before the agent was opened). This only affects
  // what renders — the URL itself is left alone, same as `tab`'s own default.
  const effectiveSection: AgentSection = working ? (isSystem(working) ? "system" : "agents") : section;
  const tab: AgentTab = effectiveSection === "system" ? "system" : "yours";

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
  // A hand-made folder renders even while it is empty — that is the whole
  // window in which the user has to be able to see it. Not while a search is
  // running, though: a filter that matched nothing in a folder should hide the
  // folder, not leave a row claiming 0 next to the results.
  // `tab` too: `library.folders` is shared by both trees, and an empty custom
  // folder on the System list would be a row for something that list has no
  // "New folder" button to have made.
  const keepEmpty = useMemo(
    () =>
      search.trim() || tab !== "yours" ? new Set<string>() : new Set(library?.folders ?? []),
    [search, tab, library],
  );
  const rows = useMemo(
    () => agentRows({ agents: visible, folders, closedFolders, folderOf, keepEmpty }),
    [visible, folders, closedFolders, folderOf, keepEmpty],
  );
  // The Agents root row's count. Scope-aware, because it has to be: the
  // query itself is keyed by `projectPath`, so scoping to a project already
  // narrows `all` to that project's directory, and counting the raw result
  // put "19" above a tree reading "No agents yet" (`visibleAgents` drops
  // anything whose `source` isn't "project"). Search is deliberately still
  // ignored — the row names the size of the library, not how much of it the
  // filter box is currently showing.
  const agentsCount = useMemo(
    () => visibleAgents(all, { tab: "yours", scope, search: "" }).length,
    [all, scope],
  );

  /** How many agents are filed in `folder`, counted against the whole library
   *  the query returned — deliberately not off `rows`, which the scope picker,
   *  the section and the filter box have each already narrowed. `all` holds
   *  both the global and the project agents whenever a project is in scope
   *  (that is what `visibleAgents`' `source` check exists to filter), so a
   *  folder that looks empty from inside one project can still be holding
   *  global agents, and the delete guard has to see them. */
  const folderCount = (folder: string) => all.filter((a) => folderOf(a) === folder).length;

  const flash = (message: string) => useToast.getState().show("Agents", message);
  // `unwrap` throws the AppError envelope, a plain object, so a bare String(e)
  // renders "[object Object]" (same trap as ScriptRunView.tsx:125). The remedy
  // is the actionable half, so fold it into the toast's one message line.
  const fail = (title: string, e: unknown) => {
    const remedy = errorRemedy(e);
    const message = errorMessage(e);
    useToast
      .getState()
      .show(title, remedy ? `${message} — ${remedy}` : message, { tone: "bad" });
  };
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
    // Whether or not the batch finished, some files may already be on disk,
    // the list has to be re-read rather than left showing the old folders.
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

  // `updateAgentBundle` takes no arguments of its own, but the report and
  // failure dialogs both need the SHA pair as it stood the moment the update
  // was triggered, `agentBundle.data` has already moved on to the new value
  // by the time a success/error callback runs (the query invalidates right
  // alongside). Carrying it through as the mutation's variables, the same
  // shape `moveToFolder` reads its `folder` from, sidesteps a stale closure.
  const updateBundle = useMutation({
    mutationFn: (_vars: { fromSha: string | null; toSha: string | null }) =>
      unwrap(commands.updateAgentBundle()),
    onSuccess: (result, { fromSha, toSha }) => {
      void queryClient.invalidateQueries({ queryKey: ["agent-files"] });
      void queryClient.invalidateQueries({ queryKey: ["agent-bundle"] });
      // `skills` too: the reinstall rewrites `~/.claude/skills` alongside the
      // agents, and the Capabilities tab reads that query.
      void queryClient.invalidateQueries({ queryKey: ["agent-skills"] });
      setBundleReport({ ...result, fromSha, toSha });
    },
    onError: (e, { fromSha }) => {
      // Invalidated on this path too: a 300s timeout can kill the CLI
      // mid-sync, so "the update failed" does not imply "nothing on disk
      // moved" and the cached list may already be wrong.
      void queryClient.invalidateQueries({ queryKey: ["agent-files"] });
      void queryClient.invalidateQueries({ queryKey: ["agent-bundle"] });
      void queryClient.invalidateQueries({ queryKey: ["agent-skills"] });
      setBundleFailure({ message: errorMessage(e), stillOnSha: fromSha });
    },
  });

  const bundleChip = bundleChipState(agentBundle.data, {
    loading: agentBundle.isLoading,
    error: agentBundle.isError,
  });

  // One definition behind both the chip's trigger and the failure dialog's
  // "Try again". The SHA pair is bookkeeping for the report, NOT a
  // precondition: the chip only ever offers the trigger from
  // `updateAvailable`, but a retry runs after `onError` invalidated the
  // freshness query, which may since have failed for the same reason the
  // update did. Refusing to fire without a known SHA pair would make "Try
  // again" a silent no-op exactly when the user needs it.
  const startUpdate = () => {
    updateBundle.mutate(
      bundleChip.kind === "updateAvailable"
        ? { fromSha: bundleChip.fromSha, toSha: bundleChip.toSha }
        : { fromSha: agentBundle.data?.installedSha ?? null, toSha: null },
    );
  };

  const openDraft = (agent: AgentFile, overlay: Overlay, path: string) => {
    setDraft({ base: { agent, overlay }, agent, overlay });
    onSelect(path);
    onEditorTab("setup");
  };

  /** New-agent draft, from either the sidebar's + or the empty pane's CTA.
   *
   *  Pins `?section=` to "agents" on the way in. The header's + renders on
   *  every root, so it can be pressed from Skills or System agents, and
   *  `effectiveSection` only overrides the section while the draft is open —
   *  going back would restore the old one and drop the user on the Skills
   *  placeholder, or on the system list, with the agent they just made
   *  nowhere in sight.
   *
   *  `DEFAULT_OVERLAY` unmodified, so the agent saves published. It used to
   *  override `status: "draft"`, and draft is not cosmetic: `agent_deny_rules`
   *  (`crates/hyprlayer-core/src/agents.rs:615`) emits an `Agent(<name>)` deny
   *  rule for every draft at session launch, so the agent a user had just
   *  written, named and saved was the one thing they could not spawn — and the
   *  only way out was a Publish item buried in the editor's kebab that patched
   *  the overlay without saving it, so it took a second Save too. Draft is
   *  still reachable, as a deliberate "park this one" from the same menu. */
  const createAgent = (folder?: string) => {
    setDefaultIntent(null);
    onSection("agents");
    const blank = newAgent(scope === "project" && projectName ? "project" : "user");
    // `category`, not `overlay.folder`: for an owned agent the category is
    // what `folderOf` reads first, and it lives in the file's own frontmatter,
    // so the filing survives the agent being exported or moved between
    // machines. The overlay's copy is the legacy path the editor retires.
    openDraft(
      folder ? { ...blank, category: folder } : blank,
      { ...DEFAULT_OVERLAY },
      NEW_AGENT_PATH,
    );
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

  // Shared by the sidebar's drilled-in back chevron and the editor's own
  // Cancel button — one back affordance, reached from two places (the
  // editor no longer draws its own chevron, see AgentEditor's header).
  const onBack = () => {
    setDraft(null);
    setDefaultIntent(null);
    onSelect(null);
  };

  // The open agent's folder, for the editor's breadcrumb — the drilled-in
  // sidebar shows the root rows and this agent's own sections, never the
  // folder it sits in, so the breadcrumb is the only way back to it. Reuses
  // the same bound `folderOf` the tree groups by, not the raw 3-arg function.
  //
  // Resolved from `stored`, not the draft: `folderOf` derives an uncategorized
  // agent's folder from its name (`phaseFolder`), so typing a rename in the
  // editor would walk the crumb from Discover to Any stage while the file is
  // still filed under Discover — and clicking it would expand the wrong
  // folder. A brand-new agent has no `stored`, so the segment drops out, which
  // is right: it isn't filed anywhere yet.
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
        resizing={dragWidth !== null}
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
          // Same reason `createAgent` pins the section: folders are an Agents
          // concept, and the new one is only visible on that root.
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
              // Commit once, on release: see `dragWidth` above.
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
              setActionTarget(menu.agent);
              setMenu(null);
              setDialog("delete");
            }}
          >
            {menu.agent.modified ? "Restore shipped version" : "Delete agent"}
          </ContextMenuItem>
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
          {/* Only a hand-made folder can be deleted: a derived one (a phase, a
              plugin's name, a `category` a user wrote in frontmatter) is not a
              thing settings holds, so there is nothing to remove — it exists
              for as long as an agent resolves into it. And only while it is
              empty: unfiling the members would mean rewriting each of their
              files, which is what "Move to folder…" is for. */}
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
                  // Drop the collapse state too, or a folder made again under
                  // the same name would arrive closed for no visible reason.
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

/**
 * The content pane with nothing open: a call to action on the Agents/System
 * agents roots, the "not shipped yet" placeholder on Skills. The sidebar is
 * the list — this column never repeats it, see the plan's decision to drop
 * the table entirely.
 */
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
