import type { RefObject } from "react";

import { Button, DropdownSelect, Field, Popover, TextInput } from "@/components/ui";
import type { NameCount } from "@/lib/bindings";
import { DOC_SCOPE_LABEL, DOC_SCOPES, DOC_STATUS_LABEL, DOC_STATUSES, DOC_TYPE_META, DOC_TYPES } from "@/lib/domain/docs";
import { asLibraryBox, boxQuery } from "./libraryBoxes";
import type { LibrarySearch } from "./librarySearch";

export const listFiltersActive = (s: LibrarySearch): boolean =>
  Boolean(s.type || s.status || s.scope || s.author || s.tag);

const CLEARED = { type: undefined, status: undefined, scope: undefined, author: undefined, tag: undefined };

export function ListFilterPopover({
  open,
  onClose,
  anchorRef,
  search,
  onUpdate,
  authors,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  search: LibrarySearch;
  onUpdate: (patch: Partial<LibrarySearch>) => void;
  authors: NameCount[];
}) {
  // The box wins on its own dimension, so its control would do nothing.
  const owned = boxQuery(asLibraryBox(search.box));
  const single = (value: string) => value || undefined;

  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      align="end"
      trapFocus
      aria-label="Filters"
      className="max-h-96 w-64 space-y-2.5 overflow-y-auto p-3"
    >
      {!("types" in owned) && (
        <Field label="Type">
          <DropdownSelect
            aria-label="Type"
            value={search.type ?? ""}
            options={[
              { value: "", label: "All types" },
              ...DOC_TYPES.map((t) => ({ value: t, label: DOC_TYPE_META[t].label })),
            ]}
            onChange={(v) => onUpdate({ type: single(v) })}
          />
        </Field>
      )}
      {!("statuses" in owned) && (
        <Field label="Status">
          <DropdownSelect
            aria-label="Status"
            value={search.status ?? ""}
            options={[
              { value: "", label: "All statuses" },
              ...DOC_STATUSES.map((s) => ({ value: s, label: DOC_STATUS_LABEL[s] })),
            ]}
            onChange={(v) => onUpdate({ status: single(v) })}
          />
        </Field>
      )}
      <Field label="Scope">
        <DropdownSelect
          aria-label="Scope"
          value={search.scope ?? ""}
          options={[
            { value: "", label: "All scopes" },
            ...DOC_SCOPES.map((s) => ({ value: s, label: DOC_SCOPE_LABEL[s] })),
          ]}
          onChange={(v) => onUpdate({ scope: single(v) })}
        />
      </Field>
      <Field label="Author">
        <DropdownSelect
          aria-label="Author"
          value={search.author ?? ""}
          options={[{ value: "", label: "All authors" }, ...authors.map((a) => ({ value: a.name, label: a.name }))]}
          onChange={(v) => onUpdate({ author: single(v) })}
        />
      </Field>
      <Field label="Tag">
        <TextInput
          value={search.tag ?? ""}
          onChange={(e) => onUpdate({ tag: single(e.target.value) })}
          placeholder="Exact tag"
        />
      </Field>
      <Button
        size="sm"
        disabled={!listFiltersActive(search)}
        onClick={() => onUpdate(CLEARED)}
        className="w-full justify-center"
      >
        Clear filters
      </Button>
    </Popover>
  );
}
