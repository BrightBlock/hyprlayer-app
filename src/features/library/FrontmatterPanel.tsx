import { useState, type ReactNode } from "react";

import type { DocMeta, FmPatchDto } from "@/lib/bindings";
import { DropdownSelect, TextInput } from "@/components/ui";
import { DOC_STATUSES, DOC_TYPES } from "./constants";

/** Keep a non-canonical current value selectable instead of silently dropping it,
 *  plus a leading em-dash option for clearing the field. */
function optionsFor(canonical: readonly string[], current: string) {
  const values = current && !canonical.includes(current) ? [current, ...canonical] : [...canonical];
  return [{ value: "", label: "—" }, ...values.map((v) => ({ value: v, label: v }))];
}

function Labeled({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-[10px] font-medium uppercase tracking-wider text-ink-dim">
        {label}
      </span>
      {children}
    </label>
  );
}

type Props = {
  meta: DocMeta;
  /** True while the editor buffer is dirty, patching would clobber it. */
  disabled: boolean;
  busy: boolean;
  /** Apply a single-field frontmatter patch (set or remove). */
  onApply: (patch: FmPatchDto) => void;
};

/**
 * Inline frontmatter card; each field applied on change as a one-field FmPatchDto.
 * Patching writes the file (mtime-guarded), so the card is disabled while the body
 * buffer is dirty, parent remounts it (`key`) to reseed values + base mtime.
 */
export function FrontmatterPanel({ meta, disabled, busy, onApply }: Props) {
  const fm = meta.fm;
  const [type, setType] = useState(fm.type ?? "");
  const [status, setStatus] = useState(fm.status ?? "");
  const [author, setAuthor] = useState(fm.author ?? "");
  const [date, setDate] = useState(fm.date ?? "");
  const [tags, setTags] = useState<string[]>(fm.tags ?? []);
  const [adding, setAdding] = useState(false);
  const [draftTag, setDraftTag] = useState("");

  const locked = disabled || busy;

  // Apply one string field if it actually changed; a cleared value is a removal.
  const applyField = (key: string, next: string) => {
    const original = (fm[key as keyof typeof fm] as string | null | undefined) ?? "";
    const value = next.trim();
    if (value === original.trim()) return;
    onApply(value === "" ? { remove: [key] } : { set: [{ key, value: { kind: "str", value } }] });
  };

  const applyTags = (next: string[]) => {
    const original = fm.tags ?? [];
    if (next.join("\0") === original.join("\0")) return;
    onApply(
      next.length === 0
        ? { remove: ["tags"] }
        : { set: [{ key: "tags", value: { kind: "list", values: next } }] },
    );
  };

  const addDraftTag = () => {
    const tag = draftTag.trim();
    setDraftTag("");
    setAdding(false);
    if (!tag || tags.includes(tag)) return;
    const next = [...tags, tag];
    setTags(next);
    applyTags(next);
  };

  const removeTag = (tag: string) => {
    const next = tags.filter((t) => t !== tag);
    setTags(next);
    applyTags(next);
  };

  return (
    <section aria-label="Frontmatter">
      <div className="mb-2 flex items-center justify-between">
        <h2 className="text-[11px] font-semibold uppercase tracking-wider text-ink-dim">
          Frontmatter
        </h2>
        <span className="text-[11px] text-ink-dim/70">YAML · git-tracked</span>
      </div>
      <fieldset
        disabled={locked}
        className="rounded-lg border border-edge bg-panel-2/40 p-4 disabled:opacity-60"
      >
        <div className="grid grid-cols-2 gap-x-4 gap-y-3">
          <Labeled label="Type">
            <DropdownSelect
              aria-label="Type"
              value={type}
              options={optionsFor(DOC_TYPES, type)}
              onChange={(v) => {
                setType(v);
                applyField("type", v);
              }}
            />
          </Labeled>
          <Labeled label="Status">
            <DropdownSelect
              aria-label="Status"
              value={status}
              options={optionsFor(DOC_STATUSES, status)}
              onChange={(v) => {
                setStatus(v);
                applyField("status", v);
              }}
            />
          </Labeled>
          <Labeled label="Author">
            <TextInput
              value={author}
              onChange={(e) => setAuthor(e.target.value)}
              onBlur={() => applyField("author", author)}
              placeholder="—"
            />
          </Labeled>
          <Labeled label="Updated">
            <TextInput
              value={date}
              onChange={(e) => setDate(e.target.value)}
              onBlur={() => applyField("date", date)}
              placeholder="YYYY-MM-DD"
            />
          </Labeled>
        </div>

        <div className="mt-3">
          <span className="mb-1 block text-[10px] font-medium uppercase tracking-wider text-ink-dim">
            Tags
          </span>
          <div className="flex flex-wrap items-center gap-1.5">
            {tags.map((tag) => (
              <span
                key={tag}
                className="inline-flex items-center gap-1 rounded bg-panel px-1.5 py-0.5 text-xs text-ink"
              >
                {tag}
                <button
                  type="button"
                  onClick={() => removeTag(tag)}
                  className="text-ink-dim hover:text-ink"
                  aria-label={`Remove tag ${tag}`}
                >
                  ×
                </button>
              </span>
            ))}
            {adding ? (
              <input
                autoFocus
                value={draftTag}
                onChange={(e) => setDraftTag(e.target.value)}
                onBlur={addDraftTag}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addDraftTag();
                  } else if (e.key === "Escape") {
                    setDraftTag("");
                    setAdding(false);
                  }
                }}
                placeholder="new tag"
                className="w-24 rounded border border-edge bg-panel-2 px-1.5 py-0.5 text-xs text-ink placeholder:text-ink-dim/70 focus:border-ink-dim/70 focus:outline-none"
              />
            ) : (
              <button
                type="button"
                onClick={() => setAdding(true)}
                className="rounded border border-dashed border-edge px-1.5 py-0.5 text-xs text-ink-dim hover:border-ink-dim/70 hover:text-ink"
              >
                + add
              </button>
            )}
          </div>
        </div>
      </fieldset>
    </section>
  );
}
