import {
  useEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import {
  validateBranchPrefix,
  validatePrefixList,
  type BranchPrefix,
} from "@/lib/domain/branchPrefix";
import { cx, IconButton, TextInput } from "@/components/ui";
import { PencilIcon, PlusIcon, XIcon } from "@/components/icons";

type Props = {
  title: ReactNode;
  description?: ReactNode;
  value: BranchPrefix[];
  onChange: (next: BranchPrefix[]) => void;
  disabled?: boolean;
  blockReseed?: boolean;
  inheritedNote: ReactNode;
};

type DraftRow = { label: string; prefix: string; keywords: string };

type AddDraft = { label: string; prefix: string; keywords: string };

function toDraft(list: BranchPrefix[]): DraftRow[] {
  return list.map((p) => ({
    label: p.label,
    prefix: p.prefix,
    keywords: (p.keywords ?? []).join(", "),
  }));
}

function toRecords(
  rows: { label: string; prefix: string; keywords: string }[],
): BranchPrefix[] {
  return rows.map((r) => ({
    label: r.label.trim(),
    prefix: r.prefix.trim(),
    keywords: r.keywords
      .split(",")
      .map((k) => k.trim().toLowerCase())
      .filter(Boolean),
  }));
}

const INVALID_FIELD = "border-bad/60 focus:border-bad";
const COMPACT_FIELD = "py-1";

const ROW_BUTTON =
  "shrink-0 rounded px-1 py-0.5 text-xs text-ink-dim hover:bg-edge/60 disabled:cursor-not-allowed disabled:opacity-50";

const BLANK_ADD: AddDraft = { label: "", prefix: "", keywords: "" };

export function BranchPrefixEditor({
  title,
  description,
  value,
  onChange,
  disabled,
  blockReseed,
  inheritedNote,
}: Props) {
  const [draft, setDraft] = useState<DraftRow[]>(() => toDraft(value));
  const lastSent = useRef(JSON.stringify(value));
  const [editing, setEditing] = useState<Set<number>>(new Set());

  useEffect(() => {
    if (blockReseed) return;
    const next = JSON.stringify(value);
    if (next === lastSent.current) return;
    lastSent.current = next;
    setDraft(toDraft(value));
    setEditing(new Set());
  }, [value, blockReseed]);

  const rowErrors = validatePrefixList(draft);

  function commit(next: DraftRow[]) {
    if (validatePrefixList(next).size > 0) return;
    const records = toRecords(next);
    lastSent.current = JSON.stringify(records);
    onChange(records);
  }

  function edit(i: number, patch: Partial<DraftRow>) {
    setDraft((rows) => rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  }

  function remove(i: number) {
    const next = draft.filter((_, j) => j !== i);
    setDraft(next);
    setEditing(new Set());
    commit(next);
  }

  function move(i: number, delta: number) {
    const j = i + delta;
    if (j < 0 || j >= draft.length) return;
    const next = [...draft];
    [next[i], next[j]] = [next[j], next[i]];
    setDraft(next);
    setEditing(new Set());
    commit(next);
  }

  function doneEditing(i: number, rows: DraftRow[]) {
    if (validatePrefixList(rows).has(i)) return;
    setEditing((prev) => {
      const next = new Set(prev);
      next.delete(i);
      return next;
    });
  }

  const rowCommitProps = (i: number) => ({
    onBlur: (e: FocusEvent<HTMLInputElement>) => {
      commit(draft);
      if (
        e.currentTarget.closest("li")?.contains(e.relatedTarget as Node | null)
      )
        return;
      doneEditing(i, draft);
    },
    onKeyDown: (e: KeyboardEvent) => {
      if (e.key !== "Enter") return;
      commit(draft);
      doneEditing(i, draft);
    },
  });

  const [adding, setAdding] = useState<AddDraft | null>(null);
  const addPrefix = adding?.prefix.trim() ?? "";
  const addPrefixRule = addPrefix ? validateBranchPrefix(addPrefix) : null;
  const addDuplicate =
    addPrefix && draft.some((r) => r.prefix.trim() === addPrefix)
      ? "Another branch type already uses this prefix."
      : null;
  const addError = addPrefixRule ? `Prefix ${addPrefixRule}.` : addDuplicate;
  const canAdd = !!adding && !!adding.label.trim() && !!addPrefix && !addError;

  function confirmAdd() {
    if (!adding || !canAdd) return;
    const row: DraftRow = {
      label: adding.label,
      prefix: adding.prefix,
      keywords: adding.keywords,
    };
    const next = [...draft, row];
    setDraft(next);
    commit(next);
    setAdding(null);
  }

  function cancelAdd() {
    setAdding(null);
  }

  function onAddKeyDown(e: KeyboardEvent) {
    if (e.key === "Enter") confirmAdd();
    else if (e.key === "Escape") cancelAdd();
  }

  const showEmptyState = draft.length === 0 && !adding;

  return (
    <div>
      <div className="flex items-center justify-between gap-3">
        {title}
        <div className="flex shrink-0 items-center gap-2">
          {showEmptyState && (
            <span className="flex items-center gap-1.5 text-xs text-ink-dim">
              <span
                aria-hidden="true"
                className="h-1.5 w-1.5 shrink-0 rounded-full bg-ink-dim/60"
              />
              {inheritedNote}
            </span>
          )}
          <IconButton
            disabled={disabled || !!adding}
            aria-label="Add branch type"
            title="Add branch type"
            onClick={() => setAdding(BLANK_ADD)}
          >
            <PlusIcon size={15} />
          </IconButton>
        </div>
      </div>
      {description && showEmptyState && (
        <p className="mb-3 mt-1.5 max-w-xl text-xs leading-relaxed text-ink-dim">
          {description}
        </p>
      )}
      {(draft.length > 0 || adding) && (
        <>
          <div className="mb-1 flex items-center gap-2 px-2.5">
            <span className="w-32 text-2xs font-semibold uppercase tracking-wider text-ink-dim">
              Label
            </span>
            <span className="w-28 text-2xs font-semibold uppercase tracking-wider text-ink-dim">
              Prefix
            </span>
            <span className="min-w-0 flex-1 text-2xs font-semibold uppercase tracking-wider text-ink-dim">
              Keywords
            </span>
          </div>
          <ul className="mb-3 space-y-0.5">
            {draft.map((row, i) => {
              const name = row.label.trim() || "type";
              const rowError = rowErrors.get(i);
              const isEditing = editing.has(i);
              return (
                <li key={i}>
                  <div className="flex items-center gap-2">
                    {!isEditing ? (
                      <>
                        <span
                          className="w-32 truncate px-2.5 py-1 text-sm text-ink"
                          title={row.label}
                        >
                          {row.label}
                        </span>
                        <span
                          className="w-28 truncate px-2.5 py-1 font-mono text-sm text-ink"
                          title={row.prefix}
                        >
                          {row.prefix}
                        </span>
                        <span
                          className="min-w-0 flex-1 truncate px-2.5 py-1 text-sm text-ink-dim"
                          title={row.keywords}
                        >
                          {row.keywords || "—"}
                        </span>
                        <button
                          type="button"
                          className={ROW_BUTTON}
                          onClick={() =>
                            setEditing((prev) => new Set(prev).add(i))
                          }
                          disabled={disabled}
                          aria-label={`Edit ${name}`}
                        >
                          <PencilIcon size={12} />
                        </button>
                      </>
                    ) : (
                      <>
                        <TextInput
                          aria-label="Label"
                          placeholder="Bug fix"
                          className={cx(
                            "w-32",
                            COMPACT_FIELD,
                            rowError?.label && INVALID_FIELD,
                          )}
                          value={row.label}
                          disabled={disabled}
                          onChange={(e) => edit(i, { label: e.target.value })}
                          {...rowCommitProps(i)}
                        />
                        <TextInput
                          aria-label="Prefix"
                          placeholder="fix/"
                          className={cx(
                            "w-28 font-mono",
                            COMPACT_FIELD,
                            rowError?.prefix && INVALID_FIELD,
                          )}
                          value={row.prefix}
                          disabled={disabled}
                          onChange={(e) => edit(i, { prefix: e.target.value })}
                          {...rowCommitProps(i)}
                        />
                        <div className="min-w-0 flex-1">
                          <TextInput
                            aria-label="Keywords"
                            className={COMPACT_FIELD}
                            placeholder={
                              'fix, bug, crash — single words; empty uses the label, "-" never suggests'
                            }
                            value={row.keywords}
                            disabled={disabled}
                            onChange={(e) =>
                              edit(i, { keywords: e.target.value })
                            }
                            {...rowCommitProps(i)}
                          />
                        </div>
                      </>
                    )}
                    <button
                      type="button"
                      className={ROW_BUTTON}
                      onClick={() => move(i, -1)}
                      disabled={disabled || i === 0}
                      aria-label={`Move ${name} up`}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      className={ROW_BUTTON}
                      onClick={() => move(i, 1)}
                      disabled={disabled || i === draft.length - 1}
                      aria-label={`Move ${name} down`}
                    >
                      ↓
                    </button>
                    <button
                      type="button"
                      className={`${ROW_BUTTON} hover:text-bad`}
                      onClick={() => remove(i)}
                      disabled={disabled}
                      aria-label={`Remove ${name}`}
                    >
                      ✕
                    </button>
                  </div>
                  {isEditing && (rowError?.label || rowError?.prefix) && (
                    <p role="alert" className="mt-1 pl-1 text-xs text-bad">
                      {rowError.label ?? rowError.prefix}
                    </p>
                  )}
                </li>
              );
            })}
            {adding && (
              <li className="rounded-md border border-dashed border-edge p-1.5">
                <div className="flex items-center gap-2">
                  <TextInput
                    aria-label="Label"
                    placeholder="Bug fix"
                    className={cx("w-32", COMPACT_FIELD)}
                    value={adding.label}
                    disabled={disabled}
                    autoFocus
                    onChange={(e) =>
                      setAdding({ ...adding, label: e.target.value })
                    }
                    onKeyDown={onAddKeyDown}
                  />
                  <TextInput
                    aria-label="Prefix"
                    placeholder="fix/"
                    className={cx(
                      "w-28 font-mono",
                      COMPACT_FIELD,
                      addError && INVALID_FIELD,
                    )}
                    value={adding.prefix}
                    disabled={disabled}
                    onChange={(e) =>
                      setAdding({ ...adding, prefix: e.target.value })
                    }
                    onKeyDown={onAddKeyDown}
                  />
                  <div className="min-w-0 flex-1">
                    <TextInput
                      aria-label="Keywords"
                      className={COMPACT_FIELD}
                      placeholder={
                        'fix, bug, crash — single words; empty uses the label, "-" never suggests'
                      }
                      value={adding.keywords}
                      disabled={disabled}
                      onChange={(e) =>
                        setAdding({ ...adding, keywords: e.target.value })
                      }
                      onKeyDown={onAddKeyDown}
                    />
                  </div>
                  <button
                    type="button"
                    className={`${ROW_BUTTON} hover:text-bad`}
                    onClick={cancelAdd}
                    disabled={disabled}
                    aria-label="Cancel add"
                  >
                    <XIcon size={12} />
                  </button>
                  <button
                    type="button"
                    className={`${ROW_BUTTON} text-accent`}
                    onClick={confirmAdd}
                    disabled={disabled || !canAdd}
                    aria-label="Add"
                  >
                    <PlusIcon size={12} />
                  </button>
                </div>
                {addError && (
                  <p role="alert" className="mt-1 pl-1 text-xs text-bad">
                    {addError}
                  </p>
                )}
              </li>
            )}
          </ul>
        </>
      )}
    </div>
  );
}
