/**
 * The Agents sidebar: root nav (Agents / System agents / Skills), the filter
 * and scope controls, and the folder/agent tree — the Prompts anatomy
 * (`PromptLibrary.tsx:610-747`) applied to the agent library. Presentational:
 * props in, callbacks out, no queries, no store reads except the width.
 */

import { useRef, useState, type ReactNode } from "react";

import { Badge, IconButton, Popover, RefreshButton, Spinner, TextInput, Toggle } from "@/components/ui";
import {
  ChevronDownIcon,
  ChevronLeftIcon,
  DocTextIcon,
  DownloadIcon,
  FolderIcon,
  LayersIcon,
  PeopleIcon,
  PlusIcon,
  PuzzleIcon,
  ShieldIcon,
  WrenchIcon,
  ZapIcon,
} from "@/components/icons";
import { RepoAvatar } from "@/components/RepoAvatar";
import { useSignedIn } from "@/lib/account";
import { formatRelativeTime } from "@/lib/format";
import { SIDE_PANEL_MIN } from "@/lib/uiStore";
import { useViewport } from "@/lib/useViewport";
import type { AgentFile, AgentLibrarySettings, Project } from "@/lib/bindings";
import {
  editorTabs,
  folderColor,
  overlayFor,
  statusOf,
  type AgentRow,
  type AgentSection,
  type BundleChipState,
  type EditorTab,
} from "./agents";
import { SoonBadge } from "./parts";

/** Header band and inline icon buttons, matching Prompts' own
 *  (`PromptLibrary.tsx:618-649`) — the app's inline icon-button tier
 *  (ADR-0006 "Icon buttons"), not `IconButton`'s h-8 chrome tier. */
const HEADER_ICON_BTN =
  "flex h-6 w-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink";

/** Where a `FolderRow`'s label starts, in px from the row's left edge: its own
 *  `px-2.5` inset (10) + the disclosure chevron (`w-2.5`, 10) + `gap-2` (8) +
 *  the folder icon (13) + `gap-2` (8). */
const FOLDER_LABEL_X = 49;

/** Left inset for a depth-0 agent row, chosen so its *label* lands on
 *  `FOLDER_LABEL_X`: the row's own leading chrome is a 6px dot, a `gap-2` (8)
 *  and its `border` (1), so it starts 15px earlier than its label.
 *
 *  That makes a loose agent's name align with a folder's name — they are
 *  siblings — and pushes a foldered agent (indent 14) one step further in, so
 *  its dot lands in the folder label's own column. Same relationship
 *  `ChangesPanel`'s file rows keep to their directory rows (`:88`, `:201`):
 *  the child's glyph sits at the parent's label, the child's label past it.
 *  Before this the base was a flat 10, which put a foldered agent's dot 24px
 *  left of its folder's label and read as outdented from its own parent. */
const AGENT_ROW_BASE = FOLDER_LABEL_X - 15;

