import { useEffect, useRef, useState } from "react";

import {
  useBackendHealth,
  useProjectDetail,
  useSetWorktreeFiles,
  useSyncThoughts,
  useWorktreeFiles,
} from "@/lib/queries";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  Dialog,
  ErrorNote,
  PageError,
  PageSpinner,
  Spinner,
  TextInput,
} from "@/components/ui";
import type { CheckStatus, HealthCheck } from "@/lib/bindings";
import { isGitProject } from "@/lib/domain/projects";
import { ScriptsCard } from "./ScriptsCard";

const CHECK_TONE: Record<CheckStatus, "ok" | "warn" | "bad"> = {
  pass: "ok",
  warn: "warn",
  fail: "bad",
};

function HealthRow({ check }: { check: HealthCheck }) {
  return (
    <div className="flex items-start gap-3 py-1.5">
      <Badge tone={CHECK_TONE[check.status]}>{check.status}</Badge>
      <div className="min-w-0">
        <p className="text-sm">{check.label}</p>
        {check.detail && (
          <p className="selectable truncate font-mono text-xs text-ink-dim" title={check.detail}>
            {check.detail}
          </p>
        )}
        {check.remedy && check.status !== "pass" && (
          <p className="mt-0.5 text-xs text-warn">{check.remedy}</p>
        )}
      </div>
    </div>
  );
}

/**
 * Editor for the "Files copied into new worktrees" list. Whole-list replace on
 * every add/remove, matching the `patchClaude`/`visibleModels` pattern.
 */
