import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { ArchivedWorkstreamView, WorkstreamView } from "@/lib/bindings";
import { Button, Dialog, DropdownSelect, ErrorNote, Field, TextInput } from "@/components/ui";
import { ArchiveIcon, PlusIcon, SplitIcon, WarningTriangleIcon } from "@/components/icons";
import type { WorktreeMode } from "@/lib/bindings";
import { formatRelativeTime } from "@/lib/format";
import { isAppError } from "@/lib/ipc";
import { basename } from "@/lib/path";
import {
  useArchivedWorkstreams,
  useCreateWorkstream,
  useDiscardProjectChanges,
  useFetchProjectRemote,
  useRestoreWorkstream,
  useWorkstreamBases,
} from "@/lib/queries";

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

/** The remote-only rows carry their full ref (`origin/feature`); the local branch
 * they'd create is the ref minus its remote prefix. */
function localNameFor(remoteRef: string): string {
  const slash = remoteRef.indexOf("/");
  return slash === -1 ? remoteRef : remoteRef.slice(slash + 1);
}

/**
 * New-workstream dialog, built around one branch combobox. There is no mode
 * dropdown: the branch you pick determines the shape.
 *
 * The pinned "New branch" row creates a fresh branch and worktree from a base.
 * Picking an existing branch instead reveals a Worktree/Here choice, where
 * Worktree checks it out into its own worktree and Here switches the project's
 * checkout with no worktree.
 *
 * The list holds local branches plus remote-only ones. Picking a remote row
 * creates the local tracking branch first, since checking out a remote ref
 * would detach HEAD. Opening the dialog fires a `git fetch`, so a just-pushed
 * branch appears without a trip to the terminal.
 *
 * A folder project has none of this: the dialog collapses to a title and the
 * workstream runs in the project folder.
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
  // A folder project has no branches to list, don't even ask.
  const bases = useWorkstreamBases(projectPath, open && !folder);
  const refreshRemote = useFetchProjectRemote(projectPath);

  const defaultBase = bases.data?.defaultBase ?? "";
  const currentBranch = bases.data?.currentBranch ?? "";
  // Memoized so identities are stable across renders, otherwise the `?? []`
  // literals would re-fire the default-selection effect every render.
  const branches = useMemo(() => bases.data?.branches ?? [], [bases.data?.branches]);
  const localBranches = useMemo(
    () => bases.data?.localBranches ?? [],
    [bases.data?.localBranches],
  );
  // Remote-only branches, already filtered backend-side to those with no local
  // counterpart, listed after the locals so the branches you have come first.
  const remoteBranches = useMemo(
    () => bases.data?.remoteBranches ?? [],
    [bases.data?.remoteBranches],
  );
  const pickable = useMemo(
    () => [...localBranches, ...remoteBranches],
    [localBranches, remoteBranches],
  );
  const remoteSet = useMemo(() => new Set(remoteBranches), [remoteBranches]);
  const filtered = useMemo(() => filterBranches(pickable, query), [pickable, query]);

  // An in-place create refused to switch branches because the project checkout
  // has uncommitted work. Recoverable in the dialog, so it doesn't render as a
  // bare error (see CheckoutDirtyNotice).
  const checkoutDirty = isAppError(create.error) && create.error.code === "checkout_dirty";
  const dirtyChanges = checkoutDirty && isAppError(create.error)
    ? dirtyCount(create.error.message)
    : null;
  const archived = useArchivedWorkstreams(checkoutDirty);
  const discard = useDiscardProjectChanges(projectPath);
  const restore = useRestoreWorkstream();
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  // Whose changes those are: the most recently archived in-place row still
  // recording the branch the checkout is on. Branch adoption is what keeps that
  // match honest, the row follows `git checkout` right up until it's archived.
  const leftBehind = useMemo(
    () =>
      checkoutDirty && currentBranch
        ? archived.data?.find(
            (v) =>
              v.item.projectPath === projectPath &&
              v.item.worktreeMode === "inplace" &&
              v.item.branch === currentBranch,
          )
        : undefined,
    [checkoutDirty, currentBranch, archived.data, projectPath],
  );

  const isNew = selectedBranch === null;
  const remoteSelected = !isNew && !!selectedBranch && remoteSet.has(selectedBranch);
  // The current branch is already checked out in the project's own tree, so it
  // can't also be checked out into a dedicated worktree, git refuses a branch
  // already checked out elsewhere. Only "My checkout" (in-place) is valid for it.
  const currentSelected = !isNew && !!currentBranch && selectedBranch === currentBranch;

  // Force in-place when the current branch is picked, a new worktree for it
  // would error. Keyed on the selection so switching to another branch frees the
  // choice again.
  useEffect(() => {
    if (currentSelected && place === "worktree") setPlace("here");
  }, [currentSelected, place]);

  // Re-scan branches on open, since the dialog stays mounted across opens and
  // react-query's normal refetch-on-mount never fires again. The fetch runs
  // alongside: the list paints immediately from the refs already on disk, and any
  // branch pushed since the last fetch drops in when the network call lands.
  useEffect(() => {
    if (!open || folder) return;
    void bases.refetch();
    refreshRemote.mutate();
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
    setConfirmDiscard(false);
    create.reset();
    discard.reset();
    restore.reset();
  };

  const close = () => {
    if (create.isPending) return;
    reset();
    onClose();
  };

  /** Recovery is in flight, the whole dialog holds still until it settles. */
  const recovering = discard.isPending || restore.isPending;

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
          // Creation succeeded, close and jump straight to the new workstream.
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
  // A remote-only pick becomes a local branch tracking it, so say what will be
  // created rather than naming a branch the user doesn't have yet.
  const localFor = remoteSelected && selectedBranch ? localNameFor(selectedBranch) : selectedBranch;
  const hint = folder
    ? "Runs in the project folder. No branch, no worktree. Add as many as you like."
    : isNew
      ? "Creates a new branch named from your title, in a dedicated worktree."
      : remoteSelected
        ? inplace
          ? `Creates local branch ${localFor} tracking ${selectedBranch} and switches your checkout to it. No worktree is created.`
          : `Creates local branch ${localFor} tracking ${selectedBranch}, in a dedicated worktree.`
        : inplace
          ? currentSelected
            ? `Attaches a workstream to ${selectedBranch} in your current checkout. No worktree is created.`
            : `Switches your checkout to ${selectedBranch}. No worktree is created.`
          : `Checks ${selectedBranch} out into a dedicated worktree.`;

  return (
    <>
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
                  detail={
                    remoteSet.has(b) ? "remote" : b === currentBranch ? "active" : undefined
                  }
                  selected={selectedBranch === b}
                  disabled={create.isPending}
                  onSelect={() => setSelectedBranch(b)}
                />
              ))}
              {refreshRemote.isPending && (
                <li className="px-3 py-2 text-xs text-ink-dim">Fetching remote branches…</li>
              )}
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
                    ? `${selectedBranch} is already checked out here. Use My checkout.`
                    : undefined,
                },
                { value: "here", label: "My checkout" },
              ]}
            />
          </Field>
        )}

        <p className="text-xs leading-relaxed text-ink-dim">{hint}</p>
        {!folder && bases.isError && <ErrorNote error={bases.error} />}
        {create.isError &&
          (checkoutDirty ? (
            <CheckoutDirtyNotice
              count={dirtyChanges}
              onBranch={currentBranch}
              target={selectedBranch}
              leftBehind={leftBehind}
              busy={recovering}
              onUnarchive={(id) =>
                restore.mutate(id, {
                  onSuccess: (view) => {
                    reset();
                    onClose();
                    onCreated(view);
                  },
                })
              }
            />
          ) : (
            <ErrorNote error={create.error} />
          ))}
        {discard.isError && <ErrorNote error={discard.error} />}
        {restore.isError && <ErrorNote error={restore.error} />}
        <div className="flex justify-end gap-2">
          {checkoutDirty && (
            <Button
              type="button"
              variant="danger"
              className="mr-auto"
              disabled={create.isPending || recovering}
              onClick={() => setConfirmDiscard(true)}
            >
              Discard {changeCount(dirtyChanges)}
            </Button>
          )}
          <Button type="button" onClick={close} disabled={create.isPending || recovering}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            busy={create.isPending}
            disabled={!title.trim() || recovering}
          >
            {create.isPending ? busyLabel : "Create"}
          </Button>
        </div>
      </form>
    </Dialog>

    {/* Sibling, not nested: the panel scrolls and owns a backdrop filter, and
        `useEscape` stacks so this one takes Escape while it's up. */}
    <Dialog
      open={confirmDiscard}
      onClose={() => setConfirmDiscard(false)}
      title="Discard changes?"
    >
      <p className="text-sm leading-relaxed text-ink-dim">
        Discard {changeCount(dirtyChanges)} in{" "}
        <span className="font-mono text-ink">{basename(projectPath)}</span>? Modified files go
        back to the last commit; untracked files are left alone. This cannot be undone.
      </p>
      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={() => setConfirmDiscard(false)}>Cancel</Button>
        <Button
          variant="danger"
          busy={discard.isPending}
          onClick={() => {
            setConfirmDiscard(false);
            discard.mutate(undefined, { onSuccess: submit });
          }}
        >
          Discard
        </Button>
      </div>
    </Dialog>
    </>
  );
}

