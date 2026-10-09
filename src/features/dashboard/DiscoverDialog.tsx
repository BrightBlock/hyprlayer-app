import { useEffect, useMemo, useState, type ReactNode } from "react";

import type { GithubRepo } from "@/lib/bindings";
import { Dialog, IconButton, RefreshButton, Spinner, TextInput } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { RepoAvatar } from "@/components/RepoAvatar";
import { formatCompactAge, formatRelativeTime } from "@/lib/format";
import { githubRepoRef } from "@/lib/github";
import { useBootstrap, useRecentRepos } from "@/lib/queries";

type Props = {
  open: boolean;
  onClose: () => void;
  onPick: (repo: GithubRepo) => void;
};

const ALL_OWNERS = null;

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
  sample: string;
};

export function DiscoverDialog({ open, onClose, onPick }: Props) {
  const bootstrap = useBootstrap();
  const recents = useRecentRepos(open);

  const [filter, setFilter] = useState("");
  const [owner, setOwner] = useState<string | null>(ALL_OWNERS);

  const all = useMemo(() => recents.data ?? [], [recents.data]);

  const cloned = useMemo(() => {
    const set = new Set<string>();
    for (const project of bootstrap.data?.projects ?? []) {
      const ref = githubRepoRef(project.originUrl);
      if (ref) set.add(`${ref.owner}/${ref.name}`.toLowerCase());
    }
    return set;
  }, [bootstrap.data]);

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

  const ownerExists = owner !== ALL_OWNERS && owners.some((o) => o.name === owner);
  const activeOwner = ownerExists ? owner : ALL_OWNERS;
  useEffect(() => {
    if (!ownerExists && owner !== ALL_OWNERS) setOwner(ALL_OWNERS);
  }, [ownerExists, owner]);

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
    setFilter("");
    setOwner(ALL_OWNERS);
    onClose();
  };

  const pick = (repo: GithubRepo) => {
    onPick(repo);
    close();
  };

  return (
    <Dialog
      open={open}
      onClose={close}
      ariaLabel="Discover repositories"
      padded={false}
      scroll={false}
      size="4xl"
      className="h-[600px]"
    >
      <div className="flex h-full bg-panel">
        <div className="flex w-52 flex-none flex-col border-r border-edge bg-surface px-3 py-4">
          <div className="px-2.5 pb-3.5 pt-1 text-body font-semibold text-ink">
            Discover repositories
          </div>
          <div className="px-2.5 pb-1.5 text-label text-ink-dim">
            Organizations
          </div>
          <div className="flex flex-col gap-0.5 overflow-y-auto">
            <OrgRow
              active={activeOwner === ALL_OWNERS}
              count={all.length}
              onClick={() => setOwner(ALL_OWNERS)}
            >
              <Icon name="globe" />
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
            className="mt-auto px-2.5 py-1.5 text-ui-sm"
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
            <div className="group/search relative flex-1">
              <Icon
                name="search"
                size="sm"
                className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-dim group-focus-within/search:text-ink"
              />
              <TextInput
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Filter repositories…"
                autoFocus
                aria-label="Filter repositories"
                className="pl-8"
              />
            </div>
            <IconButton onClick={close} aria-label="Close" className="flex-none">
              <Icon name="close" size="sm" />
            </IconButton>
          </div>

          <div className="flex flex-1 flex-col gap-0.5 overflow-y-auto px-3 py-2">
            {recents.isLoading ? (
              <div className="flex items-center gap-2 px-2.5 py-6 text-body text-ink-dim">
                <Spinner size="sm" /> Loading your GitHub repositories…
              </div>
            ) : all.length === 0 ? (
              <p className="px-2.5 py-6 text-center text-body text-ink-dim">
                No repositories found. Sign in with <span className="font-mono">gh auth login</span>{" "}
                to browse yours here, then refresh.
              </p>
            ) : repos.length === 0 ? (
              <p className="px-2.5 py-6 text-center text-body text-ink-dim">
                No repositories match “{filter}”.
              </p>
            ) : (
              repos.map((repo) => (
                <RepoRow
                  key={repo.nameWithOwner}
                  repo={repo}
                  cloned={isCloned(repo)}
                  onPick={() => pick(repo)}
                />
              ))
            )}
          </div>

          <div className="flex items-center justify-between border-t border-edge px-5 py-2.5 text-ui-sm text-ink-dim">
            <span>
              {scoped.length} {scoped.length === 1 ? "repository" : "repositories"}
            </span>
            {clonedCount > 0 && <span>{clonedCount} already cloned</span>}
          </div>
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
      className={`flex items-center gap-2 rounded-md px-2.5 py-1.5 text-body ${
        active ? "bg-panel-2 font-medium text-ink" : "text-ink-dim hover:bg-panel-2"
      }`}
    >
      {children}
      <span className="flex-none text-ui-sm text-ink-dim">{count}</span>
    </button>
  );
}

function RepoRow({
  repo,
  cloned,
  onPick,
}: {
  repo: GithubRepo;
  cloned: boolean;
  onPick: () => void;
}) {
  const [owner, name] = splitRef(repo.nameWithOwner);
  const age = formatCompactAge(repo.pushedAt ? Date.parse(repo.pushedAt) : null);

  return (
    <button
      type="button"
      onClick={onPick}
      disabled={cloned}
      className="group flex h-9 w-full flex-none items-center gap-3 rounded-md px-3 text-left enabled:hover:bg-edge/50 disabled:cursor-not-allowed disabled:opacity-60"
    >
      <RepoAvatar source={repo.nameWithOwner} name={owner} size={22} />
      <span className="flex min-w-0 flex-1 items-baseline gap-2 overflow-hidden whitespace-nowrap">
        <span className="flex-none text-body">
          <span className="text-ink-dim">{owner}/</span>
          <span className="font-semibold text-ink">{name}</span>
        </span>
        {repo.description && (
          <span className="truncate text-ui text-ink-dim">{repo.description}</span>
        )}
      </span>
      {repo.private && (
        <span className="flex-none text-ink-dim">
          <Icon name="lock" size="xs" />
          <span className="sr-only">Private</span>
        </span>
      )}
      {cloned ? (
        <span className="flex-none text-ui-sm text-ink-dim">Cloned</span>
      ) : (
        <>
          <span className="flex-none text-ui-sm text-ink-dim group-hover:hidden">{age}</span>
          <span className="hidden flex-none items-center gap-1.5 text-ui font-medium text-ink group-hover:flex">
            <Icon name="download" size="xs" />
            Clone…
          </span>
        </>
      )}
    </button>
  );
}
