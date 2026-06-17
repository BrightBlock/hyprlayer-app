import { useState } from "react";

import { useUpdate } from "../../lib/updater";
import { Badge, Button, Card, CardTitle } from "../../components/ui";
import { DownloadIcon } from "../../components/icons";

/**
 * Compact "Update available" pill for the sidebar footer. Renders only when a
 * newer signed release is pending and the user hasn't dismissed that version.
 * Clicking installs it (download → swap → relaunch).
 */
export function UpdatePill({ collapsed }: { collapsed: boolean }) {
  const pendingVersion = useUpdate((s) => s.pendingVersion);
  const installing = useUpdate((s) => s.installing);
  const dismissedVersion = useUpdate((s) => s.dismissedVersion);
  const install = useUpdate((s) => s.install);

  if (!pendingVersion || pendingVersion === dismissedVersion) return null;

  const busy = installing !== null;
  const label = installing === "installing" ? "Restarting…" : installing ? "Downloading…" : "Update";

  if (collapsed) {
    return (
      <button
        type="button"
        onClick={() => void install()}
        disabled={busy}
        className="flex h-8 w-8 items-center justify-center rounded-md bg-accent/15 text-accent hover:bg-accent/25 disabled:opacity-60"
        title={`Update to v${pendingVersion}`}
        aria-label={`Update to v${pendingVersion}`}
      >
        <DownloadIcon />
      </button>
    );
  }

  return (
    <button
      type="button"
      onClick={() => void install()}
      disabled={busy}
      className="flex w-full items-center gap-2 rounded-md border border-accent/40 bg-accent/10 px-2.5 py-1.5 text-xs font-medium text-accent hover:bg-accent/20 disabled:opacity-60"
      title={`Update to v${pendingVersion} and restart`}
    >
      <DownloadIcon />
      <span className="min-w-0 flex-1 truncate text-left">{label}</span>
      <span className="shrink-0 font-mono opacity-80">v{pendingVersion}</span>
    </button>
  );
}

/**
 * Settings → Updates card: shows the running version, a manual "Check now",
 * and — when an update is pending — its notes plus an install button.
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
      <div className="flex items-center justify-between">
        <CardTitle className="mb-0">Updates</CardTitle>
        <Button onClick={() => void onCheck()} busy={checking}>
          Check now
        </Button>
      </div>

      <p className="mt-1 text-sm text-ink-dim">
        Current version{" "}
        <span className="font-mono text-ink">{currentVersion || "—"}</span>. Hyprlayer checks for
        signed updates on launch and every few hours.
      </p>

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
          <p className="mt-3 text-sm">
            {result === "uptodate" && <Badge tone="ok">Up to date</Badge>}
            {result === "available" && <Badge tone="accent">Update found</Badge>}
            {result === "error" && <Badge tone="warn">Couldn’t check — try again</Badge>}
          </p>
        )
      )}
    </Card>
  );
}
