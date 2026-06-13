import { Link } from "@tanstack/react-router";

import { useBootstrap, useRedetectBinaries } from "../lib/queries";
import { Badge, Button, Card, CardTitle, EmptyState, ErrorNote, PageError, Spinner } from "../components/ui";
import { BinaryCard } from "../features/system/BinaryCard";

export function Onboarding() {
  const bootstrap = useBootstrap();
  const redetect = useRedetectBinaries();

  if (bootstrap.isLoading) {
    return (
      <EmptyState title="Checking your environment…">
        <Spinner />
      </EmptyState>
    );
  }
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  const data = bootstrap.data!;
  const config = data.config;
  const requiredOk = data.binaries.every((b) => b.optional || b.state.state === "ok");
  const ready = requiredOk && config.state === "ready" && data.projects.length > 0;

  return (
    <div className="mx-auto max-w-2xl space-y-4 p-8">
      <div>
        <h1 className="text-xl font-semibold">Welcome to Hyprlayer</h1>
        <p className="mt-1 text-sm text-ink-dim">
          Research, plan, implement, and ship — with your AI agents and your team, in one place.
          Let's check the toolchain this app drives under the hood.
        </p>
      </div>

      <Card>
        <div className="flex items-center justify-between">
          <CardTitle className="mb-0">Command-line tools</CardTitle>
          <Button onClick={() => redetect.mutate()} busy={redetect.isPending}>
            Re-detect
          </Button>
        </div>
        <div className="mt-3 space-y-2">
          {data.binaries.map((b) => (
            <BinaryCard key={b.kind} status={b} />
          ))}
        </div>
      </Card>

      <Card>
        <CardTitle>Hyprlayer configuration</CardTitle>
        {config.state === "ready" && (
          <div className="space-y-1 text-sm">
            <p>
              <Badge tone="ok">Found</Badge>
              <span className="selectable ml-2 font-mono text-xs text-ink-dim">{config.path}</span>
            </p>
            <p className="text-ink-dim">
              User <span className="font-mono text-ink">{config.user || "—"}</span> · default backend{" "}
              <span className="font-mono text-ink">{config.backendKind}</span>
              {!config.configured && (
                <span className="ml-2">
                  <Badge tone="warn">incomplete</Badge>
                </span>
              )}
            </p>
          </div>
        )}
        {config.state === "missing" && (
          <div className="space-y-2 text-sm">
            <p>
              <Badge tone="warn">Not set up yet</Badge>
            </p>
            <p className="text-ink-dim">
              No hyprlayer config was found at{" "}
              <span className="selectable font-mono text-xs">{config.path}</span>. Run{" "}
              <code className="selectable rounded bg-panel-2 px-1">hyprlayer thoughts init</code> in a
              repository to set up your thoughts storage — the app will pick it up automatically.
              In-app setup lands in the next milestone.
            </p>
          </div>
        )}
        {config.state === "error" && (
          <ErrorNote error={{ message: config.message, remedy: config.remedy }} />
        )}
      </Card>

      <Card>
        <CardTitle>Projects</CardTitle>
        {data.projects.length > 0 ? (
          <ul className="space-y-1 text-sm">
            {data.projects.map((p) => (
              <li key={p.path} className="flex items-center gap-2">
                <Badge tone={p.existsOnDisk ? "ok" : "warn"}>{p.backendKind}</Badge>
                <span className="font-medium">{p.mappedName}</span>
                <span className="selectable truncate font-mono text-xs text-ink-dim">{p.path}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-ink-dim">
            No repositories are mapped yet. Run{" "}
            <code className="selectable rounded bg-panel-2 px-1">hyprlayer thoughts init</code> inside a
            repository, and it will appear here.
          </p>
        )}
      </Card>

      {ready && (
        <div className="flex justify-end">
          <Link to="/projects/$projectPath" params={{ projectPath: data.projects[0].path }}>
            <Button variant="primary">Open {data.projects[0].mappedName} →</Button>
          </Link>
        </div>
      )}
    </div>
  );
}
