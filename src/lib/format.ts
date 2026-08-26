
export function formatElapsed(ms: number): string {
  const totalSec = Math.max(0, ms) / 1000;
  if (totalSec < 60) return `${totalSec.toFixed(1)}s`;
  const totalMin = Math.floor(totalSec / 60);
  if (totalMin < 60) {
    return `${totalMin}m ${String(Math.floor(totalSec % 60)).padStart(2, "0")}s`;
  }
  return `${Math.floor(totalMin / 60)}h ${String(totalMin % 60).padStart(2, "0")}m`;
}

function ageToken(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w`;
  if (days < 365) return `${Math.floor(days / 30)}mo`;
  return `${Math.floor(days / 365)}y`;
}

export function formatCompactAge(epochMs: number | null, from: number = Date.now()): string {
  if (epochMs === null || !Number.isFinite(epochMs)) return "";
  return ageToken(Math.max(0, Math.floor((from - epochMs) / 60_000)));
}

export function formatRelativeTime(epochMs: number | null, from: number = Date.now()): string {
  if (epochMs === null || !Number.isFinite(epochMs)) return "just now";
  const seconds = Math.max(0, Math.floor((from - epochMs) / 1000));
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  return `${ageToken(Math.floor(seconds / 60))} ago`;
}

export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}
