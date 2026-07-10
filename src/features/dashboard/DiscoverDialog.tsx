import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { GithubRepo, Project } from "@/lib/bindings";
import {
  Dialog,
  ErrorNote,
  Field,
  IconButton,
  RefreshButton,
  Spinner,
  TextInput,
} from "@/components/ui";
import { DownloadIcon, GlobeIcon, LockIcon, SearchIcon, XIcon } from "@/components/icons";
import { RepoAvatar } from "@/components/RepoAvatar";
import { formatCompactAge, formatRelativeTime } from "@/lib/format";
import { githubRepoRef } from "@/lib/github";
import { useBootstrap, useCloneProject, useRecentRepos } from "@/lib/queries";

type Props = {
  open: boolean;
  onClose: () => void;
  /** The freshly cloned project, so the caller can navigate to it. */
  onAdded: (project: Project) => void;
};

/** "Don't filter by owner". Null, not "", so a malformed owner can't collide. */
const ALL_OWNERS = null;

/** Split a `owner/name` GitHub identifier; a name with no owner keeps its slot. */
function splitRef(nameWithOwner: string): [owner: string, name: string] {
  const slash = nameWithOwner.indexOf("/");
  if (slash === -1) return ["", nameWithOwner];
  return [nameWithOwner.slice(0, slash), nameWithOwner.slice(slash + 1)];
}

function ownerOf(repo: GithubRepo): string {
  return splitRef(repo.nameWithOwner)[0];
}

type Owner = {
  name: string;
  count: number;
  /** A `owner/repo` from this owner — RepoAvatar resolves the org icon from it. */
  sample: string;
};

/**
 * "Discover" — a browse-first counterpart to Add. Organizations are the modal's
 * structure (a left rail), repos a single dense column: name is the only bright
 * thing, owner/description/age step back, and Clone surfaces on row hover.
 * Cloning lands an app-owned copy under the repos root, same as the Add dialog —
 * which is also where you go to paste a URL or open a repo from disk.
 */
