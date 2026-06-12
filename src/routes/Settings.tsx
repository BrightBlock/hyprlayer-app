import { Link } from "@tanstack/react-router";

import { useBootstrap, useRedetectBinaries } from "../lib/queries";
import { Button, Card, CardTitle, ErrorNote, Spinner } from "../components/ui";
import { BinaryCard } from "../features/system/BinaryCard";

export function Settings() {
  const bootstrap = useBootstrap();
  const redetect = useRedetectBinaries();

  if (bootstrap.isLoading) {
    return (
      <div className="flex justify-center p-12">
        <Spinner />
      </div>
    );
  }
  if (bootstrap.isError) {
    return (
      <div className="p-8">
        <ErrorNote error={bootstrap.error} />
      </div>
    );
  }

  const data = bootstrap.data!;

  return (
    <div className="mx-auto max-w-2xl space-y-4 p-8">
      <h1 className="text-xl font-semibold">Settings</h1>

      <Card>
        <div className="flex items-center justify-between">
          <CardTitle className="mb-0">Binaries</CardTitle>
          <Button onClick={() => redetect.mutate()} busy={redetect.isPending}>
            Re-detect
          </Button>
        </div>
        <p className="mt-1 text-xs text-ink-dim">
          The app searches your shell PATH plus common install locations. Set an explicit path if a
          tool isn't found.
        </p>
        <div className="mt-3 space-y-2">
          {data.binaries.map((b) => (
            <BinaryCard key={b.kind} status={b} showOverride />
          ))}
        </div>
      </Card>

      <Card>
        <CardTitle>Hyprlayer config</CardTitle>
        <p className="selectable font-mono text-xs text-ink-dim">{data.config.path}</p>
        <p className="mt-2 text-xs text-ink-dim">
          The hyprlayer CLI owns this file; the app reads it and follows changes live.
        </p>
      </Card>

      <Card>
        <CardTitle>Projects</CardTitle>
        <div className="divide-y divide-edge/60">
          {data.projects.map((project) => (
            <Link
              key={project.path}
              to="/settings/projects/$projectPath"
              params={{ projectPath: project.path }}
              className="block py-2 text-sm hover:text-ink"
            >
              <span className="block text-ink">{project.mappedName}</span>
              <span className="selectable block truncate font-mono text-xs text-ink-dim">
                {project.path}
              </span>
            </Link>
          ))}
          {data.projects.length === 0 && (
            <p className="py-2 text-sm text-ink-dim">No projects mapped.</p>
          )}
        </div>
      </Card>
    </div>
  );
}
