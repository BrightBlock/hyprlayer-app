import { useMemo, useState } from "react";

import type { GithubRepo, Project } from "../../lib/bindings";
import { Button, Dialog, ErrorNote, Field, Spinner, TextInput } from "../../components/ui";
import { RepoAvatar } from "../../components/RepoAvatar";
import { useBootstrap, useCloneProject, useRecentRepos } from "../../lib/queries";

type Props = {
  open: boolean;
  onClose: () => void;
  /** The freshly cloned project, so the caller can navigate to it. */
  onAdded: (project: Project) => void;
};

/**
 * "Discover" — a browse-first counterpart to Add. Lists the user's GitHub repos
 * (via gh) as a filterable grid you can scan and clone with one click, plus a
 * paste-a-URL escape hatch. Cloning lands an app-owned copy under the repos
 * root, same as the Add dialog.
 */
export function DiscoverDialog({ open, onClose, onAdded }: Props) {
  const bootstrap = useBootstrap();
  const recents = useRecentRepos(open);
  const clone = useCloneProject();

  const [filter, setFilter] = useState("");
  const [url, setUrl] = useState("");
  const [user, setUser] = useState("");
  const [pendingTarget, setPendingTarget] = useState<string | null>(null);

  const reposRoot = bootstrap.data?.settings.reposRoot ?? null;
  // First run: no config yet, so the backend needs a username to bootstrap one
  // before the clone can be registered (same requirement as the Add dialog).
  const configMissing = bootstrap.data?.config.state === "missing";
  const bootstrapUser = configMissing ? user.trim() || null : null;
  const needsUser = configMissing && !bootstrapUser;

  const repos = useMemo(() => {
    const all = recents.data ?? [];
    const needle = filter.trim().toLowerCase();
    if (!needle) return all;
    return all.filter(
      (r) =>
        r.nameWithOwner.toLowerCase().includes(needle) ||
        (r.description ?? "").toLowerCase().includes(needle),
    );
  }, [recents.data, filter]);

  const close = () => {
    if (clone.isPending) return;
    setFilter("");
    setUrl("");
    setUser("");
    setPendingTarget(null);
    clone.reset();
    onClose();
  };

  const startClone = (cloneUrl: string, target: string) => {
    if (clone.isPending || needsUser) return;
    setPendingTarget(target);
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
    <Dialog open={open} onClose={close} title="Discover repositories" className="!max-w-5xl">
      <div className="space-y-3">
        {configMissing && (
          <Field label="Your name">
            <TextInput
              value={user}
              onChange={(e) => setUser(e.target.value)}
              placeholder="e.g. jane"
              disabled={clone.isPending}
            />
            <p className="mt-1 text-xs text-ink-dim">
              First-run setup: attributes your thought documents. Required before the first clone.
            </p>
          </Field>
        )}

        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            const trimmed = url.trim();
            if (trimmed) startClone(trimmed, trimmed);
          }}
        >
          <TextInput
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="Paste a repo URL — https://github.com/user/repo.git"
            disabled={clone.isPending}
            className="font-mono text-xs"
          />
          <Button
            type="submit"
            variant="primary"
            busy={clone.isPending && pendingTarget === url.trim()}
            disabled={!url.trim() || clone.isPending || needsUser}
          >
            Clone
          </Button>
        </form>

        <div className="flex items-center justify-between gap-2">
          <p className="text-xs font-medium uppercase tracking-wider text-ink-dim">Your repositories</p>
          {recents.data && recents.data.length > 6 && (
            <TextInput
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter…"
              className="h-7 w-40 py-0 text-xs"
            />
          )}
        </div>

        {recents.isLoading ? (
          <div className="flex items-center gap-2 py-6 text-sm text-ink-dim">
            <Spinner size={14} /> Loading your GitHub repositories…
          </div>
        ) : (recents.data?.length ?? 0) === 0 ? (
          <p className="rounded-md border border-edge bg-panel-2 px-3 py-6 text-center text-sm text-ink-dim">
            No repositories found. Sign in with <span className="font-mono">gh auth login</span> to
            browse yours here, or paste a URL above.
          </p>
        ) : (
          <div className="grid max-h-[60vh] grid-cols-1 gap-2 overflow-y-auto pr-1 sm:grid-cols-2">
            {repos.map((repo) => (
              <RepoCard
                key={repo.nameWithOwner}
                repo={repo}
                busy={clone.isPending && pendingTarget === repo.url}
                disabled={clone.isPending || needsUser}
                onClone={() => startClone(repo.url, repo.url)}
              />
            ))}
            {repos.length === 0 && (
              <p className="col-span-full py-4 text-center text-sm text-ink-dim">
                No repositories match “{filter}”.
              </p>
            )}
          </div>
        )}

        {clone.error != null && <ErrorNote error={clone.error} />}
      </div>
    </Dialog>
  );
}

function RepoCard({
  repo,
  busy,
  disabled,
  onClone,
}: {
  repo: GithubRepo;
  busy: boolean;
  disabled: boolean;
  onClone: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClone}
      disabled={disabled}
      className="group flex items-center gap-3 rounded-md border border-edge bg-panel-2 px-3 py-2.5 text-left transition-colors hover:border-brand/50 hover:bg-edge/40 disabled:cursor-not-allowed disabled:opacity-60"
    >
      <RepoAvatar source={repo.nameWithOwner} name={repo.nameWithOwner} size={26} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-medium text-ink">{repo.nameWithOwner}</span>
        {repo.description && (
          <span className="line-clamp-1 text-xs text-ink-dim">{repo.description}</span>
        )}
      </span>
      <span className="shrink-0 text-xs font-medium text-ink-dim group-hover:text-brand">
        {busy ? <Spinner size={14} /> : "Clone"}
      </span>
    </button>
  );
}