export function DiscoverDialog({ open, onClose, onAdded }: Props) {
  const bootstrap = useBootstrap();
  const recents = useRecentRepos(open);
  const clone = useCloneProject();

  const [filter, setFilter] = useState("");
  const [owner, setOwner] = useState<string | null>(ALL_OWNERS);
  const [user, setUser] = useState("");
  const [pendingTarget, setPendingTarget] = useState<string | null>(null);

  const reposRoot = bootstrap.data?.settings.reposRoot ?? null;
  // First run: no config yet, so the backend needs a username to bootstrap one
  // before the clone can be registered (same requirement as the Add dialog).
  const configMissing = bootstrap.data?.config.state === "missing";
  const bootstrapUser = configMissing ? user.trim() || null : null;
  const needsUser = configMissing && !bootstrapUser;

  const all = useMemo(() => recents.data ?? [], [recents.data]);

  const cloned = useMemo(() => {
    const set = new Set<string>();
    for (const project of bootstrap.data?.projects ?? []) {
      const ref = githubRepoRef(project.originUrl);
      if (ref) set.add(`${ref.owner}/${ref.name}`.toLowerCase());
    }
    return set;
  }, [bootstrap.data]);

  // `/user/repos` spans the personal account and every org the user belongs to,
  // so the owner segment is the only grouping the payload gives us.
  const owners = useMemo(() => {
    const byName = new Map<string, Owner>();
    for (const repo of all) {
      const name = ownerOf(repo);
      const seen = byName.get(name);
      if (seen) seen.count += 1;
      else byName.set(name, { name, count: 1, sample: repo.nameWithOwner });
    }
    return [...byName.values()].sort((a, b) => b.count - a.count);
  }, [all]);

  // A refresh can retire the selected owner (left the org, repo deleted). Fall
  // back to "all" rather than stranding the grid behind an owner that's gone.
  const ownerExists = owner !== ALL_OWNERS && owners.some((o) => o.name === owner);
  const activeOwner = ownerExists ? owner : ALL_OWNERS;
  // Commit the fallback, so a later refetch that resurfaces the owner doesn't
  // silently yank the scope back to it.
  useEffect(() => {
    if (!ownerExists && owner !== ALL_OWNERS) setOwner(ALL_OWNERS);
  }, [ownerExists, owner]);

  // The rail scopes; the toolbar searches within that scope. Both footer counts
  // and the search placeholder describe the scope, not the search result.
  const scoped = useMemo(
    () => (activeOwner === ALL_OWNERS ? all : all.filter((r) => ownerOf(r) === activeOwner)),
    [all, activeOwner],
  );

  const repos = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return scoped;
    return scoped.filter(
      (r) =>
        r.nameWithOwner.toLowerCase().includes(needle) ||
        (r.description ?? "").toLowerCase().includes(needle),
    );
  }, [scoped, filter]);

  const isCloned = (repo: GithubRepo) => cloned.has(repo.nameWithOwner.toLowerCase());
  const clonedCount = useMemo(
    () => scoped.filter((r) => cloned.has(r.nameWithOwner.toLowerCase())).length,
    [scoped, cloned],
  );

  const close = () => {
    if (clone.isPending) return;
    setFilter("");
    setOwner(ALL_OWNERS);
    setUser("");
    setPendingTarget(null);
    clone.reset();
    onClose();
  };

  const startClone = (cloneUrl: string) => {
    if (clone.isPending || needsUser) return;
    setPendingTarget(cloneUrl);
    clone.mutate(
      { url: cloneUrl, location: reposRoot, name: null, bootstrapUser },
      {
        onSuccess: (project) => {
          close();
          onAdded(project);
        },
        onSettled: () => setPendingTarget(null),
      },
    );
  };

  return (
    <Dialog
      open={open}
      onClose={close}
      ariaLabel="Discover repositories"
      padded={false}
      scroll={false}
      className="!max-w-[940px] h-[600px]"
    >
      <div className="flex h-full bg-panel">
        <div className="flex w-52 flex-none flex-col border-r border-edge bg-surface px-3 py-4">
          <div className="px-2.5 pb-3.5 pt-1 text-[13px] font-semibold tracking-[0.02em] text-ink">
            Discover repositories
          </div>
          <div className="px-2.5 pb-1.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-ink-dim">
            Organizations
          </div>
          <div className="flex flex-col gap-0.5 overflow-y-auto">
            <OrgRow
              active={activeOwner === ALL_OWNERS}
              count={all.length}
              onClick={() => setOwner(ALL_OWNERS)}
            >
              <GlobeIcon size={16} />
              <span className="flex-1 truncate text-left">All</span>
            </OrgRow>
            {owners.map((org) => (
              <OrgRow
                key={org.name}
                active={activeOwner === org.name}
                count={org.count}
                onClick={() => setOwner(org.name)}
              >
                <RepoAvatar source={org.sample} name={org.name} size={16} />
                <span className="flex-1 truncate text-left">{org.name}</span>
              </OrgRow>
            ))}
          </div>

          <RefreshButton
            onClick={() => void recents.refetch()}
            busy={recents.isFetching}
            label="Refresh repositories"
            className="mt-auto px-2.5 py-1.5 text-[11px]"
          >
            {recents.isFetching
              ? "Syncing…"
              : recents.dataUpdatedAt
                ? `Synced ${formatRelativeTime(recents.dataUpdatedAt)}`
                : "Never synced"}
          </RefreshButton>
        </div>

        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center gap-2 border-b border-edge px-5 py-4">
            <div className="relative flex-1">
              <SearchIcon
                size={14}
                className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-dim"
              />
              <TextInput
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder={`Filter ${scoped.length} ${scoped.length === 1 ? "repository" : "repositories"}…`}
                autoFocus={!configMissing}
                aria-label="Filter repositories"
                className="pl-8"
              />
            </div>
            <IconButton onClick={close} aria-label="Close" className="flex-none">
              <XIcon size={14} />
            </IconButton>
          </div>

          {configMissing && (
            <div className="border-b border-edge px-5 py-4">
              <Field label="Your name">
                <TextInput
                  value={user}
                  onChange={(e) => setUser(e.target.value)}
                  placeholder="e.g. jane"
                  autoFocus
                  disabled={clone.isPending}
                />
                <p className="mt-1 text-xs text-ink-dim">
                  First-run setup: attributes your thought documents. Required before the first
                  clone.
                </p>
              </Field>
            </div>
          )}

          <div className="flex flex-1 flex-col gap-0.5 overflow-y-auto px-3 py-2">
            {recents.isLoading ? (
              <div className="flex items-center gap-2 px-2.5 py-6 text-sm text-ink-dim">
                <Spinner size={14} /> Loading your GitHub repositories…
              </div>
            ) : all.length === 0 ? (
              <p className="px-2.5 py-6 text-center text-sm text-ink-dim">
                No repositories found. Sign in with <span className="font-mono">gh auth login</span>{" "}
                to browse yours here, then refresh.
              </p>
            ) : repos.length === 0 ? (
              <p className="px-2.5 py-6 text-center text-sm text-ink-dim">
                No repositories match “{filter}”.
              </p>
            ) : (
              repos.map((repo) => (
                <RepoRow
                  key={repo.nameWithOwner}
                  repo={repo}
                  busy={clone.isPending && pendingTarget === repo.url}
                  disabled={clone.isPending || needsUser}
                  cloned={isCloned(repo)}
                  onClone={() => startClone(repo.url)}
                />
              ))
            )}
          </div>

          <div className="flex items-center justify-between border-t border-edge px-5 py-2.5 text-[11px] text-ink-dim">
            <span>
              {scoped.length} {scoped.length === 1 ? "repository" : "repositories"}
            </span>
            {clonedCount > 0 && <span>{clonedCount} already cloned</span>}
          </div>

          {clone.error != null && (
            <div className="border-t border-edge px-5 py-4">
              <ErrorNote error={clone.error} />
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
}

function OrgRow({
  active,
  count,
  onClick,
  children,
}: {
  active: boolean;
  count: number;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={`flex items-center gap-2 rounded-md px-2.5 py-1.5 text-[13px] ${
        active ? "bg-panel-2 font-medium text-ink" : "text-ink-dim hover:bg-panel-2"
      }`}
    >
      {children}
      <span className="flex-none text-[11px] text-ink-dim">{count}</span>
    </button>
  );
}

/**
 * One dense repo row. The age gives way to a Clone affordance on hover — but
 * only when the row can actually be cloned, so a dead row never offers one.
 */
function RepoRow({
  repo,
  busy,
  disabled,
  cloned,
  onClone,
}: {
  repo: GithubRepo;
  busy: boolean;
  disabled: boolean;
  /** Already a project. Cloning again would fail on the existing directory. */
  cloned: boolean;
  onClone: () => void;
}) {
  const [owner, name] = splitRef(repo.nameWithOwner);
  const age = formatCompactAge(repo.pushedAt ? Date.parse(repo.pushedAt) : null);
  const inert = disabled || cloned;

  return (
    <button
      type="button"
      onClick={onClone}
      disabled={inert}
      className="group flex h-10 w-full flex-none items-center gap-3 rounded-md px-3 text-left enabled:hover:bg-panel-2 disabled:cursor-not-allowed disabled:opacity-60"
    >
      <RepoAvatar source={repo.nameWithOwner} name={owner} size={22} />
      <span className="flex min-w-0 flex-1 items-baseline gap-2 overflow-hidden whitespace-nowrap">
        <span className="flex-none text-[13px]">
          <span className="text-ink-dim">{owner}/</span>
          <span className="font-semibold text-ink">{name}</span>
        </span>
        {repo.description && (
          <span className="truncate text-xs text-ink-dim">{repo.description}</span>
        )}
      </span>
      {repo.private && (
        <span className="flex-none text-ink-dim">
          <LockIcon size={12} />
          <span className="sr-only">Private</span>
        </span>
      )}
      {busy ? (
        <span className="flex flex-none items-center gap-1.5 text-xs text-accent">
          <Spinner size={13} /> Cloning…
        </span>
      ) : cloned ? (
        <span className="flex-none text-[11px] text-ink-dim">Cloned</span>
      ) : inert ? (
        <span className="flex-none text-[11px] text-ink-dim">{age}</span>
      ) : (
        <>
          <span className="flex-none text-[11px] text-ink-dim group-hover:hidden">{age}</span>
          <span className="hidden flex-none items-center gap-1.5 rounded-md bg-accent px-2.5 py-1 text-xs font-semibold text-accent-ink group-hover:flex">
            <DownloadIcon size={12} strokeWidth={2} />
            Clone
          </span>
        </>
      )}
    </button>
  );
}
