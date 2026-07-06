import { useState } from "react";

import { useUpdate } from "@/lib/updater";
import { Badge, Button, Card, CardTitle } from "@/components/ui";

/**
 * Settings → Updates card: current version, a manual "Check now", and — when an
 * update is pending — its release notes plus an install button.
 */
export function UpdatesCard() {
  const currentVersion = useUpdate((s) => s.currentVersion);
  const pendingVersion = useUpdate((s) => s.pendingVersion);
  const changelog = useUpdate((s) => s.changelog);
  const installing = useUpdate((s) => s.installing);
  const checkNow = useUpdate((s) => s.checkNow);
  const install = useUpdate((s) => s.install);

  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<"available" | "uptodate" | "error" | null>(null);

  const entry = changelog?.find((e) => e.version === pendingVersion) ?? null;
  const busy = installing !== null;

  const onCheck = async () => {
    setChecking(true);
    setResult(null);
    setResult(await checkNow());
    setChecking(false);
  };

  return (
    <Card>
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <CardTitle className="mb-0">Updates</CardTitle>
          <p className="mt-1 text-sm text-ink-dim">
            Current version <span className="font-mono text-ink">{currentVersion || "—"}</span>
          </p>
        </div>
        <Button onClick={() => void onCheck()} busy={checking}>
          Check now
        </Button>
      </div>

      {pendingVersion ? (
        <div className="mt-3 rounded-md border border-accent/40 bg-accent/10 p-3">
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm font-medium text-ink">
              Version <span className="font-mono">{pendingVersion}</span> is available
            </span>
            <Button variant="primary" onClick={() => void install()} busy={busy}>
              {installing === "installing" ? "Restarting…" : "Update & restart"}
            </Button>
          </div>
          {entry?.summary && <p className="mt-2 text-sm text-ink-dim">{entry.summary}</p>}
          {entry?.notes && entry.notes.length > 0 && (
            <ul className="mt-2 list-disc space-y-0.5 pl-5 text-sm text-ink-dim">
              {entry.notes.map((note, i) => (
                <li key={i}>{note}</li>
              ))}
            </ul>
          )}
        </div>
      ) : (
        result && (
          <p className="mt-3">
            {result === "uptodate" && <Badge tone="ok">Up to date</Badge>}
            {result === "available" && <Badge tone="accent">Update found</Badge>}
            {result === "error" && <Badge tone="warn">Couldn’t check — try again</Badge>}
          </p>
        )
      )}
    </Card>
  );
}
