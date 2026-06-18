import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { homeDir, join } from "@tauri-apps/api/path";

import type { GithubRepo, Project } from "../../lib/bindings";
import { Button, Dialog, ErrorNote, Field, Spinner, TextInput } from "../../components/ui";
import { RepoAvatar } from "../../components/RepoAvatar";
import {
  useAddProject,
  useBootstrap,
  useCloneProject,
  useRecentRepos,
} from "../../lib/queries";

type Mode = "clone" | "disk";

type Props = {
  open: boolean;
  onClose: () => void;
  /** The freshly added project, so the caller can navigate to it. */
  onAdded: (project: Project) => void;
};

/**
 * Add a project — either by cloning a hosted repo (a GitHub URL or one of the
 * user's recent repos) into the repos root, or by importing a local repo from
 * disk. A disk import is registered in place (never copied), so worktrees
 * branch off the user's existing repo rather than a duplicate.
 */
export function AddProjectDialog({ open: isOpen, onClose, onAdded }: Props) {
  const bootstrap = useBootstrap();
  // A true first run has no config yet; the CLI needs a username to bootstrap one.
  const configMissing = bootstrap.data?.config.state === "missing";

  const [mode, setMode] = useState<Mode>("clone");
  const [url, setUrl] = useState("");
  const [diskPath, setDiskPath] = useState("");
  const [location, setLocation] = useState("");
  const [user, setUser] = useState("");

  const clone = useCloneProject();
  const add = useAddProject();
  const recents = useRecentRepos(isOpen && mode === "clone");

  const pending = clone.isPending || add.isPending;
  const error = clone.error ?? add.error;

  // Default the clone location to the configured repos root, else ~/hyprlayer/repos.
  useEffect(() => {
    if (!isOpen) return;
    const configured = bootstrap.data?.settings.reposRoot;
    if (configured) {
      setLocation((cur) => cur || configured);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const def = await join(await homeDir(), "hyprlayer", "repos");
        if (!cancelled) setLocation((cur) => cur || def);
      } catch {
        // Leave blank → the backend falls back to its own default.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, bootstrap.data?.settings.reposRoot]);

  const reset = () => {
    setUrl("");
    setDiskPath("");
    setUser("");
    clone.reset();
    add.reset();
  };

  const close = () => {
    if (pending) return;
    reset();
    onClose();
  };

  const succeed = (project: Project) => {
    reset();
    onClose();
    onAdded(project);
  };

  const browse = async (set: (path: string) => void, title: string) => {
    const picked = await open({ directory: true, multiple: false, title });
    if (typeof picked === "string") set(picked);
  };

  const bootstrapUser = configMissing ? user.trim() || null : null;
  const primaryInput = (mode === "clone" ? url : diskPath).trim();
  const canSubmit = Boolean(primaryInput) && (!configMissing || Boolean(user.trim())) && !pending;

  const submit = () => {
    if (!canSubmit) return;
    if (mode === "clone") {
      clone.mutate(
        { url: primaryInput, location: location.trim() || null, name: null, bootstrapUser },
        { onSuccess: succeed },
      );
    } else {
      add.mutate(
        { path: primaryInput, directoryName: null, bootstrapUser },
        { onSuccess: succeed },
      );
    }
  };

  return (
    <Dialog open={isOpen} onClose={close} title="Add project" className="max-w-lg">
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <div className="flex gap-1 rounded-lg border border-edge bg-panel-2 p-0.5 text-sm">
          {(
            [
              ["clone", "Clone from URL"],
              ["disk", "Open from disk"],
            ] as [Mode, string][]
          ).map(([m, label]) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              disabled={pending}
              className={`flex-1 rounded-md px-3 py-1.5 font-medium transition-colors ${
                mode === m ? "bg-edge text-ink" : "text-ink-dim hover:text-ink"
              }`}
            >
              {label}
            </button>
          ))}
        </div>

        {mode === "clone" ? (
          <>
            <Field label="Repository URL">
              <TextInput
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://github.com/user/repo.git"
                autoFocus
                disabled={pending}
              />
            </Field>

            <RecentRepos query={recents} onPick={(r) => setUrl(r.url)} />

            <Field label="Location">
              <div className="flex gap-2">
                <TextInput
                  value={location}
                  onChange={(e) => setLocation(e.target.value)}
                  placeholder="~/hyprlayer/repos"
                  disabled={pending}
                  className="font-mono text-xs"
                />
                <Button
                  type="button"
                  onClick={() => void browse(setLocation, "Choose where to clone")}
                  disabled={pending}
                >
                  Browse
                </Button>
              </div>
            </Field>
          </>
        ) : (
          <Field label="Local repository">
            <div className="flex gap-2">
              <TextInput
                value={diskPath}
                onChange={(e) => setDiskPath(e.target.value)}
                placeholder="/path/to/your/repo"
                autoFocus
                disabled={pending}
                className="font-mono text-xs"
              />
              <Button
                type="button"
                onClick={() => void browse(setDiskPath, "Select a git repository")}
                disabled={pending}
              >
                Browse
              </Button>
            </div>
            <p className="mt-1 text-xs leading-relaxed text-ink-dim">
              Added in place — hyprlayer references this repo directly and never copies it. Worktrees
              branch off it.
            </p>
          </Field>
        )}

        {configMissing && (
          <Field label="Your name">
            <TextInput
              value={user}
              onChange={(e) => setUser(e.target.value)}
              placeholder="e.g. jane"
              disabled={pending}
            />
            <p className="mt-1 text-xs text-ink-dim">
              Attributes your thought documents (one-time, first-run setup).
            </p>
          </Field>
        )}

        {error != null && <ErrorNote error={error} />}

        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={close} disabled={pending}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" busy={pending} disabled={!canSubmit}>
            {pending
              ? mode === "clone"
                ? "Cloning…"
                : "Importing…"
              : mode === "clone"
                ? "Clone repo"
                : "Add project"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/** The "Recent repos" picker — the user's GitHub repos via gh. Hidden entirely
 *  when gh is missing/unauthenticated (the URL field still works). */
function RecentRepos({
  query,
  onPick,
}: {
  query: ReturnType<typeof useRecentRepos>;
  onPick: (repo: GithubRepo) => void;
}) {
  if (query.isLoading) {
    return (
      <div className="flex items-center gap-2 px-1 py-1 text-xs text-ink-dim">
        <Spinner size={12} /> Loading your repos…
      </div>
    );
  }
  const repos = query.data ?? [];
  if (repos.length === 0) return null;
  return (
    <div>
      <p className="mb-1 text-xs font-medium text-ink-dim">Recent repos</p>
      <div className="max-h-44 overflow-y-auto rounded-md border border-edge bg-panel-2">
        {repos.map((repo) => (
          <button
            key={repo.nameWithOwner}
            type="button"
            onClick={() => onPick(repo)}
            className="flex w-full items-center gap-2.5 border-b border-edge/50 px-3 py-2 text-left last:border-0 hover:bg-edge/40"
          >
            <RepoAvatar source={repo.nameWithOwner} name={repo.nameWithOwner} size={22} />
            <span className="flex min-w-0 flex-col">
              <span className="truncate text-sm text-ink">{repo.nameWithOwner}</span>
              {repo.description && (
                <span className="line-clamp-1 text-xs text-ink-dim">{repo.description}</span>
              )}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
