import { useMemo } from "react";

import type { CatalogProvider } from "@/lib/bindings";
import { isFreeOpencodeModel } from "@/lib/domain/harnessing";
import { ModelMultiSelectDialog, type ModelRowGroup } from "./ModelMultiSelectDialog";

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
  onlyConfigured?: Set<string>;
}) {
  const groups: ModelRowGroup[] = useMemo(
    () =>
      providers
        .map((p) => {
          const configured = !onlyConfigured || onlyConfigured.has(p.id);
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
        .sort((a, b) => Number(b.configured) - Number(a.configured))
        .map((g) => g.group),
    [providers, onlyConfigured],
  );

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
