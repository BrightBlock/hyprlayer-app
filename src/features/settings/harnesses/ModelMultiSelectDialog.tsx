import { useMemo, useState, type ReactNode } from "react";

import { Button, Dialog, TextInput } from "@/components/ui";
import { Icon } from "@/components/Icon";

export type ModelRow = {
  id: string;
  name: string;
  detail?: string;
};

export type ModelRowGroup = {
  key: string;
  label?: string;
  models: ModelRow[];
};

export function ModelMultiSelectDialog({
  open,
  onClose,
  onSave,
  groups,
  selected,
  title = "Add models",
  searchPlaceholder = "Filter models",
  emptyLabel = "No models available.",
  addCustom,
  bulkActions,
}: {
  open: boolean;
  onClose: () => void;
  onSave: (next: string[]) => void;
  groups: ModelRowGroup[];
  selected: string[];
  title?: string;
  searchPlaceholder?: string;
  emptyLabel?: string;
  addCustom?: { placeholder: string };
  bulkActions?: (apply: (ids: string[]) => void) => ReactNode;
}) {
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(selected));
  const [newId, setNewId] = useState("");
  const [added, setAdded] = useState<string[]>([]);

  const allGroups: ModelRowGroup[] = useMemo(
    () =>
      added.length > 0
        ? [
            ...groups,
            { key: "__added", label: "Added", models: added.map((id) => ({ id, name: id })) },
          ]
        : groups,
    [groups, added],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return allGroups.filter((g) => g.models.length > 0);
    return allGroups
      .map((g) => ({
        ...g,
        models: g.models.filter(
          (m) =>
            m.name.toLowerCase().includes(q) ||
            m.id.toLowerCase().includes(q) ||
            (g.label ?? "").toLowerCase().includes(q),
        ),
      }))
      .filter((g) => g.models.length > 0);
  }, [allGroups, query]);

  function addTypedId() {
    const id = newId.trim();
    if (!id) return;
    const known = allGroups.some((g) => g.models.some((m) => m.id === id));
    if (!known) setAdded((prev) => [...prev, id]);
    setChosen((prev) => new Set(prev).add(id));
    setNewId("");
  }

  function toggle(id: string, on: boolean) {
    setChosen((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  const close = onClose;

  return (
    <Dialog open={open} onClose={close} title={title} size="xl">
      <div className="relative mb-3">
        <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-dim">
          <Icon name="search" size="sm" />
        </span>
        <TextInput
          autoFocus
          className="pl-8"
          placeholder={searchPlaceholder}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      {addCustom && (
        <div className="mb-3 flex gap-2">
          <TextInput
            value={newId}
            placeholder={addCustom.placeholder}
            onChange={(e) => setNewId(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addTypedId();
              }
            }}
          />
          <Button onClick={addTypedId} disabled={!newId.trim()}>
            Add
          </Button>
        </div>
      )}

      <div className="max-h-[24rem] overflow-y-auto">
        {filtered.length === 0 && (
          <p className="px-1 py-6 text-center text-body text-ink-dim">
            {query ? `No models match “${query}”.` : emptyLabel}
          </p>
        )}
        {filtered.map((group) => (
          <div key={group.key} className="mb-3">
            {group.label && (
              <p className="px-1 py-1 text-label text-ink-dim">
                {group.label}
              </p>
            )}
            <ul>
              {group.models.map((m) => (
                <li key={m.id}>
                  <label className="flex cursor-pointer items-center gap-2.5 rounded px-1 py-1.5 hover:bg-edge/40">
                    <input
                      type="checkbox"
                      className="h-4 w-4 shrink-0 cursor-pointer accent-accent"
                      checked={chosen.has(m.id)}
                      onChange={(e) => toggle(m.id, e.target.checked)}
                    />
                    <span className="min-w-0">
                      <span className="text-body text-ink">{m.name}</span>
                      <span className="block text-meta text-ink-dim">
                        {m.detail ?? m.id}
                      </span>
                    </span>
                  </label>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>

      <div className="mt-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-ui text-ink-dim">{chosen.size} selected</span>
          {bulkActions?.((ids) => setChosen(new Set(ids)))}
        </div>
        <div className="flex gap-2">
          <Button onClick={close}>Cancel</Button>
          <Button
            variant="primary"
            onClick={() => {
              onSave([...chosen]);
              close();
            }}
          >
            Save
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
