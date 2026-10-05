
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

const COUNT_UNITS = [
  { size: 1_000_000, suffix: "M" },
  { size: 1_000, suffix: "k" },
];

// Floors at every step: a count never reads larger than it is.
export function formatCompactCount(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0";
  const whole = Math.floor(n);
  const unit = COUNT_UNITS.find((u) => whole >= u.size);
  if (!unit) return String(whole);
  const tenths = Math.floor(whole / (unit.size / 10));
  const units = Math.floor(tenths / 10);
  const tenth = tenths % 10;
  return units >= 10 || tenth === 0 ? `${units}${unit.suffix}` : `${units}.${tenth}${unit.suffix}`;
}

export function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

// Two letters even for a one-word handle ("jt" → "JT").
export function initials(handle: string | null | undefined): string | null {
  const words = (handle ?? "").split(/[\s._-]+/).filter(Boolean);
  if (words.length === 0) return null;
  const letters =
    words.length === 1
      ? Array.from(words[0]).slice(0, 2)
      : [Array.from(words[0])[0], Array.from(words[words.length - 1])[0]];
  return letters.join("").toUpperCase();
}

export function initialsOf(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

const BYTE_UNITS = [
  { size: 1e12, suffix: "TB", digits: 1 },
  { size: 1e9, suffix: "GB", digits: 1 },
  { size: 1e6, suffix: "MB", digits: 0 },
  { size: 1e3, suffix: "KB", digits: 0 },
];

// Decimal units, as Finder reports sizes. A value that would round to 1000 of
// one unit is shown as the next ("1.0 GB", never "1000 MB").
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 1e3) return `${Math.max(0, Math.round(bytes || 0))} B`;
  const at = BYTE_UNITS.findIndex((u) => bytes >= u.size);
  const unit =
    at > 0 && Number((bytes / BYTE_UNITS[at].size).toFixed(BYTE_UNITS[at].digits)) >= 1000
      ? BYTE_UNITS[at - 1]
      : BYTE_UNITS[at];
  return `${(bytes / unit.size).toFixed(unit.digits)} ${unit.suffix}`;
}
