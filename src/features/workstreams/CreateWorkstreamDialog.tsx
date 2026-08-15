import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { Button, Dialog, DropdownSelect, ErrorNote, Field, TextInput } from "@/components/ui";
import { PlusIcon, SplitIcon } from "@/components/icons";
import type { WorktreeMode } from "@/lib/bindings";
import { useCreateWorkstream, useWorkstreamBases } from "@/lib/queries";

type Props = {
  projectPath: string;
  open: boolean;
  onClose: () => void;
  /** Called with the freshly created item so the caller can select its tab. */
  onCreated: (view: WorkstreamView) => void;
  /** A folder project: no git, so the dialog is title-only. */
  folder?: boolean;
};

/** Where work on an existing branch lives: a dedicated worktree, or the
 * project's own checkout (no worktree). */
type Place = "worktree" | "here";

/** Prefix matches first, then any substring; non-matches dropped. Mirrors the
 * composer's command filter so branch search feels the same. */
function filterBranches(all: string[], query: string): string[] {
  const q = query.trim().toLowerCase();
  if (!q) return all;
  const prefix: string[] = [];
  const substr: string[] = [];
  for (const b of all) {
    const name = b.toLowerCase();
    if (name.startsWith(q)) prefix.push(b);
    else if (name.includes(q)) substr.push(b);
  }
  return [...prefix, ...substr];
}

/**
 * New-workstream dialog. A single branch combobox is the heart of it: the
 * pinned "New branch" row (the default) creates a fresh branch + worktree from a
 * base; picking an existing local branch instead reveals a Worktree/Here choice
 * — Worktree checks it out into a dedicated worktree, Here switches the
 * project's own checkout to it with no worktree. There is no mode dropdown; the
 * branch you choose determines the shape. The list is local branches only — a
 * remote-tracking ref would detach HEAD.
 *
 * A folder project has none of that: the dialog collapses to a title, and the
 * workstream runs in the project folder itself.
 */
