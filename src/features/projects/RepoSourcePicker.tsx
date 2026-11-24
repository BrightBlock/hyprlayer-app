import { useEffect, type ReactNode } from "react";

import type { GithubRepo } from "@/lib/bindings";
import { cx, Field, RefreshButton, Spinner, TextInput } from "@/components/ui";
import { PathField } from "@/components/PathField";
import { RepoAvatar } from "@/components/RepoAvatar";
import { useRecentRepos } from "@/lib/queries";

export type RepoSource =
  | { mode: "clone"; url: string; location: string }
  | { mode: "disk"; path: string }
  | { mode: "create"; path: string };

type Props = {
  value: RepoSource;
  onChange: (next: RepoSource) => void;
  modes?: Array<RepoSource["mode"]>;
  labels?: Partial<Record<RepoSource["mode"], string>>;
  variant?: "tabs" | "cards";
  icons?: Partial<Record<RepoSource["mode"], ReactNode>>;
  descriptions?: Partial<Record<RepoSource["mode"], string>>;
  defaultLocation?: string;
  disabled?: boolean;
  showRecents?: boolean;
  diskFooter?: ReactNode;
  diskLabel?: string;
  diskBrowseTitle?: string;
  diskPlaceholder?: string;
  createSlot?: ReactNode;
};

const MODE_LABELS: Record<RepoSource["mode"], string> = {
  clone: "Clone from URL",
  disk: "Open from disk",
  create: "Create new",
};

export function RepoSourcePicker({
  value,
  onChange,
  modes = ["clone", "disk"],
  labels,
  variant = "tabs",
  icons,
  descriptions,
  defaultLocation = "",
  disabled = false,
  showRecents = true,
  diskFooter,
  diskLabel = "Local repository",
  diskBrowseTitle = "Select a git repository",
  diskPlaceholder = "/path/to/your/repo",
  createSlot,
}: Props) {
  const recents = useRecentRepos(showRecents && value.mode === "clone");

  useEffect(() => {
    if (!defaultLocation) return;
    if (value.mode === "clone" && !value.location) {
      onChange({ mode: "clone", url: "", location: defaultLocation });
    }
    if (value.mode === "create" && !value.path) {
      onChange({ mode: "create", path: defaultLocation });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
      {modes.length > 1 && variant === "cards" && (
        <div className="grid grid-cols-3 gap-2">
          {modes.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              disabled={disabled}
              aria-pressed={value.mode === m}
              className={cx(
                "flex flex-col gap-1.5 rounded-card border p-3 text-left disabled:cursor-not-allowed disabled:opacity-50",
                value.mode === m
                  ? "border-ink bg-panel ring-1 ring-ink"
                  : "border-edge bg-panel/90 hover:bg-panel",
              )}
            >
              <span className="flex items-center gap-2 text-body font-semibold text-ink">
                {icons?.[m]}
                {labels?.[m] ?? MODE_LABELS[m]}
              </span>
              {descriptions?.[m] && <span className="text-ui-sm text-ink-dim">{descriptions[m]}</span>}
            </button>
          ))}
        </div>
      )}
      {modes.length > 1 && variant === "tabs" && (
        <div className="flex gap-1 rounded-lg border border-edge bg-panel p-1 text-body">
          {modes.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => setMode(m)}
              disabled={disabled}
              className={`flex-1 rounded-md border px-3 py-2 font-medium ${
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
            <PathField
              value={value.location}
              onChange={(location) => onChange({ ...value, location })}
              placeholder="~/thoughts"
              disabled={disabled}
              browseTitle="Choose where to clone"
            />
          </Field>
        </>
      )}

      {value.mode === "disk" && (
        <Field label={diskLabel}>
          <PathField
            value={value.path}
            onChange={(path) => onChange({ ...value, path })}
            placeholder={diskPlaceholder}
            autoFocus
            disabled={disabled}
            browseTitle={diskBrowseTitle}
          />
          {diskFooter}
        </Field>
      )}

      {value.mode === "create" && createSlot}

      {value.mode === "create" && !createSlot && (
        <Field label="New repository location">
          <PathField
            value={value.path}
            onChange={(path) => onChange({ ...value, path })}
            placeholder="~/thoughts"
            autoFocus
            disabled={disabled}
            browseTitle="Choose where to create the repo"
          />
          <p className="mt-1 text-ui leading-prose text-ink-dim">
            A new git repository will be initialised at this path.
          </p>
        </Field>
      )}
    </div>
  );
}

function RecentRepos({
  query,
  selectedUrl,
  onPick,
}: {
  query: ReturnType<typeof useRecentRepos>;
  selectedUrl: string;
  onPick: (repo: GithubRepo) => void;
}) {
  if (query.isLoading) {
    return (
      <div className="flex items-center gap-2 px-1 py-1 text-ui text-ink-dim">
        <Spinner size="xs" /> Loading your repos…
      </div>
    );
  }
  const repos = query.data ?? [];
  return (
    <div>
      <div className="mb-1 flex items-center justify-between gap-2">
        <p className="text-ui font-medium text-ink-dim">Recent repos</p>
        <RefreshButton
          onClick={() => void query.refetch()}
          busy={query.isFetching}
          label="Refresh recent repos"
        />
      </div>
      {repos.length === 0 ? (
        <p className="rounded-md border border-edge bg-panel-2 px-3 py-2 text-ui text-ink-dim">
          None found. Sign in with <span className="font-mono">gh auth login</span>, then refresh.
        </p>
      ) : (
        <div className="max-h-44 divide-y divide-edge/60 overflow-y-auto rounded-md border border-edge">
          {repos.map((repo) => (
            <button
              key={repo.nameWithOwner}
              type="button"
              onClick={() => onPick(repo)}
              className={`flex w-full items-center gap-3 px-3 py-2 text-left ${
                selectedUrl.trim() === repo.url ? "bg-panel-2" : "hover:bg-panel-2/60"
              }`}
            >
              <RepoAvatar source={repo.nameWithOwner} name={repo.nameWithOwner} size={24} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-body text-ink">{repo.nameWithOwner}</span>
                {repo.description && (
                  <span className="block truncate text-ui text-ink-dim">{repo.description}</span>
                )}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
