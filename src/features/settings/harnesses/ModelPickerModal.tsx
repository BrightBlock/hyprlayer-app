import { useMemo } from "react";

import type { CatalogProvider } from "@/lib/bindings";
import { isFreeOpencodeModel } from "@/lib/domain/harnessing";
import { ModelMultiSelectDialog, type ModelRowGroup } from "./ModelMultiSelectDialog";

/** OpenCode's model picker: groups the models.dev catalog by provider and applies
 *  OpenCode's keyed/free-provider rules, then renders the shared multi-select
 *  shell. Models are keyed `provider/model`; the caller persists the chosen ids
 *  (Settings → providers.opencode.selectedModels). */
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
  const groups: ModelRowGroup[] = useMemo(
    () =>
      providers
        .map((p) => {
          const configured = !onlyConfigured || onlyConfigured.has(p.id);
          // An unconfigured provider still offers its free (keyless) models,
          // OpenCode Zen's `-free` models authenticate without an API key.
          const models = configured ? p.models : p.models.filter((m) => isFreeOpencodeModel(m.id));
          return {
            configured,
            group: {
              key: p.id,
              label: p.name,
              models: models.map((m) => ({ id: `${p.id}/${m.id}`, name: m.name })),
            },
          };
        })
        .filter((g) => g.group.models.length > 0)
        // Configured (keyed) providers stay on top; keyless free-only ones sink
        // below. Sort is stable, so each tier keeps its alphabetical order.
        .sort((a, b) => Number(b.configured) - Number(a.configured))
        .map((g) => g.group),
    [providers, onlyConfigured],
  );

  // Unmount while closed rather than passing `open={false}` through: the shared
  // dialog seeds its draft selection at mount, so staying mounted would let that
  // draft go stale against `selected`.
  if (!open) return null;

  return (
    <ModelMultiSelectDialog
      open
      onClose={onClose}
      onSave={onSave}
      groups={groups}
      selected={selected}
    />
  );
}
