/**
 * Editor for a list of branch types ("Bug fix" → `fix/`), mounted twice: on the
 * repo's own list in `ProjectSettingsDialog` and on the global one in
 * Settings → Git. Presentation only — it holds no query and imports nothing
 * from `@/features` or `@/routes`, so each consumer owns its own IPC pair and
 * renders its own `ErrorNote`.
 *
 * `validate_prefixes` (`crates/hyprlayer-core/src/branch_prefix.rs`) remains the
 * AUTHORITY — it is what a direct IPC call or a hand-edited `settings.toml` has
 * to pass. But it is not the only copy: `validatePrefixList`
 * (`src/lib/domain/branchPrefix.ts`) mirrors it so a bad prefix marks its own
 * field as you type. Round-tripping a save to learn the answer put the message
 * in the consumer's `ErrorNote` at the top of the card, several hundred pixels
 * above the row it was about, and left the offending input unmarked.
 *
 * New types go through a SEPARATE add form (`adding`, below), not a live row in
 * the table. An earlier version pushed a blank row straight into the table and
 * relied on "isNew" to keep it out of the payload until both fields were filled
 * — which meant a row where the user filled in the label and keywords but never
 * the prefix stayed silently unsaved forever, with nothing on screen to say so.
 * The add form only touches the table at all once `confirmAdd` has a row that
 * passes validation, so there is never a half-real row for that bug to hide in.
 *
 * Every row in the table renders as TEXT, not as inputs — an added type is a
 * finished entry, and it stays one across a reopen or an app restart because
 * that is simply how a saved row renders, with no session state deciding it.
 * The pencil is the only way into inputs, tracked by `editing` below, which is
 * deliberately transient: nothing about "I am mid-edit" should outlive the
 * mount, let alone the process.
 */
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
  /** The section's heading, rendered at the left of the header row that carries
   *  the add button. A node, not a string, because the two surfaces style their
   *  headings differently — uppercase `SectionHeading` in the project dialog, a
   *  plain `text-sm` line in Settings → Git — and neither should have to give
   *  that up to share this row. */
  title: ReactNode;
  /** Prose under the header row, shown only while the list is empty — it is an
   *  invitation to configure, not a permanent caption. */
  description?: ReactNode;
  /** The list this surface OWNS — the repo's own list, or the global one.
   *  Never a resolved list; empty means "inherit". */
  value: BranchPrefix[];
  /** Called with the whole next list. Whole-list replace, matching
   *  `WorktreeFilesCard` (`ProjectSettingsDialog.tsx:71-80`). */
  onChange: (next: BranchPrefix[]) => void;
  disabled?: boolean;
  /** Hold the draft still: a save is in flight, or the last one was rejected and
   *  the user is still correcting it. Consumers pass `save.isPending || save.isError`. */
  blockReseed?: boolean;
  /** Shown when `value` is empty: what this surface inherits instead. */
  inheritedNote: ReactNode;
};

/**
 * One SAVED row in progress. `keywords` is the raw text of its cell, not the
 * record's `string[]`: splitting on every keystroke and re-joining for display
 * makes the separator undeletable — parse `"fix,"` to `["fix", ""]`, render it
 * back as `"fix, "`, and each backspace over the comma restores it. The parse
 * happens once, in [`toRecords`], on the same blur/Enter that commits.
 *
 * Every row here came from `value` — a row still being typed in lives in
 * `adding` instead, never here, so there is no "is this one still new"
 * question for [`commit`] to answer.
 */
type DraftRow = { label: string; prefix: string; keywords: string };

/** The row under construction in the add form, or none. A plain object rather
 *  than a `DraftRow` pushed into the table: see the module doc for why a
 *  half-typed type must not be a real row. */
type AddDraft = { label: string; prefix: string; keywords: string };

function toDraft(list: BranchPrefix[]): DraftRow[] {
  return list.map((p) => ({
    label: p.label,
    prefix: p.prefix,
    // `keywords` is optional in the binding because the Rust field carries
    // `#[serde(default)]`, so a hand-written record can omit it entirely.
    keywords: (p.keywords ?? []).join(", "),
  }));
}

/** Draft rows as records, normalized to the bytes the backend stores —
 *  `normalize_branch_prefixes` (`src-tauri/src/commands/mod.rs:80-94`) trims
 *  labels and prefixes and trims, lowercases and de-empties keywords. */
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

/** Same treatment as `WorktreeFilesCard`'s remove button
 *  (`ProjectSettingsDialog.tsx:96-104`): a small control sitting inside another
 *  surface, so `hover:bg-edge/60` per ADR-0006's inline tier rather than the
 *  menu/chrome `hover:bg-panel-2`. */
/** Red border on the field itself, matching `ErrorNote`'s own `border-bad/30`
 *  family. `cx` merges through `twMerge`, so this beats `TextInput`'s
 *  `border-edge` base (ADR-0006). */
