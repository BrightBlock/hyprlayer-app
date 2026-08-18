import { useState, type FormEvent } from "react";

import type { DocMeta, FmPatchDto, FmSetEntry } from "@/lib/bindings";
import { Button, DropdownSelect, Field, TextInput } from "@/components/ui";
import { DOC_SCOPES, DOC_STATUSES, DOC_TYPES } from "./constants";
import { parseTagsInput } from "./text";

const STRING_KEYS = ["title", "type", "date", "status", "ticket", "project", "scope", "author"] as const;
type StringKey = (typeof STRING_KEYS)[number];
type FormValues = Record<StringKey, string> & { tags: string };

/** Keep a non-canonical current value selectable instead of silently dropping it,
 *  plus a leading em-dash option for clearing the field. */
function optionsFor(canonical: readonly string[], current: string) {
  const values = current && !canonical.includes(current) ? [current, ...canonical] : [...canonical];
  return [{ value: "", label: "—" }, ...values.map((v) => ({ value: v, label: v }))];
}

type Props = {
  meta: DocMeta;
  /** True while the editor buffer is dirty, patching would clobber it. */
  disabled: boolean;
  busy: boolean;
  onApply: (patch: FmPatchDto) => void;
};

/**
 * Edits the 10-field schema and submits only the changed fields as an
 * FmPatchDto (cleared fields become removals). Remount (via `key`) when the
 * underlying document changes so the form reseeds.
 */
export function FrontmatterForm({ meta, disabled, busy, onApply }: Props) {
  const fm = meta.fm;
  const [values, setValues] = useState<FormValues>(() => ({
    title: fm.title ?? "",
    type: fm.type ?? "",
    date: fm.date ?? "",
    status: fm.status ?? "",
    ticket: fm.ticket ?? "",
    project: fm.project ?? "",
    scope: fm.scope ?? "",
    author: fm.author ?? "",
    tags: (fm.tags ?? []).join(", "),
  }));

  const setValue = (key: keyof FormValues, value: string) =>
    setValues((prev) => ({ ...prev, [key]: value }));

  function buildPatch(): FmPatchDto {
    const set: FmSetEntry[] = [];
    const remove: string[] = [];
    for (const key of STRING_KEYS) {
      const original = (fm[key] ?? "").trim();
      const next = values[key].trim();
      if (next === original) continue;
      if (next === "") remove.push(key);
      else set.push({ key, value: { kind: "str", value: next } });
    }
    const originalTags = fm.tags ?? [];
    const nextTags = parseTagsInput(values.tags);
    if (nextTags.join("\0") !== originalTags.join("\0")) {
      if (nextTags.length === 0) remove.push("tags");
      else set.push({ key: "tags", value: { kind: "list", values: nextTags } });
    }
    return { set, remove };
  }

  const patch = buildPatch();
  const hasChanges = (patch.set?.length ?? 0) + (patch.remove?.length ?? 0) > 0;

  function submit(e: FormEvent) {
    e.preventDefault();
    if (disabled || busy || !hasChanges) return;
    onApply(patch);
  }

  const selectField = (label: string, key: StringKey, canonical: readonly string[]) => (
    <Field label={label}>
      <DropdownSelect
        aria-label={label}
        value={values[key]}
        options={optionsFor(canonical, values[key])}
        onChange={(v) => setValue(key, v)}
      />
    </Field>
  );

  const textField = (label: string, key: StringKey, placeholder?: string) => (
    <Field label={label}>
      <TextInput
        value={values[key]}
        onChange={(e) => setValue(key, e.target.value)}
        placeholder={placeholder}
      />
    </Field>
  );

  return (
    <form onSubmit={submit}>
      <fieldset disabled={disabled || busy} className="grid grid-cols-2 gap-3">
        <div className="col-span-2">{textField("Title", "title")}</div>
        {selectField("Type", "type", DOC_TYPES)}
        {selectField("Status", "status", DOC_STATUSES)}
        {selectField("Scope", "scope", DOC_SCOPES)}
        {textField("Date", "date", "YYYY-MM-DD")}
        {textField("Ticket", "ticket")}
        {textField("Project", "project")}
        <div className="col-span-2">{textField("Author", "author")}</div>
        <div className="col-span-2">
          <Field label="Tags (comma-separated)">
            <TextInput
              value={values.tags}
              onChange={(e) => setValue("tags", e.target.value)}
              placeholder="refactor, ipc, phase-1"
            />
          </Field>
        </div>
      </fieldset>
      <div className="mt-3 flex justify-end">
        <Button type="submit" variant="primary" disabled={disabled || !hasChanges} busy={busy}>
          Apply frontmatter
        </Button>
      </div>
    </form>
  );
}
