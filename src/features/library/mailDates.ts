export type DateGroup = "Today" | "Yesterday" | "This week" | "Earlier";

const DAY_MS = 86_400_000;

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

// Rounded: a DST day is 23 or 25 hours long.
function daysAgo(epochMs: number, now: Date): number {
  return Math.round((startOfDay(now) - startOfDay(new Date(epochMs))) / DAY_MS);
}

export function dateGroup(epochMs: number, now: Date): DateGroup {
  const days = daysAgo(epochMs, now);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days <= 6) return "This week";
  return "Earlier";
}

// A date-only stamp (frontmatter `date`) has no time of day to show.
export function mailTime(epochMs: number, now: Date, dateOnly = false): string {
  const at = new Date(epochMs);
  const days = daysAgo(epochMs, now);
  if (days <= 0) {
    return dateOnly ? "Today" : at.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
  }
  if (days === 1) return "Yesterday";
  return at.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(at.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

// Frontmatter dates are usually `YYYY-MM-DD`, which Date.parse would read as UTC midnight.
export function parseDocDate(date: string | null | undefined): { ms: number; dateOnly: boolean } | null {
  if (!date) return null;
  const day = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date.trim());
  if (day) return { ms: new Date(+day[1], +day[2] - 1, +day[3]).getTime(), dateOnly: true };
  const ms = Date.parse(date);
  return Number.isNaN(ms) ? null : { ms, dateOnly: false };
}
