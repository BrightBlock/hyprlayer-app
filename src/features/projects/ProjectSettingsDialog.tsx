import { useBackendHealth, useProjectDetail, useSyncThoughts } from "../../lib/queries";
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
} from "../../components/ui";
import type { CheckStatus, HealthCheck } from "../../lib/bindings";

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

type Props = {
  /** Project repo path; the dialog is open whenever this is non-null. */
  projectPath: string;
  onClose: () => void;
};

/**
 * Per-project settings as a dialog opened from the sidebar (right-click a
 * project). Holds the storage summary, backend health checks, and the manual
 * thoughts sync — the actions that used to live on the standalone project
 * settings route, now reachable where the projects themselves live.
 */
export function ProjectSettingsDialog({ projectPath, onClose }: Props) {
  const detail = useProjectDetail(projectPath);
  const health = useBackendHealth(projectPath);
  const sync = useSyncThoughts(projectPath);

  const title = detail.data ? `Project settings — ${detail.data.project.mappedName}` : "Project settings";

  return (
    <Dialog open onClose={onClose} title={title} className="!max-w-3xl">
      {detail.isLoading ? (
        <PageSpinner />
      ) : detail.isError ? (
        <PageError error={detail.error} />
      ) : (
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-4">
            <p className="selectable min-w-0 truncate font-mono text-xs text-ink-dim">
              {detail.data!.project.path}
            </p>
            <Button
              variant="primary"
              onClick={() => sync.mutate(null)}
              busy={sync.isPending}
              title="hyprlayer thoughts sync"
            >
              Sync thoughts
            </Button>
          </div>

          {sync.isError && <ErrorNote error={sync.error} />}
          {sync.isSuccess && (
            <Card>
              <CardTitle>Sync output</CardTitle>
              <pre className="selectable max-h-40 overflow-auto whitespace-pre-wrap font-mono text-xs text-ink-dim">
                {sync.data.output.trim() || "Already up to date."}
              </pre>
            </Card>
          )}

          <Card>
            <CardTitle>Storage</CardTitle>
            <div className="space-y-1 text-sm">
              <p>
                <Badge tone="accent">{detail.data!.storage.backend}</Badge>
                {detail.data!.project.profile && (
                  <span className="ml-2 text-ink-dim">profile {detail.data!.project.profile}</span>
                )}
              </p>
              {detail.data!.thoughtsRoot && (
                <p className="text-ink-dim">
                  Thoughts root:{" "}
                  <span className="selectable font-mono text-xs text-ink">
                    {detail.data!.thoughtsRoot}
                  </span>
                </p>
              )}
            </div>
          </Card>

          <Card>
            <div className="flex items-center justify-between">
              <CardTitle className="mb-0">Backend health</CardTitle>
              <Button onClick={() => void health.refetch()} busy={health.isFetching}>
                Run checks
              </Button>
            </div>
            <div className="mt-2 divide-y divide-edge/50">
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