export type AgentSidebarProps = {
  /** Rendered width (px). Owned by `AgentLibrary`, which resolves the live
   *  drag against the persisted store value — see `uiStore`'s `partialize`
   *  note on why the drag itself must not reach the store. */
  width: number;
  /** Collapsed to zero width by the header's sidebar toggle (`agentsCollapsed`). */
  collapsed: boolean;
  /** Drops the width transition while the resize handle is being dragged, so
   *  the panel tracks the cursor instead of easing behind it. */
  resizing: boolean;
  section: AgentSection;
  onSection: (section: AgentSection) => void;
  /** The open agent, or null for the list. Drills the sidebar in — see
   *  "Sidebar gains a drilled-in state" in the plan's Phase 2. */
  selectedAgent: AgentFile | null;
  /** Clears the selection and any unsaved draft — the drilled-in header
   *  band's only back affordance now that the editor no longer has its own. */
  onBack: () => void;
  editorTab: EditorTab;
  onEditorTab: (tab: EditorTab) => void;

  /** Everything `agentRows()` produced for the current section — see
   *  `AgentLibrary`, which derives `AgentTab` from `section` before calling it. */
  rows: AgentRow[];
  /** Owned (non-system) agents in the current scope — the Agents root row's
   *  count. Scope-aware (the caller's query is keyed by project path), but
   *  unaffected by the search box. */
  agentsCount: number;
  /** Resolves an agent's Draft status for the tree row's badge. */
  library: AgentLibrarySettings | undefined;
  /** Every mapped project, for the scope picker. */
  projects: Project[];
  /** Path of the project in scope, or null for the global library. */
  selectedProject: string | null;
  onSelectProject: (path: string | null) => void;
  search: string;
  onSearch: (value: string) => void;
  onToggleFolder: (folder: string) => void;
  onOpen: (agent: AgentFile) => void;
  onMenu: (agent: AgentFile, at: { x: number; y: number }) => void;
  /** Right-click on a folder header: "New agent in <folder>", and the delete
   *  for a hand-made one. Same idiom as `onMenu` — the row reports the point,
   *  `AgentLibrary` owns the menu. */
  onFolderMenu: (folder: string, at: { x: number; y: number }) => void;
  /** The dot beside each agent's name, already resolved: the sidebar renders
   *  the colour, it does not decide it (same contract as `AgentList` had). */
  dotColorOf: (agent: AgentFile) => string;
  loading: boolean;

  onCreate: () => void;
  onNewFolder: () => void;
  /** State of the "Group uncategorized" switch, folded into the header's
   *  overflow menu. Only meaningful for owned agents' folder derivation, so
   *  the menu that carries it only renders on the Agents root. */
  groupUncategorized: boolean;
  onGroupUncategorized: (on: boolean) => void;

  /** The bundle-freshness chip, now living in the footer band. */
  bundle: {
    state: BundleChipState;
    updating: boolean;
    checking: boolean;
    onRefresh: () => void;
    onUpdate: () => void;
  };
};

