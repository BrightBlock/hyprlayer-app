
import { useRef, useState, type ReactNode } from "react";

import { Badge, IconButton, Popover, RefreshButton, Spinner, TextInput, Toggle } from "@/components/ui";
import { Icon, type IconName } from "@/components/Icon";
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

const HEADER_ICON_BTN =
  "flex h-6 w-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink";

const FOLDER_LABEL_X = 49;

const AGENT_ROW_BASE = FOLDER_LABEL_X - 15;

export type AgentSidebarProps = {
  width: number;
  collapsed: boolean;
  section: AgentSection;
  onSection: (section: AgentSection) => void;
  selectedAgent: AgentFile | null;
  onBack: () => void;
  editorTab: EditorTab;
  onEditorTab: (tab: EditorTab) => void;

  rows: AgentRow[];
  agentsCount: number;
  library: AgentLibrarySettings | undefined;
  projects: Project[];
  selectedProject: string | null;
  onSelectProject: (path: string | null) => void;
  search: string;
  onSearch: (value: string) => void;
  onToggleFolder: (folder: string) => void;
  onOpen: (agent: AgentFile) => void;
  onMenu: (agent: AgentFile, at: { x: number; y: number }) => void;
  onFolderMenu: (folder: string, at: { x: number; y: number }) => void;
  dotColorOf: (agent: AgentFile) => string;
  loading: boolean;

  onCreate: () => void;
  onNewFolder: () => void;
  groupUncategorized: boolean;
  onGroupUncategorized: (on: boolean) => void;

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

  const rootVariant = (s: AgentSection): "filled" | "trail" | "plain" =>
    s !== section ? "plain" : drilled ? "trail" : "filled";
  const rootNav = (
    <div className="space-y-0.5 border-b border-edge pb-3">
      <RootRow
        icon={<Icon name="layers" />}
        label="Agents"
        trailing={<span className="shrink-0 text-ui text-ink-dim opacity-70">{agentsCount}</span>}
        variant={rootVariant("agents")}
        onClick={() => onSection("agents")}
      />
      <RootRow
        icon={<Icon name="plugin" />}
        label="System agents"
        variant={rootVariant("system")}
        onClick={() => onSection("system")}
      />
      <RootRow
        icon={<Icon name="skill" />}
        label="Skills"
        trailing={<SoonBadge />}
        variant={rootVariant("skills")}
        onClick={() => onSection("skills")}
      />
    </div>
  );

  const shellWidth = narrow ? SIDE_PANEL_MIN : width;

  return (
    <aside
      className="relative shrink-0 overflow-hidden border-r border-edge bg-panel"
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
            <Icon name="chevronLeft" size="sm" />
          </button>
          <p className="min-w-0 flex-1 truncate text-title text-ink-dim">
            {selectedAgent.name || "untitled"}
          </p>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              const at = e.currentTarget.getBoundingClientRect();
              onMenu(selectedAgent, { x: at.right - 150, y: at.bottom + 6 });
            }}
            title="More actions"
            aria-label="More actions"
            className={HEADER_ICON_BTN}
          >
            <Icon name="more" size="sm" />
          </button>
        </div>
      ) : (
        <div className="flex h-9 shrink-0 items-center justify-between border-b border-edge pl-4 pr-3">
          <p className="min-w-0 truncate text-title text-ink-dim">
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
              <Icon name="folder" size="sm" />
            </button>
            <button
              type="button"
              onClick={onCreate}
              title="New agent"
              aria-label="New agent"
              className={HEADER_ICON_BTN}
            >
              <Icon name="add" size="sm" />
            </button>
            {section === "agents" && (
              <>
                <button
                  ref={overflowRef}
                  type="button"
                  onClick={() => setOverflowOpen((v) => !v)}
                  title="More options"
                  aria-label="More options"
                  className={HEADER_ICON_BTN}
                >
                  <Icon name="more" size="sm" />
                </button>
                <Popover
                  open={overflowOpen}
                  onClose={() => setOverflowOpen(false)}
                  anchorRef={overflowRef}
                  align="end"
                  className="min-w-56"
                >
                  <div className="flex items-center justify-between gap-3 rounded px-2.5 py-1.5 text-body text-ink">
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


      <nav className="flex-1 space-y-5 overflow-y-auto px-2 pb-3 pt-2" aria-label="Agents">
        {rootNav}

        {drilled ? (
          <div>
            <p className="px-1.5 pb-1 text-label text-ink-dim">
              {selectedAgent.name || "untitled"}
            </p>
            <div className="space-y-0.5">
              {tabs.map((t) => (
                <ConfigSectionRow
                  key={t.key}
                  label={t.label}
                  icon={<Icon name={EDITOR_TAB_ICON[t.key]} />}
                  selected={editorTab === t.key}
                  onClick={() => onEditorTab(t.key)}
                />
              ))}
            </div>
          </div>
        ) : (
          <>
            {section === "skills" && (
              <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-ui leading-prose text-ink-dim">
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
                  <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-ui leading-prose text-ink-dim">
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

const EDITOR_TAB_ICON: Record<EditorTab, IconName> = {
  setup: "tool",
  prompt: "docText",
  capabilities: "prompt",
  permissions: "permissions",
  collaborators: "users",
};

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
      className={`flex w-full items-center gap-2 rounded-md border px-2.5 py-2 text-left text-body transition-colors ${
        selected ? "border-accent/30 bg-accent/10 text-ink" : "border-transparent text-ink hover:bg-edge/40"
      }`}
    >
      <span className={`shrink-0 ${selected ? "text-accent" : "text-ink-dim"}`}>{icon}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
    </button>
  );
}

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
      className={`flex w-full items-center gap-2 rounded-md border px-2.5 py-2 text-left text-body transition-colors ${
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
        <Icon name="chevronRight" size="xs" className="text-ink-dim opacity-65" />
      ) : (
        trailing
      )}
    </button>
  );
}

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
        className="flex w-full items-center gap-2 rounded-md border border-edge bg-panel-2 px-2.5 py-1.5 text-body hover:bg-edge/40"
      >
        {current ? (
          <RepoAvatar source={current.originUrl} name={current.mappedName} size={14} className="rounded" />
        ) : (
          <Icon name="layers" size="sm" className="text-ink-dim" />
        )}
        <span className="min-w-0 flex-1 truncate text-left font-medium text-ink">
          {current?.mappedName ?? "Global"}
        </span>
        <Icon name="chevronDown" size="xs" className="text-ink-dim" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        matchAnchorWidth
        className="max-h-72 overflow-y-auto"
      >
        <ScopeItem active={!value} onClick={() => choose(null)}>
          <Icon name="layers" size="sm" />
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
      className={`flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-body ${
        active ? "bg-accent/10 text-ink" : "text-ink-dim hover:bg-panel-2 hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}

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
      className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-body font-semibold text-ink-dim hover:bg-edge/40"
    >
      <span
        aria-hidden="true"
        className={`inline-flex w-3 shrink-0 transition-transform ${open ? "rotate-90" : ""}`}
      >
        <Icon name="chevronRight" size="xs" />
      </span>
      <span className="shrink-0" style={{ color: folderColor(folder) }}>
        <Icon name="folder" size="sm" />
      </span>
      <span className="min-w-0 flex-1 truncate text-ink">{folder}</span>
      <span className="shrink-0 text-ui font-normal opacity-70">{count}</span>
    </button>
  );
}

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
      className="flex w-full items-center gap-2 rounded-md border border-transparent py-2 pr-2.5 text-left text-body text-ink transition-colors hover:bg-edge/40"
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
      <div className="flex shrink-0 items-center gap-2 text-ui text-ink-dim">
        <span>Updating agents…</span>
        <IconButton disabled title="Updating agents…" aria-label="Updating agents">
          <Spinner size="sm" />
        </IconButton>
      </div>
    );
  }

  if (state.kind === "updateAvailable") {
    const to = state.to;
    return (
      <div className="flex shrink-0 items-center gap-2">
        <Badge tone="warn">New agents available</Badge>
        <span className="hidden text-meta text-ink-dim sm:inline">
          {state.from ? `${state.from} → ${to}` : to}
        </span>
        <IconButton
          onClick={onUpdate}
          title={`Update agents to ${to}`}
          aria-label={`Update agents to ${to}`}
        >
          <Icon name="download" className="text-accent" />
        </IconButton>
      </div>
    );
  }

  if (state.kind === "unknown") {
    return (
      <RefreshButton
        onClick={onRefresh}
        busy={checking}
        label="Check for agent updates"
        title={state.detail}
        className="shrink-0 text-ui"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-ink-dim/50" />
        <span className="whitespace-nowrap">Bundle unknown</span>
      </RefreshButton>
    );
  }

  const detail = [
    state.label,
    state.lastCheckedMs !== null ? `checked ${formatRelativeTime(state.lastCheckedMs)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  if (state.frozen) {
    return (
      <RefreshButton
        onClick={onRefresh}
        busy={checking}
        label="Check for agent updates"
        title={`Update the hyprlayer CLI for current skills${detail ? ` · ${detail}` : ""}`}
        className="shrink-0 text-ui"
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
      className="shrink-0 text-ui"
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
      <span className="whitespace-nowrap">Up to date</span>
    </RefreshButton>
  );
}
