/**
 * App-wide time/duration formatting. Lives in lib (not a feature) because
 * archive rows, dashboards, script docks, and transcripts all render the same
 * labels, one clock vocabulary for the whole app.
 */

/**
 * A live "time working" label for the elapsed counter: tenths
 * of a second under a minute (19.3s), then minutes + zero-padded seconds
 * (1m 23s), then hours + minutes for a rare long run (1h 04m). Negative inputs
 * clamp to zero.
 */
export function formatElapsed(ms: number): string {
  const totalSec = Math.max(0, ms) / 1000;
  if (totalSec < 60) return `${totalSec.toFixed(1)}s`;
  const totalMin = Math.floor(totalSec / 60);
  if (totalMin < 60) {
    return `${totalMin}m ${String(Math.floor(totalSec % 60)).padStart(2, "0")}s`;
  }
  return `${Math.floor(totalMin / 60)}h ${String(totalMin % 60).padStart(2, "0")}m`;
}

/**
 * The minutes-and-up unit ladder shared by the age formatters: `4m`, `2h`,
 * `3d`, `2w`, `5mo`, `1y`. Callers own the sub-minute tier and any suffix.
 */
function ageToken(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w`;
  // Gate on days, not on `months < 12`: at 364 days that reads 12 months and
  // falls through to a floor-divided "0y".
  if (days < 365) return `${Math.floor(days / 30)}mo`;
  return `${Math.floor(days / 365)}y`;
}

/**
 * The same clock as `formatRelativeTime`, stripped to a bare token, `4h`,
 * `2d`, `3w`, `1mo`, for dense rows where the column header already says the
 * number is an age. Returns "" for a missing instant so the caller renders
 * nothing rather than a misleading "just now".
 */
export function formatCompactAge(epochMs: number | null, from: number = Date.now()): string {
  if (epochMs === null || !Number.isFinite(epochMs)) return "";
  return ageToken(Math.max(0, Math.floor((from - epochMs) / 60_000)));
}

/**
 * A short relative-time label for a past instant, Claude-Code-style:
 * `just now`, `15s ago`, `4m ago`, `2h ago`, `3d ago`, `2w ago`, `5mo ago`,
 * `1y ago`. `from` defaults to a fresh `Date.now()` reading; pass it explicitly
 * in tests for determinism. Falls back to `just now` for the future or for
 * invalid values (a backfilled-from-null timestamp).
 */
export function formatRelativeTime(epochMs: number | null, from: number = Date.now()): string {
  if (epochMs === null || !Number.isFinite(epochMs)) return "just now";
  const seconds = Math.max(0, Math.floor((from - epochMs) / 1000));
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  return `${ageToken(Math.floor(seconds / 60))} ago`;
}

/**
 * A commit sha abbreviated for display. Seven characters, git's own default
 * abbreviation length; the full sha is what actually addresses a diff, so this
 * is presentation only and never round-trips.
 */
export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}
