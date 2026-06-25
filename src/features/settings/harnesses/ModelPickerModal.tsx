import { useMemo, useState } from "react";

import type { CatalogProvider } from "../../../lib/bindings";
import { isFreeOpencodeModel } from "../../session/harnessing";
import { Button, Dialog, TextInput } from "../../../components/ui";
import { SearchIcon } from "../../../components/icons";

/** Grouped, multi-select model picker. Models are keyed `provider/model`. The
 *  caller persists the chosen ids (Settings → providers.opencode.selectedModels). */
export function ModelPickerModal({
  open,
  onClose,
  providers,
  selected,
  onSave,
  onlyConfigured,
}: {
  open: boolean;
  onClose: () => void;
  providers: CatalogProvider[];
  selected: string[];
  onSave: (next: string[]) => void;
  /** When set, providers outside this set contribute only their keyless `-free`
   *  models; configured providers (those with a key) contribute all of theirs. */
  onlyConfigured?: Set<string>;
}) {
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(selected));

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    return providers
      .map((p) => {
        const configured = !onlyConfigured || onlyConfigured.has(p.id);
        let models = p.models.filter(
          (m) =>
            !q ||
            m.name.toLowerCase().includes(q) ||
            m.id.toLowerCase().includes(q) ||
            p.name.toLowerCase().includes(q),
        );
        // An unconfigured provider still offers its free (keyless) models —
        // OpenCode Zen's `-free` models authenticate without an API key.
        if (!configured) models = models.filter((m) => isFreeOpencodeModel(m.id));
        return { provider: p, configured, models };
      })
      .filter((g) => g.models.length > 0)
      // Providers the user deliberately added (with a key) stay on top; the
      // keyless free-only providers sink to the bottom so they don't clutter
      // the list. Sort is stable, so each tier keeps its alphabetical order.
      .sort((a, b) => Number(b.configured) - Number(a.configured));
  }, [providers, query, onlyConfigured]);

  function toggle(id: string, on: boolean) {
    setChosen((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  function close() {
    setQuery("");
    setChosen(new Set(selected));
    onClose();
  }

  return (
    <Dialog open={open} onClose={close} title="Add models" className="max-w-xl">
      <div className="relative mb-3">
        <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-dim">
          <SearchIcon />
        </span>
        <TextInput
          autoFocus
          className="pl-8"
          placeholder="Filter models"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      <div className="max-h-[24rem] overflow-y-auto">
        {groups.length === 0 && (
          <p className="px-1 py-6 text-center text-sm text-ink-dim">
            {query ? `No models match “${query}”.` : "No models available."}
          </p>
        )}
        {groups.map(({ provider, models }) => (
          <div key={provider.id} className="mb-3">
            <p className="px-1 py-1 text-xs font-semibold uppercase tracking-wide text-ink-dim">
              {provider.name}
            </p>
            <ul>
              {models.map((m) => {
                const id = `${provider.id}/${m.id}`;
                return (
                  <li key={id}>
                    <label className="flex cursor-pointer items-center gap-2.5 rounded px-1 py-1.5 hover:bg-edge/40">
                      <input
                        type="checkbox"
                        className="h-4 w-4 shrink-0 cursor-pointer accent-accent"
                        checked={chosen.has(id)}
                        onChange={(e) => toggle(id, e.target.checked)}
                      />
                      <span className="min-w-0">
                        <span className="text-sm text-ink">{m.name}</span>
                        <span className="block font-mono text-xs text-ink-dim">{id}</span>
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>

      <div className="mt-4 flex items-center justify-between">
        <span className="text-xs text-ink-dim">{chosen.size} selected</span>
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