const INVALID_FIELD = "border-bad/60 focus:border-bad";
/** `TextInput`'s own `py-1.5` is sized for a form, not a dense table — `cx`
 *  merges through `twMerge`, so this wins over that base and tightens every
 *  row (and the matching read-only `<span>`s below, kept in step by hand). */
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
  // An added type is FINISHED: it renders as text, and the only way back to
  // inputs is its pencil. So the state to track is the exception — which rows
  // are being edited right now — not which are read-only. Everything else
  // follows from that, including across a restart: this starts empty, every
  // row is text, and there is nothing session-shaped to persist or forget.
  //
  // An earlier version had this backwards, tracking "rows added this session"
  // as the read-only ones. That made a restart — where every row is simply a
  // saved row, not one added this session — put boxes back around all of them.
  //
  // Indices, not prefixes: the prefix is one of the fields being edited, so
  // keying on it would lose the row mid-keystroke. `remove`/`move` reshuffle
  // indices, so both clear this rather than trying to remap it.
  const [editing, setEditing] = useState<Set<number>>(new Set());

  useEffect(() => {
    // `blockReseed` is the consumer's `save.isPending || save.isError`. Without
    // it, a REJECTED save leaves the server list unchanged while `lastSent`
    // holds the rejected one, so the next refetch (`staleTime: 5_000` +
    // `refetchOnWindowFocus`, `src/lib/queryClient.ts:8-9`) would read them as
    // different and wipe the very text the ErrorNote is asking the user to fix.
    if (blockReseed) return;
    const next = JSON.stringify(value);
    if (next === lastSent.current) return; // our own round-trip echoing back
    lastSent.current = next;
    setDraft(toDraft(value));
    // A re-seed replaces the rows wholesale, so indices held here point at
    // rows that no longer exist — leaving them set opens an input on whatever
    // unrelated row inherited the index.
    setEditing(new Set());
  }, [value, blockReseed]);

  // Every row here is a saved one, so its errors show live — clearing a saved
  // prefix to type a new one is a real edit in progress, not a row still being
  // filled in for the first time. (That "still being filled in" case belongs
  // to `adding` below, which has its own, separately-gated validation.)
  const rowErrors = validatePrefixList(draft);

  function commit(next: DraftRow[]) {
    // Nothing leaves while ANY row is red. The inline message is already
    // saying what's wrong, and sending anyway would either lose the invalid
    // row's data or save everything else while it silently didn't — the
    // exact bug this field-level validation exists to prevent.
    if (validatePrefixList(next).size > 0) return;
    const records = toRecords(next);
    lastSent.current = JSON.stringify(records);
    onChange(records);
  }

  /** Replace one row's text without committing, so an intermediate `fix//`
   *  typed on the way to `fix/x/` never reaches `validate_prefix`. */
  function edit(i: number, patch: Partial<DraftRow>) {
    setDraft((rows) => rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  }

  function remove(i: number) {
    const next = draft.filter((_, j) => j !== i);
    setDraft(next);
    // Indices shift under a removal, so a kept `editing` set would point at
    // the wrong rows. Nothing is lost by closing them: every row's text is
    // already in `draft`.
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

  /** Leave edit mode for row `i`, but only once it is actually valid — a red
   *  row that closed on blur would hide the very message telling the user why
   *  nothing saved. */
  function doneEditing(i: number, rows: DraftRow[]) {
    if (validatePrefixList(rows).has(i)) return;
    setEditing((prev) => {
      const next = new Set(prev);
      next.delete(i);
      return next;
    });
  }

  /** Persist on blur and on Enter, the same idiom Settings → Git's own prefix
   *  input uses (`Settings.tsx:240-247`) — and, unlike that one, drop back to
   *  the read-only rendering afterwards, since an edited row is finished for
   *  the same reason a just-added one is. */
  const rowCommitProps = (i: number) => ({
    onBlur: (e: FocusEvent<HTMLInputElement>) => {
      commit(draft);
      // Tabbing between this row's own fields is not "done with the row".
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
  // Unlike `rowErrors` above, a still-BLANK field here is never an error, only
  // a reason Add stays disabled — the user has not finished, and shouting
  // "needs a prefix" while they are still typing the label (before they have
  // even reached the prefix field) is the premature-flash the old "isNew" row
  // used to produce. Only a PREFIX that is actually wrong — bad characters, or
  // a duplicate of a saved type's — earns the red border and the message.
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
      {/* The add button rides the heading row rather than sitting under the
          list, so the section reads as one band whatever the row count — and an
          empty list doesn't leave a lone button floating in whitespace. */}
      <div className="flex items-center justify-between gap-3">
        {title}
        <div className="flex shrink-0 items-center gap-2">
          {/* An empty list is a STATE, not an instruction, so it reads as a
              status dot rather than a sentence. The words stay alongside the dot
              because nothing on screen is a legend for it — a bare dot could not
              tell a reader whether the built-ins or their global list is in
              force. Hidden while adding: a row is actively being built, so
              "still inherited" would be misleading noise next to the form. */}
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
      {/* Empty-state copy only. Once the list has a row (or the add form is
          open) the invitation has been accepted, and the rows themselves say
          what the section is — leaving the prose up would just push them
          down the page. */}
      {description && showEmptyState && (
        <p className="mb-3 mt-1.5 max-w-xl text-xs leading-relaxed text-ink-dim">
          {description}
        </p>
      )}
      {(draft.length > 0 || adding) && (
        <>
          {/* Labels the three text columns; the icon-button column to their
              right stays unlabeled, the same way a table's action column
              usually does. Same widths as the cells below so the words sit
              directly over what they name, not just near it. */}
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
            {/* Keyed by index: records have no identity key — the same fact that
              made wholesale replace the right override semantics. The inputs are
              fully controlled from `draft[i]`, so an index key is correct across
              add/remove/reorder; it costs focus following a reorder, nothing
              else. */}
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
                  {/* One line per row, not per field: both fields can only be
                    wrong in ways that share a cause, and a second line would
                    reflow the list on every keystroke. Only while editing —
                    a text row's content was accepted by `commit` to get here. */}
                  {isEditing && (rowError?.label || rowError?.prefix) && (
                    <p role="alert" className="mt-1 pl-1 text-xs text-bad">
                      {rowError.label ?? rowError.prefix}
                    </p>
                  )}
                </li>
              );
            })}
            {/* The add form. A dashed border marks it as not a real row yet —
              nothing here reaches `draft`, let alone the backend, before
              `confirmAdd` has a row that passes validation. */}
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
