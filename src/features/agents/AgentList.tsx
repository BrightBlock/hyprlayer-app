import { Badge, TextInput } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { ListCardRow, ListHeader, NavGroupLabel } from "@/components/workshop";
import type { AgentFile, AgentLibrarySettings } from "@/lib/bindings";
import type { ModelOption } from "@/lib/domain/models";
import {
  agentMeta,
  folderColor,
  isManaged,
  isSystem,
  overlayFor,
  shortDescription,
  stagesForPhase,
  statusOf,
  type AgentRow,
  type AgentSection,
} from "./agents";

type At = { x: number; y: number };

export type AgentListProps = {
  section: AgentSection;
  folder: string | null;
  rows: AgentRow[];
  flat: AgentFile[];
  systemInFolder: AgentFile[];
  total: number;
  search: string;
  onSearch: (value: string) => void;
  selectedPath: string | null;
  library: AgentLibrarySettings | undefined;
  catalog: readonly ModelOption[];
  loading: boolean;
  onOpen: (agent: AgentFile) => void;
  onMenu: (agent: AgentFile, at: At) => void;
  onFolderMenu: (folder: string, at: At) => void;
  onToggleFolder: (folder: string) => void;
  onNewFolder: () => void;
  dotColorOf: (agent: AgentFile) => string;
};

export function AgentList(props: AgentListProps) {
  const {
    section,
    folder,
    rows,
    flat,
    systemInFolder,
    total,
    search,
    onSearch,
    loading,
    onToggleFolder,
    onFolderMenu,
    onNewFolder,
  } = props;
  const stages = folder ? stagesForPhase(folder) : [];
  const where = folder ?? "All folders";
  const empty = folder ? flat.length === 0 : rows.length === 0;

  return (
    <>
      <ListHeader
        actions={
          section === "agents" && (
            <button
              type="button"
              onClick={onNewFolder}
              title="New folder"
              aria-label="New folder"
              className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
            >
              <Icon name="folder" size="sm" />
            </button>
          )
        }
      >
        {section === "system"
          ? `${where} · ${total} system agents`
          : `${where} · ${total} of your agents${stages.length ? ` · stage: ${stages.join(", ")}` : ""}`}
      </ListHeader>

      <div className="shrink-0 border-b border-edge p-3">
        <TextInput
          value={search}
          onChange={(e) => onSearch(e.target.value)}
          placeholder="Filter…"
          aria-label="Filter agents"
        />
      </div>

      <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-3 pt-2">
        {folder
          ? flat.map((agent) => <AgentCard key={agent.path} agent={agent} {...props} />)
          : rows.map((row) =>
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
                <div key={row.agent.path} style={{ paddingLeft: row.indent ? FOLDER_CHILD_INDENT : 0 }}>
                  <AgentCard agent={row.agent} {...props} />
                </div>
              ),
            )}
        {empty && !loading && (
          <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-ui leading-prose text-ink-dim">
            {search.trim()
              ? `No agents match “${search.trim()}”`
              : section === "system"
                ? "No system agents"
                : "No agents yet"}
          </div>
        )}
        {section === "agents" && folder && systemInFolder.length > 0 && (
          <div className="pt-4">
            <NavGroupLabel>System agents in {folder}</NavGroupLabel>
            <div className="space-y-0.5">
              {systemInFolder.map((agent) => (
                <AgentCard key={agent.path} agent={agent} {...props} />
              ))}
            </div>
          </div>
        )}
      </div>
    </>
  );
}

// Centers a child's 6px dot under the folder icon: row padding 10 + half the 14px icon, less the card's border, padding and half-dot.
const FOLDER_CHILD_INDENT = 10 + 7 - (1 + 10 + 3);

function AgentCard({
  agent,
  section,
  selectedPath,
  library,
  catalog,
  onOpen,
  onMenu,
  dotColorOf,
}: AgentListProps & { agent: AgentFile }) {
  const system = isSystem(agent);
  const meta = agentMeta(agent, catalog);
  return (
    <ListCardRow
      leading={
        <span
          aria-hidden="true"
          className="h-1.5 w-1.5 rounded-full"
          style={{ background: dotColorOf(agent) }}
        />
      }
      title={agent.name || "untitled"}
      badges={
        <>
          {agent.path === library?.defaultAgent && (
            <span title="Default agent for new sessions">
              <Badge tone="accent">Default</Badge>
            </span>
          )}
          {statusOf(agent, overlayFor(library, agent.path)).tone === "warn" && (
            <Badge tone="warn">Draft</Badge>
          )}
          {system && section !== "system" && <Badge tone="dim">System</Badge>}
          {isManaged(agent) && <Badge tone="dim">Read-only</Badge>}
        </>
      }
      description={shortDescription(agent.description)}
      meta={system && agent.plugin ? `${agent.plugin} · ${meta}` : meta}
      selected={agent.path === selectedPath}
      onClick={() => onOpen(agent)}
      onContextMenu={(at) => onMenu(agent, at)}
    />
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
  onMenu: (at: At) => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY });
      }}
      aria-expanded={open}
      className="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-body font-semibold text-ink-dim hover:bg-edge/40"
    >
      <span className="shrink-0" style={{ color: folderColor(folder) }}>
        <Icon name={open ? "folderOpen" : "folder"} size="sm" />
      </span>
      <span className="min-w-0 flex-1 truncate text-ink">{folder}</span>
      <span className="shrink-0 text-ui font-normal opacity-70">{count}</span>
    </button>
  );
}