/** Best-effort count from the backend's `checkout_dirty` message. Cosmetic,
 * the copy degrades to "the changes" if the wording ever drifts. */
function dirtyCount(message: string): number | null {
  const m = /(\d+)\s+uncommitted/.exec(message);
  return m ? Number(m[1]) : null;
}

function changeCount(count: number | null): string {
  return count == null ? "the changes" : `${count} change${count === 1 ? "" : "s"}`;
}

/**
 * Why an in-place create stopped: the project checkout has uncommitted work that
 * a branch switch would carry across. Not an error note, nothing failed that a
 * retry would fix, so it reads as a blocked state with the way out attached. The
 * work is usually an archived in-place workstream's leftovers, and unarchiving is
 * the only route back to it; discarding lives in the dialog footer beside Create.
 */
function CheckoutDirtyNotice({
  count,
  onBranch,
  target,
  leftBehind,
  busy,
  onUnarchive,
}: {
  count: number | null;
  /** Branch the checkout is sitting on. */
  onBranch: string;
  /** Branch the create wanted to switch to. */
  target: string | null;
  /** The archived workstream the changes belong to, when one is identifiable. */
  leftBehind: ArchivedWorkstreamView | undefined;
  busy: boolean;
  onUnarchive: (id: string) => void;
}) {
  return (
    <div className="overflow-hidden rounded-md border border-edge bg-panel-2 text-sm">
      <div className="flex gap-3 p-3">
        <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-attention/30 bg-attention/15 text-attention">
          <WarningTriangleIcon size={14} />
        </span>
        <div className="min-w-0">
          <p className="font-medium text-ink">Your checkout has uncommitted work</p>
          <p className="mt-0.5 truncate font-mono text-xs text-ink-dim">
            {changeCount(count)} on {onBranch || "this branch"}
          </p>
          <p className="mt-2 leading-relaxed text-ink-dim">
            Switching it to <span className="font-mono text-ink">{target}</span> would carry that
            work across, so nothing was created.
          </p>
        </div>
      </div>
      {leftBehind && (
        <div className="flex items-center gap-3 border-t border-edge px-3 py-2.5">
          <ArchiveIcon size={15} className="shrink-0 text-ink-dim" />
          <div className="min-w-0 flex-1">
            <div className="truncate text-ink">{leftBehind.item.title}</div>
            <div className="truncate text-xs text-ink-dim">
              left it here · archived {formatRelativeTime(leftBehind.item.archivedAt)}
            </div>
          </div>
          <Button
            type="button"
            className="shrink-0"
            disabled={busy}
            onClick={() => onUnarchive(leftBehind.item.id)}
          >
            Unarchive
          </Button>
        </div>
      )}
    </div>
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

/** A compact two-or-more option segmented control, the lightweight stand-in
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