function WorktreeFilesCard({ projectPath }: { projectPath: string }) {
  const [newPattern, setNewPattern] = useState("");
  const files = useWorktreeFiles(projectPath);
  const setFiles = useSetWorktreeFiles(projectPath);
  const listRef = useRef<HTMLUListElement>(null);
  const prevLen = useRef(0);

  const list: string[] = files.data ?? [];

  // After an add (list grows by one), pin the newest pattern into view so it
  // doesn't land silently below the fold of the fixed-height scroll area.
  useEffect(() => {
    if (list.length === prevLen.current + 1) {
      listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
    }
    prevLen.current = list.length;
  }, [list.length]);

  function add() {
    const p = newPattern.trim();
    if (!p || list.includes(p)) return;
    setFiles.mutate([...list, p]);
    setNewPattern("");
  }

  function remove(p: string) {
    setFiles.mutate(list.filter((x) => x !== p));
  }

  return (
    <Card className="shrink-0">
      <CardTitle>Files copied into new worktrees</CardTitle>
      <p className="mb-3 text-xs leading-relaxed text-ink-dim">
        Copied from the main repo into every new worktree.{" "}
        <code className="font-mono">git worktree add</code> only carries tracked files, so list
        untracked ones (e.g. <code className="font-mono">.env</code>) here. Glob patterns
        supported. Saved to{" "}
        <code className="font-mono">.hyprlayer/settings.toml</code> (safe to commit).
      </p>
      {files.isLoading && <Spinner />}
      {files.isError && <ErrorNote error={files.error} />}
      {setFiles.isError && <ErrorNote error={setFiles.error} onDismiss={() => setFiles.reset()} />}
      {list.length > 0 && (
        <ul ref={listRef} className="mb-3 max-h-40 space-y-1 overflow-y-auto pr-1">
          {list.map((p) => (
            <li key={p} className="flex items-center gap-2">
              <span className="selectable flex-1 font-mono text-sm text-ink">{p}</span>
              <button
                type="button"
                className="shrink-0 rounded px-1 py-0.5 text-xs text-ink-dim hover:bg-edge/60 hover:text-bad"
                onClick={() => remove(p)}
                disabled={setFiles.isPending}
                aria-label={`Remove ${p}`}
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-2">
        <TextInput
          value={newPattern}
          onChange={(e) => setNewPattern(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") add();
          }}
          placeholder=".env, certs/, config/*.local.json"
          disabled={setFiles.isPending}
        />
        <Button onClick={add} disabled={!newPattern.trim() || setFiles.isPending}>
          Add
        </Button>
      </div>
    </Card>
  );
}

type Props = {
  /** Project repo path; the dialog is open whenever this is non-null. */
  projectPath: string;
  onClose: () => void;
};

/**
 * Per-project settings as a dialog opened from the sidebar (right-click a
 * project). Holds the storage summary, backend health checks, and the manual
 * thoughts sync, the actions that used to live on the standalone project
 * settings route, now reachable where the projects themselves live.
 */
export function ProjectSettingsDialog({ projectPath, onClose }: Props) {
  const detail = useProjectDetail(projectPath);
  const health = useBackendHealth(projectPath);
  const sync = useSyncThoughts(projectPath);

  const title = detail.data ? `Project settings: ${detail.data.project.mappedName}` : "Project settings";
  // A folder project has no thoughts to sync and no worktrees to seed.
  const gitBacked = isGitProject(detail.data?.project);

  return (
    <Dialog open onClose={onClose} title={title} className="!max-w-3xl">
      {detail.isLoading ? (
        <PageSpinner />
      ) : detail.isError ? (
        <PageError error={detail.error} />
      ) : (
        <div className="flex max-h-[80vh] flex-col space-y-3 overflow-y-auto pr-1">
          <div className="flex shrink-0 items-center justify-between gap-4">
            <p className="selectable min-w-0 truncate font-mono text-xs text-ink-dim">
              {detail.data!.project.path}
            </p>
            {gitBacked && (
              <Button
                variant="primary"
                onClick={() => sync.mutate(null)}
                busy={sync.isPending}
                title="hyprlayer thoughts sync"
              >
                Sync thoughts
              </Button>
            )}
          </div>

          {sync.isError && <ErrorNote error={sync.error} onDismiss={() => sync.reset()} />}
          {sync.isSuccess && (
            <Card className="shrink-0">
              <CardTitle>Sync output</CardTitle>
              <pre className="selectable max-h-40 overflow-auto whitespace-pre-wrap font-mono text-xs text-ink-dim">
                {sync.data.output.trim() || "Already up to date."}
              </pre>
            </Card>
          )}

          <Card className="shrink-0">
            <CardTitle>Storage</CardTitle>
            <div className="space-y-1 text-sm">
              <p>
                <Badge tone={detail.data!.storage ? "accent" : "dim"}>
                  {detail.data!.storage?.backend ?? "local folder"}
                </Badge>
                {detail.data!.project.profile && detail.data!.storage && (
                  <span className="ml-2 text-ink-dim">profile {detail.data!.project.profile}</span>
                )}
              </p>
              {detail.data!.thoughtsRoot ? (
                <p className="text-ink-dim">
                  Thoughts root:{" "}
                  <span className="selectable font-mono text-xs text-ink">
                    {detail.data!.thoughtsRoot}
                  </span>
                </p>
              ) : (
                !gitBacked && (
                  <p className="text-ink-dim">
                    Not a git repository, so no branches, worktrees or thoughts. Agents run in the
                    folder itself.
                  </p>
                )
              )}
            </div>
          </Card>

          {gitBacked && <WorktreeFilesCard projectPath={projectPath} />}

          <ScriptsCard projectPath={projectPath} />

          <Card className="shrink-0">
            <div className="flex shrink-0 items-center justify-between">
              <CardTitle className="mb-0">Backend health</CardTitle>
              <Button onClick={() => void health.refetch()} busy={health.isFetching}>
                Run checks
              </Button>
            </div>
            <div className="mt-2 divide-y divide-edge/50 pr-1">
              {health.isLoading && <Spinner />}
              {health.data?.checks.map((c) => <HealthRow key={c.id} check={c} />)}
              {health.isError && <ErrorNote error={health.error} />}
            </div>
          </Card>
        </div>
      )}
    </Dialog>
  );
}
