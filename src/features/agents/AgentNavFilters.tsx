import { Toggle } from "@/components/ui";
import { NavFilterRow, NavGroupLabel } from "@/components/workshop";
import { folderColor } from "./agents";

export function AgentNavFilters({
  folders,
  counts,
  total,
  folder,
  onFolder,
  onFolderMenu,
  groupUncategorized,
  onGroupUncategorized,
  showGroupToggle,
}: {
  folders: string[];
  counts: ReadonlyMap<string, number>;
  total: number;
  folder: string | null;
  onFolder: (folder: string | null) => void;
  onFolderMenu: (folder: string, at: { x: number; y: number }) => void;
  groupUncategorized: boolean;
  onGroupUncategorized: (on: boolean) => void;
  showGroupToggle: boolean;
}) {
  return (
    <div>
      <NavGroupLabel>Folders</NavGroupLabel>
      <div className="space-y-0.5">
        <NavFilterRow label="All" count={total} selected={folder === null} onClick={() => onFolder(null)} />
        {folders.map((f) => (
          <NavFilterRow
            key={f}
            label={f}
            dot={folderColor(f)}
            count={counts.get(f) ?? 0}
            selected={folder === f}
            onClick={() => onFolder(f)}
            onContextMenu={(at) => onFolderMenu(f, at)}
          />
        ))}
      </div>
      {showGroupToggle && (
        <div className="mt-2 flex items-center justify-between gap-3 px-2.5 py-1.5 text-body text-ink">
          <span>Group by stage</span>
          <Toggle
            checked={groupUncategorized}
            onChange={onGroupUncategorized}
            aria-label="Group uncategorized agents by lifecycle stage"
          />
        </div>
      )}
    </div>
  );
}