export function CreateWorkstreamDialog({
  projectPath,
  open,
  onClose,
  onCreated,
  folder = false,
}: Props) {
  const [title, setTitle] = useState("");
  const [query, setQuery] = useState("");
  // null = the pinned "New branch" row; otherwise the chosen existing branch.
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null);
  const [base, setBase] = useState("");
  // Whether the user has explicitly chosen a base, vs. it being auto-defaulted.
  const [userPickedBase, setUserPickedBase] = useState(false);
  const [place, setPlace] = useState<Place>("worktree");
  const create = useCreateWorkstream(projectPath);
  // A folder project has no branches to list — don't even ask.
  const bases = useWorkstreamBases(projectPath, open && !folder);

  const defaultBase = bases.data?.defaultBase ?? "";
  const currentBranch = bases.data?.currentBranch ?? "";
  // Memoized so identities are stable across renders — otherwise the `?? []`
  // literals would re-fire the default-selection effect every render.
  const branches = useMemo(() => bases.data?.branches ?? [], [bases.data?.branches]);
  const localBranches = useMemo(
    () => bases.data?.localBranches ?? [],
    [bases.data?.localBranches],
  );
  const filtered = useMemo(() => filterBranches(localBranches, query), [localBranches, query]);

  const isNew = selectedBranch === null;
  // The current branch is already checked out in the project's own tree, so it
  // can't also be checked out into a dedicated worktree — git refuses a branch
  // already checked out elsewhere. Only "My checkout" (in-place) is valid for it.
  const currentSelected = !isNew && !!currentBranch && selectedBranch === currentBranch;

  // Force in-place when the current branch is picked — a new worktree for it
  // would error. Keyed on the selection so switching to another branch frees the
  // choice again.
  useEffect(() => {
    if (currentSelected && place === "worktree") setPlace("here");
  }, [currentSelected, place]);

  // Re-scan local branches on open, since the dialog stays mounted across
  // opens and react-query's normal refetch-on-mount never fires again.
  useEffect(() => {
    if (open && !folder) void bases.refetch();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, folder]);

  // Pre-select the new-branch base: the detected default, else the first
  // branch. Follows a late-arriving default until the user picks one
  // themselves. Keyed on `open` so reopening re-defaults.
  useEffect(() => {
    if (!open || userPickedBase) return;
    const initial = defaultBase || branches[0];
    if (initial && initial !== base) setBase(initial);
  }, [open, userPickedBase, defaultBase, branches, base]);

  const reset = () => {
    setTitle("");
    setQuery("");
    setSelectedBranch(null);
    setBase("");
    setUserPickedBase(false);
    setPlace("worktree");
    create.reset();
  };

  const close = () => {
    if (create.isPending) return;
    reset();
    onClose();
  };

  // Resolve the combobox state into the backend's (mode, base, branch) shape.
  const resolved = (): { mode: WorktreeMode; base: string | null; branch: string | null } => {
    if (folder) return { mode: "folder", base: null, branch: null };
    if (isNew) return { mode: "isolated", base: base || null, branch: null };
    if (place === "here") return { mode: "inplace", base: null, branch: selectedBranch };
    return { mode: "existing", base: null, branch: selectedBranch };
  };

  const submit = () => {
    const trimmed = title.trim();
    if (!trimmed || create.isPending) return;
    create.mutate(
      { title: trimmed, ...resolved() },
      {
        onSuccess: (view) => {
          // Creation succeeded — close and jump straight to the new workstream.
          // Seed-copy problems (e.g. a missing .env) surface at the project level
          // (the sidebar warning), not here, so the dialog always dismisses.
          reset();
          onClose();
          onCreated(view);
        },
      },
    );
  };

  // A detected default that isn't in the branch list (rare) still needs an
  // option so the base <select> can show it as selected.
  const baseOptions =
    defaultBase && !branches.includes(defaultBase) ? [defaultBase, ...branches] : branches;

  const inplace = !isNew && place === "here";
  const busyLabel = folder
    ? "Creating…"
    : inplace
      ? "Switching branch…"
      : "Provisioning worktree…";
  const hint = folder
    ? "Runs in the project folder. No branch, no worktree — add as many as you like."
    : isNew
      ? "Creates a new branch named from your title, in a dedicated worktree."
      : inplace
        ? currentSelected
          ? `Attaches a workstream to ${selectedBranch} in your current checkout. No worktree is created.`
          : `Switches your checkout to ${selectedBranch}. No worktree is created.`
        : `Checks ${selectedBranch} out into a dedicated worktree.`;

  return (
    <Dialog open={open} onClose={close} title="New workstream">
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <TextInput
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="What are you working on?"
          autoFocus
          disabled={create.isPending}
        />

        {!folder && (
        <Field label="Branch">
          <div className="overflow-hidden rounded-md border border-edge">
            <TextInput
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search branches…"
              disabled={create.isPending || bases.isLoading}
              className="!rounded-none !border-0 !border-b !border-edge/60"
            />
            <ul className="max-h-48 overflow-y-auto py-1">
              {/* Pinned: start a fresh branch. Always present, even while filtering. */}
              <BranchRow
                icon={<PlusIcon size={14} />}
                label="New branch"
                detail="named from your title"
                selected={isNew}
                disabled={create.isPending}
                onSelect={() => setSelectedBranch(null)}
              />
              {bases.isLoading && (
                <li className="px-3 py-2 text-sm text-ink-dim">Loading branches…</li>
              )}
              {!bases.isLoading && filtered.length === 0 && query.trim() !== "" && (
                <li className="px-3 py-2 text-sm text-ink-dim">No branches match “{query}”.</li>
              )}
              {filtered.map((b) => (
                <BranchRow
                  key={b}
                  icon={<SplitIcon size={14} />}
                  label={b}
                  detail={b === currentBranch ? "active" : undefined}
                  selected={selectedBranch === b}
                  disabled={create.isPending}
                  onSelect={() => setSelectedBranch(b)}
                />
              ))}
            </ul>
          </div>
        </Field>
        )}

        {folder ? null : isNew ? (
          <Field label="Branched from">
            <DropdownSelect
              aria-label="Branched from"
              value={base}
              onChange={(v) => {
                setBase(v);
                setUserPickedBase(true);
              }}
              disabled={create.isPending || bases.isLoading}
              options={
                bases.isLoading
                  ? [{ value: "", label: "Loading branches…" }]
                  : baseOptions.map((b) => ({ value: b, label: b }))
              }
            />
          </Field>
        ) : (
          <Field label="Where">
            <Segmented
              value={place}
              onChange={setPlace}
              disabled={create.isPending}
              options={[
                {
                  value: "worktree",
                  label: "New worktree",
                  disabled: currentSelected,
                  title: currentSelected
                    ? `${selectedBranch} is already checked out here — use My checkout.`
                    : undefined,
                },
                { value: "here", label: "My checkout" },
              ]}
            />
          </Field>
        )}

        <p className="text-xs leading-relaxed text-ink-dim">{hint}</p>
        {!folder && bases.isError && <ErrorNote error={bases.error} />}
        {create.isError && <ErrorNote error={create.error} />}
        <div className="flex justify-end gap-2">
          <Button type="button" onClick={close} disabled={create.isPending}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" busy={create.isPending} disabled={!title.trim()}>
            {create.isPending ? busyLabel : "Create"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/** One row in the branch combobox: an icon, a label, and an optional dimmed
 * detail/badge on the right. The selected row is highlighted. */
function BranchRow({
  icon,
  label,
  detail,
  selected,
  disabled,
  onSelect,
}: {
  icon: ReactNode;
  label: string;
  detail?: string;
  selected: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onSelect}
        disabled={disabled}
        aria-pressed={selected}
        className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm disabled:cursor-not-allowed disabled:opacity-50 ${
          selected ? "bg-accent/15 text-ink ring-1 ring-inset ring-accent/30" : "hover:bg-edge/40"
        }`}
      >
        <span className="shrink-0 text-ink-dim">{icon}</span>
        <span className="min-w-0 flex-1 truncate text-ink">{label}</span>
        {detail && <span className="shrink-0 text-xs text-ink-dim">{detail}</span>}
      </button>
    </li>
  );
}

/** A compact two-or-more option segmented control — the lightweight stand-in
 * for a dropdown when the choices are few and worth showing at a glance. */
function Segmented<T extends string>({
  value,
  onChange,
  options,
  disabled,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string; disabled?: boolean; title?: string }[];
  disabled?: boolean;
}) {
  return (
    <div className="inline-flex rounded-md border border-edge p-0.5">
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            type="button"
            onClick={() => onChange(opt.value)}
            disabled={disabled || opt.disabled}
            title={opt.title}
            aria-pressed={active}
            className={`rounded px-3 py-1 text-sm disabled:cursor-not-allowed disabled:opacity-50 ${
              active ? "bg-accent/20 text-ink ring-1 ring-inset ring-accent/30" : "text-ink-dim hover:text-ink"
            }`}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
