import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { ArchivedWorkstreamView, WorkstreamView } from "@/lib/bindings";
import { Button, Dialog, DropdownSelect, ErrorNote, Field, TextInput } from "@/components/ui";
import { Icon } from "@/components/Icon";
import type { WorktreeMode } from "@/lib/bindings";
import { previewSlug, suggestPrefix } from "@/lib/domain/branchPrefix";
import { runsInProjectDir } from "@/lib/domain/projects";
import { formatRelativeTime } from "@/lib/format";
import { isAppError } from "@/lib/ipc";
import { basename } from "@/lib/path";
import {
  useArchivedWorkstreams,
  useBranchPrefixes,
  useCreateWorkstream,
  useDiscardProjectChanges,
  useFetchProjectRemote,
  useRestoreWorkstream,
  useWorkstreamBases,
  useWorkstreams,
} from "@/lib/queries";

type Props = {
  projectPath: string;
  open: boolean;
  onClose: () => void;
  onCreated: (view: WorkstreamView) => void;
  folder?: boolean;
};

type Place = "worktree" | "here";

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

function localNameFor(remoteRef: string): string {
  const slash = remoteRef.indexOf("/");
  return slash === -1 ? remoteRef : remoteRef.slice(slash + 1);
}

export function CreateWorkstreamDialog({
  projectPath,
  open,
  onClose,
  onCreated,
  folder = false,
}: Props) {
  const [title, setTitle] = useState("");
  const [query, setQuery] = useState("");
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null);
  const [base, setBase] = useState("");
  const [userPickedBase, setUserPickedBase] = useState(false);
  const [typePrefix, setTypePrefix] = useState<string | null>(null);
  const [userPickedType, setUserPickedType] = useState(false);
  const [place, setPlace] = useState<Place>("worktree");
  const create = useCreateWorkstream(projectPath);
  const bases = useWorkstreamBases(projectPath, open && !folder);
  const prefixes = useBranchPrefixes(projectPath, open && !folder);
  const refreshRemote = useFetchProjectRemote(projectPath);

  const defaultBase = bases.data?.defaultBase ?? "";
  const currentBranch = bases.data?.currentBranch ?? "";
  const branches = useMemo(() => bases.data?.branches ?? [], [bases.data?.branches]);
  const localBranches = useMemo(
    () => bases.data?.localBranches ?? [],
    [bases.data?.localBranches],
  );
  const remoteBranches = useMemo(
    () => bases.data?.remoteBranches ?? [],
    [bases.data?.remoteBranches],
  );
  const pickable = useMemo(
    () => [...localBranches, ...remoteBranches],
    [localBranches, remoteBranches],
  );
  const remoteSet = useMemo(() => new Set(remoteBranches), [remoteBranches]);
  const worktreeFor = useMemo(
    () => new Map((bases.data?.worktrees ?? []).map((w) => [w.branch, w.path])),
    [bases.data?.worktrees],
  );
  const active = useWorkstreams(projectPath);
  const filtered = useMemo(() => filterBranches(pickable, query), [pickable, query]);

  const typeList = useMemo(() => {
    const seen = new Set<string>();
    return (prefixes.data?.prefixes ?? []).filter((p) => {
      const prefix = p.prefix.trim();
      if (!prefix || seen.has(prefix)) return false;
      seen.add(prefix);
      return true;
    });
  }, [prefixes.data?.prefixes]);
  const defaultPrefix = prefixes.data?.defaultPrefix ?? "";

  const checkoutDirty = isAppError(create.error) && create.error.code === "checkout_dirty";
  const dirtyChanges = checkoutDirty && isAppError(create.error)
    ? dirtyCount(create.error.message)
    : null;
  const archived = useArchivedWorkstreams(checkoutDirty);
  const discard = useDiscardProjectChanges(projectPath);
  const restore = useRestoreWorkstream();
  const [confirmDiscard, setConfirmDiscard] = useState(false);
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

  const ownerOf = (branch: string, path: string | undefined) =>
    path === undefined
      ? undefined
      : active.data?.find(
          (v) =>
            v.item.worktreePath === path ||
            (v.item.branch === branch && !runsInProjectDir(v.item)),
        );

  const isNew = selectedBranch === null;
  const remoteSelected = !isNew && !!selectedBranch && remoteSet.has(selectedBranch);
  const currentSelected = !isNew && !!currentBranch && selectedBranch === currentBranch;
  const worktreePath = !isNew && selectedBranch ? worktreeFor.get(selectedBranch) : undefined;
  const owner = selectedBranch ? ownerOf(selectedBranch, worktreePath) : undefined;

  useEffect(() => {
    if (currentSelected && place === "worktree") setPlace("here");
  }, [currentSelected, place]);

  useEffect(() => {
    if (!open || folder) return;
    void bases.refetch();
    refreshRemote.mutate();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, folder]);

  useEffect(() => {
    if (!open || userPickedBase) return;
    const initial = defaultBase || branches[0];
    if (initial && initial !== base) setBase(initial);
  }, [open, userPickedBase, defaultBase, branches, base]);

  useEffect(() => {
    if (!open || userPickedType || !isNew) return;
    const next = suggestPrefix(title, typeList)?.prefix ?? null;
    if (next !== typePrefix) setTypePrefix(next);
  }, [open, userPickedType, isNew, title, typeList, typePrefix]);

  const reset = () => {
    setTitle("");
    setQuery("");
    setSelectedBranch(null);
    setBase("");
    setUserPickedBase(false);
    setTypePrefix(null);
    setUserPickedType(false);
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

  const recovering = discard.isPending || restore.isPending;

  const resolved = (): {
    mode: WorktreeMode;
    base: string | null;
    branch: string | null;
    branchPrefix: string | null;
  } => {
    if (folder) return { mode: "folder", base: null, branch: null, branchPrefix: null };
    if (isNew)
      return { mode: "isolated", base: base || null, branch: null, branchPrefix: typePrefix };
    if (worktreePath)
      return { mode: "attached", base: null, branch: selectedBranch, branchPrefix: null };
    if (place === "here")
      return { mode: "inplace", base: null, branch: selectedBranch, branchPrefix: null };
    return { mode: "existing", base: null, branch: selectedBranch, branchPrefix: null };
  };

  const openOwner = () => {
    if (!owner) return;
    reset();
    onClose();
    onCreated(owner);
  };

  const submit = () => {
    if (owner) return openOwner();
    const trimmed = title.trim();
    if (!trimmed || create.isPending) return;
    create.mutate(
      { title: trimmed, ...resolved() },
      {
        onSuccess: (view) => {
          reset();
          onClose();
          onCreated(view);
        },
      },
    );
  };

  const baseOptions =
    defaultBase && !branches.includes(defaultBase) ? [defaultBase, ...branches] : branches;

  const inplace = !isNew && place === "here";
  const busyLabel = folder
    ? "Creating…"
    : worktreePath
      ? "Opening worktree…"
      : inplace
        ? "Switching branch…"
        : "Provisioning worktree…";
  const localFor = remoteSelected && selectedBranch ? localNameFor(selectedBranch) : selectedBranch;
  const hint = folder
    ? "Runs in the project folder. No branch, no worktree. Add as many as you like."
    : isNew
      ? "Creates a new branch named from your title, in a dedicated worktree."
      : owner
        ? "This branch already has a workstream. Nothing new is created."
      : worktreePath
        ? "Opens this worktree as it is. Nothing is created or switched, and archiving the workstream leaves the folder in place."
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
              <BranchRow
                icon={<Icon name="add" size="sm" />}
                label="New branch"
                detail="named from your title"
                selected={isNew}
                disabled={create.isPending}
                onSelect={() => setSelectedBranch(null)}
              />
              {bases.isLoading && (
                <li className="px-3 py-2 text-body text-ink-dim">Loading branches…</li>
              )}
              {!bases.isLoading && filtered.length === 0 && query.trim() !== "" && (
                <li className="px-3 py-2 text-body text-ink-dim">No branches match “{query}”.</li>
              )}
              {filtered.map((b) => (
                <BranchRow
                  key={b}
                  icon={<Icon name="branch" size="sm" />}
                  label={b}
                  detail={
                    remoteSet.has(b)
                      ? "remote"
                      : b === currentBranch
                        ? "active"
                        : worktreeFor.has(b)
                          ? ownerOf(b, worktreeFor.get(b))
                            ? "workstream"
                            : "worktree"
                          : undefined
                  }
                  selected={selectedBranch === b}
                  disabled={create.isPending}
                  onSelect={() => setSelectedBranch(b)}
                />
              ))}
              {refreshRemote.isPending && (
                <li className="px-3 py-2 text-ui text-ink-dim">Fetching remote branches…</li>
              )}
            </ul>
          </div>
        </Field>
        )}

        {!folder && isNew && (
          <Field label="Type of work">
            <DropdownSelect
              aria-label="Type of work"
              value={typePrefix ?? ""}
              onChange={(v) => {
                setTypePrefix(v || null);
                setUserPickedType(true);
              }}
              disabled={create.isPending || prefixes.isLoading}
              options={[
                { value: "", label: "General" },
                ...typeList.map((p) => ({ value: p.prefix, label: p.label })),
              ]}
            />
            <p className="mt-1.5 text-ui text-ink-dim">
              Preview:{" "}
              <span className="font-mono text-ink">
                {typePrefix ?? defaultPrefix}
                {previewSlug(title)}-<span className="opacity-60">xxxx</span>
              </span>
            </p>
          </Field>
        )}

        {folder ? null : worktreePath ? (
          owner ? (
            <WorktreeCard title="Already a workstream here" detail={owner.item.title} />
          ) : (
            <Field label="Where">
              <WorktreeCard title="Existing worktree" detail={worktreePath} />
            </Field>
          )
        ) : isNew ? (
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

        <p className="text-ui leading-prose text-ink-dim">{hint}</p>
        {!folder && bases.isError && <ErrorNote error={bases.error} />}
        {!folder && isNew && prefixes.isError && <ErrorNote error={prefixes.error} />}
        {create.isError &&
          (checkoutDirty ? (
            <CheckoutDirtyNotice
              count={dirtyChanges}
              onBranch={currentBranch}
              target={selectedBranch}
              leftBehind={leftBehind}
              busy={recovering}
              onUnarchive={(id) =>
                restore.mutate({ id }, {
                  onSuccess: (view) => {
                    reset();
                    onClose();
                    onCreated(view);
                  },
                })
              }
            />
          ) : (
            <ErrorNote error={create.error} onDismiss={() => create.reset()} />
          ))}
        {discard.isError && <ErrorNote error={discard.error} onDismiss={() => discard.reset()} />}
        {restore.isError && <ErrorNote error={restore.error} onDismiss={() => restore.reset()} />}
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
          {owner ? (
            <Button type="button" variant="primary" onClick={openOwner}>
              Go to workstream
            </Button>
          ) : (
            <Button
              type="submit"
              variant="primary"
              busy={create.isPending}
              disabled={!title.trim() || recovering || (isNew && !folder && prefixes.isLoading)}
            >
              {create.isPending ? busyLabel : "Create"}
            </Button>
          )}
        </div>
      </form>
    </Dialog>

    <Dialog
      open={confirmDiscard}
      onClose={() => setConfirmDiscard(false)}
      title="Discard changes?"
    >
      <p className="text-body leading-prose text-ink-dim">
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

function dirtyCount(message: string): number | null {
  const m = /(\d+)\s+uncommitted/.exec(message);
  return m ? Number(m[1]) : null;
}

function changeCount(count: number | null): string {
  return count == null ? "the changes" : `${count} change${count === 1 ? "" : "s"}`;
}

function CheckoutDirtyNotice({
  count,
  onBranch,
  target,
  leftBehind,
  busy,
  onUnarchive,
}: {
  count: number | null;
  onBranch: string;
  target: string | null;
  leftBehind: ArchivedWorkstreamView | undefined;
  busy: boolean;
  onUnarchive: (id: string) => void;
}) {
  return (
    <div className="overflow-hidden rounded-md border border-edge bg-panel-2 text-body">
      <div className="flex gap-3 p-3">
        <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-attention/30 bg-attention/15 text-attention">
          <Icon name="warning" size="sm" />
        </span>
        <div className="min-w-0">
          <p className="font-medium text-ink">Your checkout has uncommitted work</p>
          <p className="mt-0.5 truncate text-meta text-ink-dim">
            {changeCount(count)} on {onBranch || "this branch"}
          </p>
          <p className="mt-2 leading-prose text-ink-dim">
            Switching it to <span className="font-mono text-ink">{target}</span> would carry that
            work across, so nothing was created.
          </p>
        </div>
      </div>
      {leftBehind && (
        <div className="flex items-center gap-3 border-t border-edge px-3 py-2.5">
          <Icon name="archive" className="text-ink-dim" />
          <div className="min-w-0 flex-1">
            <div className="truncate text-ink">{leftBehind.item.title}</div>
            <div className="truncate text-ui text-ink-dim">
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

function WorktreeCard({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="flex items-start gap-2.5 rounded-md border border-edge bg-panel-2 px-3 py-2.5">
      <Icon name="folder" size="sm" className="mt-0.5 text-ink-dim" />
      <div className="min-w-0">
        <div className="text-body font-medium text-ink">{title}</div>
        {/* rtl moves the ellipsis to the start so the folder name stays visible */}
        <div dir="rtl" className="mt-0.5 truncate text-left font-mono text-ui text-ink-dim" title={detail}>
          <bdi dir="ltr">{detail}</bdi>
        </div>
      </div>
    </div>
  );
}

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
        className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-body disabled:cursor-not-allowed disabled:opacity-50 ${
          selected ? "bg-accent/15 text-ink ring-1 ring-inset ring-accent/30" : "hover:bg-edge/40"
        }`}
      >
        <span className="shrink-0 text-ink-dim">{icon}</span>
        <span className="min-w-0 flex-1 truncate text-ink">{label}</span>
        {detail && <span className="shrink-0 text-ui text-ink-dim">{detail}</span>}
      </button>
    </li>
  );
}

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
            className={`rounded px-3 py-1 text-body disabled:cursor-not-allowed disabled:opacity-50 ${
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
