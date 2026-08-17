/**
 * The Agents list: the Your/System switch, scope, search, multi-select with
 * bulk actions, and the agents grouped into folders.
 */

import type { ReactNode } from "react";

import { Badge, Button, EmptyState, TextInput } from "@/components/ui";
import { FolderIcon, PlusIcon, SearchIcon } from "@/components/icons";
import { formatRelativeTime } from "@/lib/format";
import type { AgentFile, AgentLibrarySettings } from "@/lib/bindings";
import {
  dotColor,
  modelLabel,
  overlayFor,
  shortDescription,
  statusOf,
  type AgentRow,
  type AgentScope,
  type AgentTab,
} from "./agents";
import { Segmented } from "./parts";

type Props = {
  rows: AgentRow[];
  /** Everything the current filter admits — what "select all" covers. */
  agents: AgentFile[];
  library: AgentLibrarySettings | undefined;
  defaultAgent: string | null;
  tab: AgentTab;
  onTab: (tab: AgentTab) => void;
  scope: AgentScope;
  onScope: (scope: AgentScope) => void;
  /** Basename of the open project, naming the project scope. */
  projectName: string | null;
  search: string;
  onSearch: (value: string) => void;
  selected: ReadonlySet<string>;
  onToggleSelect: (path: string) => void;
  onToggleAll: () => void;
  onClearSelection: () => void;
  onToggleFolder: (folder: string) => void;
  onOpen: (agent: AgentFile) => void;
  onMenu: (agent: AgentFile, at: { x: number; y: number }) => void;
  onCreate: () => void;
  onNewFolder: () => void;
  loading: boolean;
  bulk: {
    duplicate: () => void;
    move: () => void;
    export: () => void;
    remove: () => void;
  };
};

const GRID = "grid grid-cols-[34px_minmax(0,1fr)_150px_130px_130px_36px] items-center";

