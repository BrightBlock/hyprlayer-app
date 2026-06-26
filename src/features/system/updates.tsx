import { useEffect, useRef, useState } from "react";

import { useUpdate } from "../../lib/updater";
import { Badge, Button, Card, CardTitle } from "../../components/ui";
import { DownloadIcon, XIcon } from "../../components/icons";

/**
 * Compact "Update available" card for the sidebar footer. Renders only when a
 * newer signed release is pending and the user hasn't dismissed that version.
 * Expanded, it shows the version and a one-line summary of what's new; the
 * primary button installs it (download → swap → relaunch).
 */
const EXIT_MS = 380;

export function UpdatePill({ collapsed }: { collapsed: boolean }) {
  const pendingVersion = useUpdate((s) => s.pendingVersion);
  const changelog = useUpdate((s) => s.changelog);
  const installing = useUpdate((s) => s.installing);
  const dismissedVersion = useUpdate((s) => s.dismissedVersion);
  const install = useUpdate((s) => s.install);
  const dismissUpdate = useUpdate((s) => s.dismissUpdate);

  const shouldShow = !!pendingVersion && pendingVersion !== dismissedVersion;
  const [mounted, setMounted] = useState(shouldShow);
  const [exiting, setExiting] = useState(false);
  const unmountRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (shouldShow) {
      if (unmountRef.current) clearTimeout(unmountRef.current);
      setExiting(false);
      setMounted(true);
    } else {
      setExiting(true);
      unmountRef.current = setTimeout(() => {
        setMounted(false);
        setExiting(false);
      }, EXIT_MS);
    }
    return () => { if (unmountRef.current) clearTimeout(unmountRef.current); };
  }, [shouldShow]);

  if (!mounted) return null;

  const animClass = exiting
    ? "animate-toast-out pointer-events-none"
    : "animate-toast-in";

  const busy = installing !== null;
  const entry = changelog?.find((e) => e.version === pendingVersion) ?? null;
  const label =
    installing === "installing" ? "Restarting…" : installing ? "Downloading…" : "Update & restart";

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
    <div className={`p-2 ${animClass}`}>
      <div className="rounded-lg border border-accent/40 bg-accent/10 p-2.5">
        <div className="flex items-start gap-2">
          <span className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-accent/20 text-accent">
            <DownloadIcon />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-1.5">
              <p className="text-xs font-semibold text-ink">Update available</p>
              <span className="shrink-0 font-mono text-[10px] text-ink-dim">v{pendingVersion}</span>
            </div>
            {entry?.summary && (
              <p className="mt-0.5 text-[11px] leading-snug text-ink-dim">
                {entry.summary}
              </p>
            )}
            {entry?.notes && entry.notes.length > 0 && (
              <ul className="mt-1.5 list-disc space-y-0.5 pl-3.5 text-[11px] leading-snug text-ink-dim">
                {entry.notes.slice(0, 3).map((note, i) => (
                  <li key={i}>{note}</li>
                ))}
                {entry.notes.length > 3 && (
                  <li className="-ml-3.5 list-none text-ink-dim/60">
                    +{entry.notes.length - 3} more…
                  </li>
                )}
              </ul>
            )}
          </div>
          {!busy && (
            <button
              type="button"
              onClick={() => dismissUpdate()}
              className="-mr-0.5 -mt-0.5 shrink-0 rounded p-1 text-ink-dim hover:bg-accent/15 hover:text-ink"
              title="Dismiss this update"
              aria-label="Dismiss this update"
            >
              <XIcon size={12} />
            </button>
          )}
        </div>
        <button
          type="button"
          onClick={() => void install()}
          disabled={busy}
          className="mt-2 w-full rounded-md bg-accent px-2.5 py-1.5 text-xs font-medium text-accent-ink hover:bg-accent/90 disabled:opacity-60"
        >
          {label}
        </button>
      </div>
    </div>
  );
}

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
