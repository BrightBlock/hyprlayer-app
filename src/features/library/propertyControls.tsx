import { openUrl } from "@tauri-apps/plugin-opener";
import { useMemo, useState, type ReactNode } from "react";

import { cx } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { initialsOf } from "@/lib/format";
import { useDocs } from "@/lib/queries";

const CONTROL =
  "h-7 w-full rounded-control border border-transparent bg-transparent px-2 text-body text-ink " +
  "hover:border-edge hover:bg-panel focus:border-edge focus:bg-panel focus:outline-none disabled:opacity-60";

export function PropertyRow({ id, label, children }: { id: string; label: string; children: ReactNode }) {
  return (
    <div>
      <label htmlFor={id} id={`${id}-label`} className="mb-1 block text-label text-ink-dim">
        {label}
      </label>
      {children}
    </div>
  );
}

// An off-list current value (e.g. a hand-edited status) stays selectable.
function optionsFor(canonical: readonly string[], current: string) {
  const values = current && !canonical.includes(current) ? [current, ...canonical] : [...canonical];
  return [{ value: "", label: "—" }, ...values.map((v) => ({ value: v, label: v }))];
}

type EnumSelectProps = {
  id: string;
  value: string;
  canonical: readonly string[];
  placeholder?: string;
  onChange: (next: string) => void;
};

export function EnumSelect({ id, value, canonical, placeholder, onChange }: EnumSelectProps) {
  const options = optionsFor(canonical, value);
  return (
    <select
      id={id}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className={cx(CONTROL, "font-semibold")}
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.value === "" && placeholder ? `— (${placeholder})` : o.label}
        </option>
      ))}
    </select>
  );
}

const DATE_SHAPE = /^\d{4}-\d{2}-\d{2}$/;

type FieldProps = { id: string; value: string; placeholder?: string; onChange: (next: string) => void };

export function DateField({ id, value, placeholder, onChange }: FieldProps) {
  const isDateShape = value === "" || DATE_SHAPE.test(value);
  return (
    <input
      id={id}
      type={isDateShape ? "date" : "text"}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={cx(CONTROL, isDateShape && "font-mono")}
    />
  );
}

export function TextField({ id, value, placeholder, onChange }: FieldProps) {
  return (
    <input
      id={id}
      type="text"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={CONTROL}
    />
  );
}

const URL_SHAPE = /^https?:\/\//i;

export function TicketField({ id, value, placeholder, onChange }: FieldProps) {
  const [editing, setEditing] = useState(false);
  const isLink = URL_SHAPE.test(value) && !editing;

  if (isLink) {
    return (
      <div className="flex items-center gap-1">
        <a
          id={id}
          href={value}
          onClick={(e) => {
            e.preventDefault();
            void openUrl(value);
          }}
          className={cx(CONTROL, "flex min-w-0 items-center truncate text-info underline")}
        >
          {value}
        </a>
        <button
          type="button"
          onClick={() => setEditing(true)}
          aria-label="Edit ticket"
          className="grid h-6 w-6 shrink-0 place-items-center rounded hover:bg-edge/60"
        >
          <Icon name="edit" size="xs" />
        </button>
      </div>
    );
  }

  return (
    <input
      id={id}
      type="text"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      onBlur={() => setEditing(false)}
      placeholder={placeholder}
      className={CONTROL}
    />
  );
}

export function AuthorField({ id, value, placeholder, onChange }: FieldProps) {
  const docsQuery = useDocs({});
  const authors = useMemo(() => {
    const seen = new Set<string>();
    for (const meta of docsQuery.data?.items ?? []) {
      const author = meta.fm.author?.trim();
      if (author) seen.add(author);
    }
    return Array.from(seen).sort();
  }, [docsQuery.data]);

  return (
    <div className="flex items-center gap-1.5">
      <span className="grid h-5 w-5 shrink-0 place-items-center rounded-full bg-panel-2 text-tag text-ink-dim">
        {initialsOf(value || placeholder || "")}
      </span>
      <input
        id={id}
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        list="fm-author-options"
        className={CONTROL}
      />
      <datalist id="fm-author-options">
        {authors.map((a) => (
          <option key={a} value={a} />
        ))}
      </datalist>
    </div>
  );
}

export function TagsField({ id, value, onChange }: { id: string; value: string[]; onChange: (next: string[]) => void }) {
  const [adding, setAdding] = useState(false);
  const [draftTag, setDraftTag] = useState("");

  const commit = () => {
    const tag = draftTag.trim();
    setDraftTag("");
    setAdding(false);
    if (!tag || value.includes(tag)) return;
    onChange([...value, tag]);
  };

  const removeTag = (tag: string) => {
    onChange(value.filter((t) => t !== tag));
  };

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {value.map((tag) => (
        <span key={tag} className="inline-flex items-center gap-1 rounded-control bg-panel-2 px-1.5 text-ui text-ink">
          {tag}
          <button
            type="button"
            onClick={() => removeTag(tag)}
            className="text-ink-dim hover:text-ink"
            aria-label={`Remove tag ${tag}`}
          >
            <Icon name="close" size="xs" />
          </button>
        </span>
      ))}
      {adding ? (
        <input
          id={id}
          autoFocus
          value={draftTag}
          onChange={(e) => setDraftTag(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === ",") {
              e.preventDefault();
              commit();
            } else if (e.key === "Escape") {
              setDraftTag("");
              setAdding(false);
            }
          }}
          placeholder="new tag"
          className="w-24 rounded-control border border-edge bg-panel-2 px-1.5 py-0.5 text-ui text-ink placeholder:text-ink-dim/70 focus:border-ink-dim/70 focus:outline-none"
        />
      ) : (
        <button
          id={id}
          type="button"
          onClick={() => setAdding(true)}
          className="rounded-control border border-dashed border-edge px-1.5 py-0.5 text-ui text-ink-dim hover:border-ink-dim/70 hover:text-ink"
        >
          + tag
        </button>
      )}
    </div>
  );
}
