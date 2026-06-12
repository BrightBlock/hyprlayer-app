import { Link, useParams } from "@tanstack/react-router";

import { useBackendHealth, useProjectDetail, useSyncThoughts } from "../lib/queries";
import { Badge, Button, Card, CardTitle, ErrorNote, Spinner } from "../components/ui";
import type { CheckStatus, HealthCheck } from "../lib/bindings";

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

export function ProjectSettingsPage() {
  const { projectPath } = useParams({ from: "/settings/projects/$projectPath" });
  const detail = useProjectDetail(projectPath);
  const health = useBackendHealth(projectPath);
  const sync = useSyncThoughts(projectPath);

  if (detail.isLoading) {
    return (
      <div className="flex justify-center p-12">
        <Spinner />
      </div>
    );
  }
  if (detail.isError) {
    return (
      <div className="p-8">
        <ErrorNote error={detail.error} />
      </div>
    );
  }

  const d = detail.data!;
  const settings = d.storage.settings ?? {};

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-8">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <Link
            to="/projects/$projectPath"
            params={{ projectPath }}
            className="text-xs text-ink-dim hover:text-ink"
          >
            {d.project.mappedName}
          </Link>
          <h1 className="mt-1 text-xl font-semibold">Project settings</h1>
          <p className="selectable truncate font-mono text-xs text-ink-dim">{d.project.path}</p>
        </div>
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
          <pre className="selectable overflow-x-auto whitespace-pre-wrap font-mono text-xs text-ink-dim">
            {sync.data.output.trim() || "Already up to date."}
          </pre>
        </Card>
      )}

      <Card>
        <CardTitle>Storage</CardTitle>
        <div className="space-y-1 text-sm">
          <p>
            <Badge tone="accent">{d.storage.backend}</Badge>
            {d.project.profile && <span className="ml-2 text-ink-dim">profile {d.project.profile}</span>}
          </p>
          {d.thoughtsRoot && (
            <p className="text-ink-dim">
              Thoughts root:{" "}
              <span className="selectable font-mono text-xs text-ink">{d.thoughtsRoot}</span>
            </p>
          )}
          {settings.reposDir && (
            <p className="text-ink-dim">
              Layout:{" "}
              <span className="font-mono text-xs text-ink">
                {settings.reposDir}/{d.storage.mappedName ?? "?"}/…
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
  );
}