export function AgentSidebar(props: AgentSidebarProps) {
  const {
    width,
    collapsed,
    resizing,
    section,
    onSection,
    selectedAgent,
    onBack,
    editorTab,
    onEditorTab,
    rows,
    agentsCount,
    library,
    projects,
    selectedProject,
    onSelectProject,
    search,
    onSearch,
    onToggleFolder,
    onOpen,
    onMenu,
    onFolderMenu,
    dotColorOf,
    loading,
    onCreate,
    onNewFolder,
    groupUncategorized,
    onGroupUncategorized,
    bundle,
  } = props;

  const { narrow } = useViewport();
  const [overflowOpen, setOverflowOpen] = useState(false);
  const overflowRef = useRef<HTMLButtonElement>(null);
  const tabs = editorTabs(useSignedIn());

  const drilled = selectedAgent !== null;

  // Root nav is present in both states (plan Phase 2, "Sidebar gains a
  // drilled-in state"): Agents/Skills never leave the screen. Drilled in, the
  // current root is marked but not filled — the fill belongs to the
  // Configuration row below — so it renders a trailing "›" instead of its
  // usual count/badge and the fill goes to whichever config row is active.
  const rootVariant = (s: AgentSection): "filled" | "trail" | "plain" =>
    s !== section ? "plain" : drilled ? "trail" : "filled";
  const rootNav = (
    <div className="space-y-0.5 border-b border-edge pb-3">
      <RootRow
        icon={<LayersIcon size={14} />}
        label="Agents"
        trailing={<span className="shrink-0 text-xs text-ink-dim opacity-70">{agentsCount}</span>}
        variant={rootVariant("agents")}
        onClick={() => onSection("agents")}
      />
      <RootRow
        icon={<PuzzleIcon size={14} />}
        label="System agents"
        variant={rootVariant("system")}
        onClick={() => onSection("system")}
      />
      <RootRow
        icon={<ZapIcon size={14} />}
        label="Skills"
        trailing={<SoonBadge />}
        variant={rootVariant("skills")}
        onClick={() => onSection("skills")}
      />
    </div>
  );

  const shellWidth = narrow ? SIDE_PANEL_MIN : width;

  // Width-animates between 0 and `shellWidth` so a collapse/expand glides
  // instead of snapping, matching the Library and project panels.
  // `overflow-hidden` clips the fixed-width inner shell so its content never
  // reflows mid-animation, and the transition is dropped while the resize
  // handle is dragged. Not gated on reduced motion: Windows maps that to the
  // often-off "Animation effects" toggle, which would snap the panel.
  return (
    <aside
      className={`relative shrink-0 overflow-hidden border-r border-edge bg-panel ${
        resizing ? "" : "transition-[width] duration-200 ease-out"
      }`}
      style={{ width: collapsed ? 0 : shellWidth }}
      inert={collapsed}
    >
      <div className="absolute inset-y-0 left-0 flex flex-col" style={{ width: shellWidth }}>
      {drilled ? (
        <div className="flex h-9 shrink-0 items-center gap-1.5 border-b border-edge pl-2.5 pr-3">
          <button
            type="button"
            onClick={onBack}
            title="All agents"
            aria-label="All agents"
            className={HEADER_ICON_BTN}
          >
            <ChevronLeftIcon size={14} />
          </button>
          <p className="min-w-0 flex-1 truncate text-xs font-medium uppercase tracking-wider text-ink-dim">
            {selectedAgent.name || "untitled"}
          </p>
          <button
            type="button"
            onClick={(e) => {
              // The menu closes on any window click, so the opener has to keep
              // its own click from reaching that listener. Anchor off the
              // button, not the pointer, matching AgentEditor's own kebab.
              e.stopPropagation();
              const at = e.currentTarget.getBoundingClientRect();
              onMenu(selectedAgent, { x: at.right - 150, y: at.bottom + 6 });
            }}
            title="More actions"
            aria-label="More actions"
            className={`${HEADER_ICON_BTN} tracking-widest`}
          >
            ⋮
          </button>
        </div>
      ) : (
        <div className="flex h-9 shrink-0 items-center justify-between border-b border-edge pl-4 pr-3">
          <p className="min-w-0 truncate text-xs font-medium uppercase tracking-wider text-ink-dim">
            Agents
          </p>
          <div className="flex items-center gap-0.5">
            <button
              type="button"
              onClick={onNewFolder}
              title="New folder"
              aria-label="New folder"
              className={HEADER_ICON_BTN}
            >
              <FolderIcon size={14} />
            </button>
            <button
              type="button"
              onClick={onCreate}
              title="New agent"
              aria-label="New agent"
              className={HEADER_ICON_BTN}
            >
              <PlusIcon size={14} />
            </button>
            {section === "agents" && (
              <>
                <button
                  ref={overflowRef}
                  type="button"
                  onClick={() => setOverflowOpen((v) => !v)}
                  title="More options"
                  aria-label="More options"
                  className={`${HEADER_ICON_BTN} tracking-widest`}
                >
                  ⋮
                </button>
                <Popover
                  open={overflowOpen}
                  onClose={() => setOverflowOpen(false)}
                  anchorRef={overflowRef}
                  align="end"
                  className="min-w-56"
                >
                  <div className="flex items-center justify-between gap-3 rounded px-2.5 py-1.5 text-sm text-ink">
                    <span>Group uncategorized</span>
                    <Toggle
                      checked={groupUncategorized}
                      onChange={onGroupUncategorized}
                      aria-label="Group uncategorized agents by lifecycle stage"
                    />
                  </div>
                </Popover>
              </>
            )}
          </div>
        </div>
      )}

      {/* Scope above filter, both in one bordered block — `LibrarySidebar`'s
          own controls band (`:86-104`), down to its `space-y-2 p-3`. Scope
          reads first because it decides *which* library the filter then
          narrows; the rule closes the band off from the nav below. */}
      {!drilled && (
        <div className="shrink-0 space-y-2 border-b border-edge p-3">
          {section === "agents" && projects.length > 0 && (
            <ScopePicker projects={projects} value={selectedProject} onChange={onSelectProject} />
          )}
          <TextInput
            value={search}
            onChange={(e) => onSearch(e.target.value)}
            placeholder="Filter…"
            aria-label="Filter agents"
          />
        </div>
      )}


      {/* `space-y-5` is the break between the root nav and whatever follows it
          (the tree, or the Skills placeholder): 20px, wider than the 2px
          between sibling rows inside either, so the two read as separate
          sections rather than one long list interrupted by a rule. */}
      <nav className="flex-1 space-y-5 overflow-y-auto px-2 pb-3 pt-2" aria-label="Agents">
        {rootNav}

        {drilled ? (
          <div>
            <p className="px-1.5 pb-1 text-2xs font-semibold uppercase tracking-wider text-ink-dim">
              {selectedAgent.name || "untitled"}
            </p>
            <div className="space-y-0.5">
              {tabs.map((t) => (
                <ConfigSectionRow
                  key={t.key}
                  label={t.label}
                  icon={editorTabIcon(t.key)}
                  selected={editorTab === t.key}
                  onClick={() => onEditorTab(t.key)}
                />
              ))}
            </div>
          </div>
        ) : (
          <>
            {section === "skills" && (
              <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-xs leading-relaxed text-ink-dim">
                No skills yet. They will be filed under the same folders agents use.
              </div>
            )}

            {section !== "skills" && (
              <div className="space-y-0.5">
                {rows.map((row) =>
                  row.kind === "group" ? (
                    <FolderRow
                      key={`group:${row.folder}`}
                      folder={row.folder}
                      count={row.count}
                      open={row.open}
                      onClick={() => onToggleFolder(row.folder)}
                      onMenu={(at) => onFolderMenu(row.folder, at)}
                    />
                  ) : (
                    <TreeAgentRow
                      key={row.agent.path}
                      agent={row.agent}
                      indent={row.indent}
                      isDraft={statusOf(row.agent, overlayFor(library, row.agent.path)).tone === "warn"}
                      isDefault={row.agent.path === library?.defaultAgent}
                      dot={dotColorOf(row.agent)}
                      onOpen={() => onOpen(row.agent)}
                      onMenu={(at) => onMenu(row.agent, at)}
                    />
                  ),
                )}
                {rows.length === 0 && !loading && (
                  <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-xs leading-relaxed text-ink-dim">
                    {search.trim()
                      ? `No agents match “${search.trim()}”`
                      : section === "system"
                        ? "No system agents"
                        : "No agents yet"}
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </nav>

      <div className="shrink-0 border-t border-edge px-4 py-3">
        <BundleChip {...bundle} />
      </div>
      </div>
    </aside>
  );
}

/** Config-row icon, keyed by `EditorTab` — one place mapping the section to
 *  its glyph, matching the `harnessIcon`/`ModeIcon` convention in icons.tsx. */
function editorTabIcon(key: EditorTab) {
  switch (key) {
    case "setup":
      return <WrenchIcon size={14} />;
    case "prompt":
      return <DocTextIcon size={14} />;
    case "capabilities":
      return <ZapIcon size={14} />;
    case "permissions":
      return <ShieldIcon size={14} />;
    case "collaborators":
      return <PeopleIcon size={14} />;
  }
}

/** A drilled-in Configuration row (Setup/Prompt/…): the same selectable-row
 *  treatment as `RootRow`'s filled variant, but content rows read at full
 *  `text-ink` unselected, not the root nav's dimmer `text-ink-dim`. */
function ConfigSectionRow({
  label,
  icon,
  selected,
  onClick,
}: {
  label: string;
  icon: ReactNode;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={selected}
      className={`flex w-full items-center gap-2 rounded-md border px-2.5 py-2 text-left text-sm transition-colors ${
        selected ? "border-accent/30 bg-accent/10 text-ink" : "border-transparent text-ink hover:bg-edge/40"
      }`}
    >
      <span className={`shrink-0 ${selected ? "text-accent" : "text-ink-dim"}`}>{icon}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
    </button>
  );
}

/** `"filled"` is the ordinary selected state (border + tint). `"trail"` is
 *  Phase 2's drilled-in marker: named and accent-iconed like filled, but with
 *  no border/tint of its own — that fill belongs to the active Configuration
 *  row — and a trailing "›" in place of the row's usual count/badge. */
function RootRow({
  icon,
  label,
  trailing,
  variant,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  trailing?: ReactNode;
  variant: "filled" | "trail" | "plain";
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={variant !== "plain"}
      className={`flex w-full items-center gap-2 rounded-md border px-2.5 py-2 text-left text-sm transition-colors ${
        variant === "filled"
          ? "border-accent/30 bg-accent/10 text-ink"
          : variant === "trail"
            ? "border-transparent font-medium text-ink hover:bg-edge/40"
            : "border-transparent text-ink-dim hover:bg-edge/40"
      }`}
    >
      <span className={`shrink-0 ${variant === "plain" ? "text-ink-dim" : "text-accent"}`}>{icon}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {variant === "trail" ? (
        <span aria-hidden="true" className="shrink-0 text-ink-dim opacity-65">
          ›
        </span>
      ) : (
        trailing
      )}
    </button>
  );
}

/**
 * Which library the list shows: the global `~/.claude/agents`, or one
 * project's own `.claude/agents`. Picking a project here is also what makes a
 * new agent land in that project — `AgentLibrary` uses the same selection as
 * the `projectPath` every query and write is keyed by.
 *
 * A picker rather than the Global|Project segmented control this replaces:
 * that one could only offer whichever project happened to be current
 * elsewhere in the app, so scoping to any other one meant leaving Agents,
 * selecting that project, and coming back.
 *
 * Copies `LibrarySidebar`'s `ProjectSwitcher` anatomy (`:173-241`) rather than
 * importing it — ADR-0003 allows another feature's domain API, not its
 * components — and is built from shared primitives only (`Popover`,
 * `RepoAvatar`).
 */
function ScopePicker({
  projects,
  value,
  onChange,
}: {
  projects: Project[];
  value: string | null;
  onChange: (path: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const current = projects.find((p) => p.path === value) ?? null;

  const choose = (path: string | null) => {
    onChange(path);
    setOpen(false);
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Agent scope"
        className="flex w-full items-center gap-2 rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-sm hover:bg-edge/40"
      >
        {current ? (
          <RepoAvatar source={current.originUrl} name={current.mappedName} size={14} className="rounded" />
        ) : (
          <LayersIcon size={14} className="text-ink-dim" />
        )}
        <span className="min-w-0 flex-1 truncate text-left font-medium text-ink">
          {current?.mappedName ?? "Global"}
        </span>
        <ChevronDownIcon size={12} className="shrink-0 text-ink-dim" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        matchAnchorWidth
        className="max-h-72 overflow-y-auto"
      >
        <ScopeItem active={!value} onClick={() => choose(null)}>
          <LayersIcon size={14} />
          <span className="min-w-0 flex-1 truncate">Global</span>
        </ScopeItem>
        {projects.map((p) => (
          <ScopeItem key={p.path} active={p.path === value} onClick={() => choose(p.path)}>
            <RepoAvatar source={p.originUrl} name={p.mappedName} size={14} className="rounded" />
            <span className="min-w-0 flex-1 truncate">{p.mappedName}</span>
          </ScopeItem>
        ))}
      </Popover>
    </>
  );
}

function ScopeItem({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      aria-current={active}
      className={`flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-sm ${
        active ? "bg-accent/10 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

/** A folder header: chevron, phase-coloured folder icon, name, count. Never
 *  selectable (only its own agents are), so it carries no border/tint state,
 *  just the disclosure toggle and its own right-click menu. */
function FolderRow({
  folder,
  count,
  open,
  onClick,
  onMenu,
}: {
  folder: string;
  count: number;
  open: boolean;
  onClick: () => void;
  onMenu: (at: { x: number; y: number }) => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY });
      }}
      className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm font-semibold text-ink-dim hover:bg-edge/40"
    >
      <span
        aria-hidden="true"
        className={`inline-block w-2.5 shrink-0 transition-transform ${open ? "rotate-90" : ""}`}
      >
        ›
      </span>
      <span className="shrink-0" style={{ color: folderColor(folder) }}>
        <FolderIcon size={13} />
      </span>
      <span className="min-w-0 flex-1 truncate text-ink">{folder}</span>
      <span className="shrink-0 text-xs font-normal opacity-70">{count}</span>
    </button>
  );
}

/** An agent row: dot, name, then a Draft badge when unpublished and a Default
 *  badge on the library's default agent. Weight (not size) separates it from
 *  `FolderRow` — both are `text-sm`, see the plan's note on why the old
 *  table's `text-xs` folder row inverted the hierarchy.
 *
 *  The Default badge is the only place the setting is visible: the context
 *  menu's "Set as default" writes it and the editor toggles it, but neither
 *  reads it back into the list. The old table drew it here too
 *  (`AgentList.tsx:560-563` before the sidebar rewrite) and the rewrite
 *  dropped it, which left the library with no way to answer "which one is
 *  default?" short of opening each agent. */
function TreeAgentRow({
  agent,
  indent,
  isDraft,
  isDefault,
  dot,
  onOpen,
  onMenu,
}: {
  agent: AgentFile;
  indent: number;
  isDraft: boolean;
  isDefault: boolean;
  dot: string;
  onOpen: () => void;
  onMenu: (at: { x: number; y: number }) => void;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY });
      }}
      style={{ paddingLeft: AGENT_ROW_BASE + indent }}
      className="flex w-full items-center gap-2 rounded-md border border-transparent py-2 pr-2.5 text-left text-sm text-ink transition-colors hover:bg-edge/40"
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: dot }} />
      <span className="min-w-0 flex-1 truncate font-medium">{agent.name}</span>
      {isDraft && (
        <span className="shrink-0">
          <Badge tone="warn">Draft</Badge>
        </span>
      )}
      {isDefault && (
        <span className="shrink-0" title="Default agent for new sessions">
          <Badge tone="accent">Default</Badge>
        </span>
      )}
    </button>
  );
}

/**
 * The Agents view's bundle-freshness chip: up to date (with a manual
 * recheck), an update available (with the trigger), updating, or no verdict
 * to give. Icon-only for the trigger, not a labelled `Button` — the footer
 * band's only other content is descriptive text, and a primary button would
 * dominate it.
 */
function BundleChip({
  state,
  updating,
  checking,
  onRefresh,
  onUpdate,
}: {
  state: BundleChipState;
  updating: boolean;
  checking: boolean;
  onRefresh: () => void;
  onUpdate: () => void;
}) {
  if (updating) {
    return (
      <div className="flex shrink-0 items-center gap-2 text-xs text-ink-dim">
        {/* "Updating", not a phase name: the backup is a handful of local
            copies that finish instantly and the rest of the wait is the CLI's
            tarball download, so naming a phase here would be a guess that is
            wrong for almost the whole mutation. */}
        <span>Updating agents…</span>
        <IconButton disabled title="Updating agents…" aria-label="Updating agents">
          <Spinner size={13} />
        </IconButton>
      </div>
    );
  }

  if (state.kind === "updateAvailable") {
    const to = state.to;
    return (
      <div className="flex shrink-0 items-center gap-2">
        <Badge tone="warn">New agents available</Badge>
        <span className="hidden font-mono text-xs text-ink-dim sm:inline">
          {state.from ? `${state.from} → ${to}` : to}
        </span>
        {/* The accent tint rides on the ICON, not the button: `cx` in
            components/ui.tsx merges conflicting classes now, but the icon is
            still a separate element from the button so `currentColor` has
            nothing else to resolve against. */}
        <IconButton
          onClick={onUpdate}
          title={`Update agents to ${to}`}
          aria-label={`Update agents to ${to}`}
        >
          <DownloadIcon size={15} className="text-accent" />
        </IconButton>
      </div>
    );
  }

  // `busy` on both remaining branches: the recheck behind them spawns a
  // `hyprlayer ai status` process AND makes an 8s-timeout round trip to
  // GitHub, so without it the button sits inert long enough that a user
  // clicks it again and queues a second pair.
  if (state.kind === "unknown") {
    return (
      <RefreshButton
        onClick={onRefresh}
        busy={checking}
        label="Check for agent updates"
        title={state.detail}
        className="shrink-0 text-xs"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-ink-dim/50" />
        <span className="whitespace-nowrap">Bundle unknown</span>
      </RefreshButton>
    );
  }

  // The steady state, so it stays a bare "Up to date": which bundle is
  // installed and when it was last looked at are reference details, not
  // something to read on every visit. They ride on `title` instead, which
  // `RefreshButton` keeps distinct from the `aria-label` naming the action.
  const detail = [
    state.label,
    state.lastCheckedMs !== null ? `checked ${formatRelativeTime(state.lastCheckedMs)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  // Current bytes from a dead-end source, so neither the green dot nor the
  // update trigger: the fix is a new CLI, which this chip cannot install.
  // Settings names the version and offers the button.
  if (state.frozen) {
    return (
      <RefreshButton
        onClick={onRefresh}
        busy={checking}
        label="Check for agent updates"
        title={`Update the hyprlayer CLI for current skills${detail ? ` · ${detail}` : ""}`}
        className="shrink-0 text-xs"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" />
        <span className="whitespace-nowrap">CLI out of date</span>
      </RefreshButton>
    );
  }

  return (
    <RefreshButton
      onClick={onRefresh}
      busy={checking}
      label="Check for agent updates"
      title={detail ? `Agent bundle ${detail} · click to check for updates` : undefined}
      className="shrink-0 text-xs"
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
      <span className="whitespace-nowrap">Up to date</span>
    </RefreshButton>
  );
}
