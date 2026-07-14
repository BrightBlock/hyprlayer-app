import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, type ReactNode } from "react";

import type { GithubRepo } from "@/lib/bindings";
import { Button, Field, RefreshButton, Spinner, TextInput } from "@/components/ui";
import { RepoAvatar } from "@/components/RepoAvatar";
import { useRecentRepos } from "@/lib/queries";

export type RepoSource =
  | { mode: "clone"; url: string; location: string }
  | { mode: "disk"; path: string }
  | { mode: "create"; path: string };

type Props = {
  value: RepoSource;
  onChange: (next: RepoSource) => void;
  /** Which modes to show. Defaults to ["clone", "disk"]. */
  modes?: Array<RepoSource["mode"]>;
  /** Override the button label for each mode. */
  labels?: Partial<Record<RepoSource["mode"], string>>;
  /** Default value for the location/path field when mode switches. */
  defaultLocation?: string;
  disabled?: boolean;
  /** Whether to show the GitHub recent repos list (clone mode only). */
  showRecents?: boolean;
  /** Extra content rendered under the disk path field — e.g. a hint that the
   *  folder isn't a git repo. Disk mode only. */
  diskFooter?: ReactNode;
};

const MODE_LABELS: Record<RepoSource["mode"], string> = {
  clone: "Clone from URL",
  disk: "Open from disk",
  create: "Create new",
};

/**
 * Shared repo-source picker: clone URL + recents, disk browse, or create new.
 * Used by AddProjectDialog (clone + disk) and OnboardingWizard (+ create).
 */
export function RepoSourcePicker({
  value,
  onChange,
  modes = ["clone", "disk"],
  labels,
  defaultLocation = "",
  disabled = false,
  showRecents = true,
  diskFooter,
}: Props) {
  const recents = useRecentRepos(showRecents && value.mode === "clone");

  // Seed the location/path field when the component first mounts or modes change.
  useEffect(() => {
    if (!defaultLocation) return;
    if (value.mode === "clone" && !(value as { mode: "clone"; url: string; location: string }).location) {
      onChange({ mode: "clone", url: "", location: defaultLocation });
    }
    if (value.mode === "create" && !(value as { mode: "create"; path: string }).path) {
      onChange({ mode: "create", path: defaultLocation });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const browse = async (onPick: (p: string) => void, title: string) => {
    const picked = await open({ directory: true, multiple: false, title });
    if (typeof picked === "string") onPick(picked);
  };

  const setMode = (mode: RepoSource["mode"]) => {
    if (mode === "clone") {
      const cur = value.mode === "clone" ? value : null;
      onChange({ mode: "clone", url: cur?.url ?? "", location: cur?.location ?? defaultLocation });
    } else if (mode === "disk") {
      const cur = value.mode === "disk" ? value : null;
      onChange({ mode: "disk", path: cur?.path ?? "" });
    } else {
      const cur = value.mode === "create" ? value : null;
      onChange({ mode: "create", path: cur?.path ?? defaultLocation });
    }
  };

  return (
    <div className="space-y-3">
      {/* Mode toggle — only shown when there are ≥2 modes */}
      {modes.length > 1 && (
        <div className="flex gap-1 rounded-lg border border-edge bg-panel p-1 text-sm">
          {modes.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              disabled={disabled}
              className={`flex-1 rounded-md border px-3 py-2 font-medium transition-colors ${
                value.mode === m
                  ? "border-edge bg-panel-2 text-ink"
                  : "border-transparent text-ink-dim hover:text-ink"
              }`}
            >
              {labels?.[m] ?? MODE_LABELS[m]}
            </button>
          ))}
        </div>
      )}

      {value.mode === "clone" && (
        <>
          <Field label="Repository URL">
            <TextInput
              value={value.url}
              onChange={(e) => onChange({ ...value, url: e.target.value })}
              placeholder="https://github.com/user/thoughts.git"
              autoFocus
              disabled={disabled}
            />
          </Field>

          {showRecents && (
            <RecentRepos
              query={recents}
              selectedUrl={value.url}
              onPick={(r) => onChange({ ...value, url: r.url })}
            />
          )}

          <Field label="Location">
            <div className="flex gap-2">
              <TextInput
                value={value.location}
                onChange={(e) => onChange({ ...value, location: e.target.value })}
                placeholder="~/thoughts"
                disabled={disabled}
                className="font-mono text-xs"
              />
              <Button
                type="button"
                onClick={() => void browse((p) => onChange({ ...value, location: p }), "Choose where to clone")}
                disabled={disabled}
              >
                Browse
              </Button>
            </div>
          </Field>
        </>
      )}

      {value.mode === "disk" && (
        <Field label="Local repository">
          <div className="flex gap-2">
            <TextInput
              value={value.path}
              onChange={(e) => onChange({ ...value, path: e.target.value })}
              placeholder="/path/to/your/repo"
              autoFocus
              disabled={disabled}
              className="font-mono text-xs"
            />
            <Button
              type="button"
              onClick={() => void browse((p) => onChange({ ...value, path: p }), "Select a git repository")}
              disabled={disabled}
            >
              Browse
            </Button>
          </div>
          {diskFooter}
        </Field>
      )}

      {value.mode === "create" && (
        <Field label="New repository location">
          <div className="flex gap-2">
            <TextInput
              value={value.path}
              onChange={(e) => onChange({ ...value, path: e.target.value })}
              placeholder="~/thoughts"
              autoFocus
              disabled={disabled}
              className="font-mono text-xs"
            />
            <Button
              type="button"
              onClick={() => void browse((p) => onChange({ ...value, path: p }), "Choose where to create the repo")}
              disabled={disabled}
            >
              Browse
            </Button>
          </div>
          <p className="mt-1 text-xs leading-relaxed text-ink-dim">
            A new git repository will be initialised at this path.
          </p>
        </Field>
      )}
    </div>
  );
}

/** GitHub recent repos picker (clone mode only). */
function RecentRepos({
  query,
  selectedUrl,
  onPick,
}: {
  query: ReturnType<typeof useRecentRepos>;
  /** The current URL-field value, so the picked row stays highlighted. */
  selectedUrl: string;
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
  return (
    <div>
      <div className="mb-1 flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-ink-dim">Recent repos</p>
        <RefreshButton
          onClick={() => void query.refetch()}
          busy={query.isFetching}
          label="Refresh recent repos"
        />
      </div>
      {repos.length === 0 ? (
        // Keep the refresh above reachable: signing in with gh is the fix, and
        // it happens in a terminal while this dialog stays open.
        <p className="rounded-md border border-edge bg-panel-2 px-3 py-2 text-xs text-ink-dim">
          None found. Sign in with <span className="font-mono">gh auth login</span>, then refresh.
        </p>
      ) : (
        <div className="max-h-44 divide-y divide-edge/60 overflow-y-auto rounded-md border border-edge">
          {repos.map((repo) => (
            <button
              key={repo.nameWithOwner}
              type="button"
              onClick={() => onPick(repo)}
              className={`flex w-full items-center gap-3 px-3 py-2 text-left transition-colors ${
                selectedUrl.trim() === repo.url ? "bg-panel-2" : "hover:bg-panel-2/60"
              }`}
            >
              <RepoAvatar source={repo.nameWithOwner} name={repo.nameWithOwner} size={24} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm text-ink">{repo.nameWithOwner}</span>
                {repo.description && (
                  <span className="block truncate text-xs text-ink-dim">{repo.description}</span>
                )}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
