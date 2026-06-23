import { open } from "@tauri-apps/plugin-dialog";
import { useEffect } from "react";

import type { GithubRepo } from "../../lib/bindings";
import { Button, Field, Spinner, TextInput } from "../../components/ui";
import { RepoAvatar } from "../../components/RepoAvatar";
import { useRecentRepos } from "../../lib/queries";

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
};

const MODE_LABELS: Record<RepoSource["mode"], string> = {
  clone: "Clone from URL",
  disk: "Open from disk",
  create: "Create new",
};

/**
 * Shared repo-source picker: clone URL + recents, disk browse, or create new.
 *
 * Used in AddProjectDialog (clone + disk) and OnboardingWizard (clone + disk + create).
 * Extracted from AddProjectDialog so both surfaces share the same picker logic
 * without duplication.
 */
export function RepoSourcePicker({
  value,
  onChange,
  modes = ["clone", "disk"],
  labels,
  defaultLocation = "",
  disabled = false,
  showRecents = true,
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
        <div className="flex gap-1 rounded-lg border border-edge bg-panel-2 p-0.5 text-sm">
          {modes.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              disabled={disabled}
              className={`flex-1 rounded-md px-3 py-1.5 font-medium transition-colors ${
                value.mode === m ? "bg-edge text-ink" : "text-ink-dim hover:text-ink"
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

          {showRecents && <RecentRepos query={recents} onPick={(r) => onChange({ ...value, url: r.url })} />}

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
          <p className="mt-1 text-xs leading-relaxed text-ink-dim">
            Referenced in place — hyprlayer reads this directory directly.
          </p>
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