export function AgentList({
  rows,
  agents,
  library,
  defaultAgent,
  tab,
  onTab,
  scope,
  onScope,
  projectName,
  search,
  onSearch,
  selected,
  onToggleSelect,
  onToggleAll,
  onClearSelection,
  onToggleFolder,
  onOpen,
  onMenu,
  onCreate,
  onNewFolder,
  loading,
  bulk,
}: Props) {
  const allOn = agents.length > 0 && agents.every((a) => selected.has(a.path));
  const selCount = selected.size;

  return (
    <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
      <div className="flex items-start justify-between gap-5 px-7 pb-3.5 pt-5">
        <h1 className="min-w-0 truncate text-xl font-semibold tracking-tight text-ink">Agents</h1>
        <div className="flex shrink-0 items-center gap-2">
          <Button onClick={onNewFolder}>
            <FolderIcon size={14} />
            New folder
          </Button>
          <Button variant="primary" onClick={onCreate}>
            <PlusIcon size={14} />
            Create Agent
          </Button>
        </div>
      </div>

      <div className="flex items-center gap-3 px-7 pb-3.5">
        <Segmented
          aria-label="Agent library"
          value={tab}
          onChange={onTab}
          options={[
            { key: "yours", label: "Your Agents" },
            { key: "system", label: "System Agents" },
          ]}
        />
        {tab === "yours" && (
          <div className="inline-flex items-center gap-2 text-xs text-ink-dim">
            <span className="opacity-80">Scope</span>
            <Segmented
              size="sm"
              aria-label="Agent scope"
              value={scope}
              onChange={onScope}
              options={[
                { key: "global", label: "Global" },
                {
                  key: "project",
                  label: projectName ?? "Project",
                  mono: Boolean(projectName),
                },
              ]}
              disabled={!projectName}
            />
          </div>
        )}

        <div className="flex-1" />
        <div className="relative w-58">
          <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-dim">
            <SearchIcon size={13} />
          </span>
          <TextInput
            value={search}
            onChange={(e) => onSearch(e.target.value)}
            placeholder="Search agents…"
            aria-label="Search agents"
            className="pl-7"
          />
        </div>
      </div>

      {selCount > 0 && (
        <div className="mx-7 mb-3 flex items-center gap-2.5 rounded-lg border border-accent/35 bg-accent/10 px-3 py-2">
          <span className="text-xs font-semibold text-accent">
            {selCount} {selCount === 1 ? "agent" : "agents"} selected
          </span>
          <span className="h-4 w-px bg-edge" />
          <BulkAction onClick={bulk.duplicate}>Duplicate</BulkAction>
          <BulkAction onClick={bulk.move}>Move to folder…</BulkAction>
          <BulkAction onClick={bulk.export}>Export</BulkAction>
          <BulkAction onClick={bulk.remove} danger>
            Delete
          </BulkAction>
          <div className="flex-1" />
          <button
            type="button"
            onClick={onClearSelection}
            className="rounded px-1.5 py-0.5 text-xs text-ink-dim hover:text-ink"
          >
            Clear
          </button>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-7 pb-7">
        <div
          className={`${GRID} border-b border-edge px-2 pb-2 text-[11px] font-semibold uppercase tracking-wider text-ink-dim`}
        >
          <Check checked={allOn} onChange={onToggleAll} label="Select all agents" />
          <div>Name</div>
          <div>Model</div>
          <div>Status</div>
          <div>Updated</div>
          <div />
        </div>

        {rows.map((row) =>
          row.kind === "group" ? (
            <button
              key={`group:${row.folder}`}
              type="button"
              onClick={() => onToggleFolder(row.folder)}
              className="flex w-full items-center gap-2 border-b border-edge/60 px-2 py-2.5 text-xs font-semibold text-ink-dim"
            >
              <span
                aria-hidden="true"
                className={`inline-block w-3 transition-transform ${row.open ? "rotate-90" : ""}`}
              >
                ›
              </span>
              <FolderIcon size={13} />
              <span className="text-ink">{row.folder}</span>
              <span className="opacity-70">
                {row.count} {row.count === 1 ? "agent" : "agents"}
              </span>
            </button>
          ) : (
            <AgentRowView
              key={row.agent.path}
              agent={row.agent}
              indent={row.indent}
              library={library}
              isDefault={row.agent.path === defaultAgent}
              selected={selected.has(row.agent.path)}
              onToggleSelect={() => onToggleSelect(row.agent.path)}
              onOpen={() => onOpen(row.agent)}
              onMenu={(at) => onMenu(row.agent, at)}
            />
          ),
        )}

        {rows.length === 0 && !loading && (
          <div className="py-14">
            <EmptyState
              title={
                search.trim()
                  ? `No agents match “${search.trim()}”`
                  : tab === "system"
                    ? "No system agents"
                    : "No agents yet"
              }
            >
              {search.trim()
                ? "Try a shorter search, or clear it to see everything in this scope."
                : tab === "system"
                  ? "System agents come from installed Claude Code plugins. Install one and it shows up here."
                  : "An agent bundles a prompt, a model, the tools it may use and the permissions it runs under — so a workstream can start the same way every time."}
            </EmptyState>
            {!search.trim() && tab === "yours" && (
              <div className="mt-3.5 flex justify-center">
                <Button variant="primary" onClick={onCreate}>
                  Create Agent
                </Button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function BulkAction({
  onClick,
  danger,
  children,
}: {
  onClick: () => void;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded px-1.5 py-0.5 text-xs ${
        danger ? "text-bad hover:bg-bad/10" : "text-ink hover:bg-panel-2"
      }`}
    >
      {children}
    </button>
  );
}

/** The row/header checkbox. A real input so it keeps keyboard and AT behaviour. */
function Check({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: () => void;
  label: string;
}) {
  return (
    <label className="flex w-fit cursor-pointer items-center" onClick={(e) => e.stopPropagation()}>
      <input type="checkbox" checked={checked} onChange={onChange} className="sr-only" />
      <span className="sr-only">{label}</span>
      <span
        aria-hidden="true"
        className={`grid h-[15px] w-[15px] place-items-center rounded border border-edge text-[10px] text-accent-ink ${
          checked ? "bg-accent" : ""
        }`}
      >
        {checked ? "✓" : ""}
      </span>
    </label>
  );
}

function AgentRowView({
  agent,
  indent,
  library,
  isDefault,
  selected,
  onToggleSelect,
  onOpen,
  onMenu,
}: {
  agent: AgentFile;
  indent: number;
  library: AgentLibrarySettings | undefined;
  isDefault: boolean;
  selected: boolean;
  onToggleSelect: () => void;
  onOpen: () => void;
  onMenu: (at: { x: number; y: number }) => void;
}) {
  const overlay = overlayFor(library, agent.path);
  const status = statusOf(agent, overlay);
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY });
      }}
      className={`${GRID} h-13 cursor-pointer rounded-lg border-b border-edge/60 px-2 hover:bg-panel-2 ${
        selected ? "bg-accent/10" : ""
      }`}
    >
      <Check checked={selected} onChange={onToggleSelect} label={`Select ${agent.name}`} />
      <div className="flex min-w-0 items-center gap-2" style={{ paddingLeft: indent }}>
        <span
          aria-hidden="true"
          className="h-[7px] w-[7px] shrink-0 rounded-full"
          style={{ background: dotColor(agent, overlay) }}
        />
        <span className="shrink-0 whitespace-nowrap text-sm font-medium text-ink">
          {agent.name}
        </span>
        {isDefault && (
          <span className="shrink-0 whitespace-nowrap">
            <Badge tone="accent">Default</Badge>
          </span>
        )}
        {agent.plugin && (
          <span className="shrink-0 whitespace-nowrap">
            <Badge tone="dim">{agent.plugin}</Badge>
          </span>
        )}
        {agent.modified && (
          <span className="shrink-0 whitespace-nowrap">
            <Badge tone="warn">Modified</Badge>
          </span>
        )}
        <span className="min-w-0 truncate text-xs text-ink-dim" title={agent.description}>
          {shortDescription(agent.description)}
        </span>
      </div>
      <div className="truncate font-mono text-xs text-ink-dim">{modelLabel(agent.model)}</div>
      <div>
        <Badge tone={status.tone}>{status.label}</Badge>
      </div>
      <div className="text-xs text-ink-dim">
        {agent.updatedAt ? formatRelativeTime(agent.updatedAt) : "—"}
      </div>
      <button
        type="button"
        aria-label={`Actions for ${agent.name}`}
        onClick={(e) => {
          e.stopPropagation();
          onMenu({ x: e.clientX - 150, y: e.clientY + 6 });
        }}
        className="grid h-6.5 w-6.5 place-items-center rounded-md tracking-widest text-ink-dim hover:bg-panel hover:text-ink"
      >
        ⋮
      </button>
    </div>
  );
}
