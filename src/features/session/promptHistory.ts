import type { TranscriptItem } from "./transcript";

export const MAX_HISTORY = 15;

export function userPrompts(items: TranscriptItem[]): string[] {
  const out: string[] = [];
  for (const it of items) {
    if (it.kind !== "user") continue;
    if (out.length > 0 && out[out.length - 1] === it.text) continue;
    out.push(it.text);
  }
  return out.length > MAX_HISTORY ? out.slice(out.length - MAX_HISTORY) : out;
}

export type HistoryNav = { index: number | null; stash: string };
export const IDLE_NAV: HistoryNav = { index: null, stash: "" };

export type NavResult = { nav: HistoryNav; draft: string } | null;

export function recallPrev(prompts: string[], nav: HistoryNav, draft: string): NavResult {
  if (prompts.length === 0) return null;
  if (nav.index === null) {
    const index = prompts.length - 1;
    return { nav: { index, stash: draft }, draft: prompts[index] };
  }
  if (nav.index <= 0) return null;
  const index = nav.index - 1;
  return { nav: { ...nav, index }, draft: prompts[index] };
}

export function recallNext(prompts: string[], nav: HistoryNav, _draft: string): NavResult {
  if (nav.index === null) return null;
  if (nav.index >= prompts.length - 1) {
    return { nav: IDLE_NAV, draft: nav.stash };
  }
  const index = nav.index + 1;
  return { nav: { ...nav, index }, draft: prompts[index] };
}

export function recallAt(
  prompts: string[],
  nav: HistoryNav,
  draft: string,
  index: number,
): NavResult {
  if (index < 0 || index >= prompts.length) return null;
  const stash = nav.index === null ? draft : nav.stash;
  return { nav: { index, stash }, draft: prompts[index] };
}

export function stackWindow(
  count: number,
  active: number | null,
  max: number,
): { start: number; end: number } {
  if (count <= max) return { start: 0, end: count };
  let start = count - max;
  if (active !== null && active < start) start = active;
  return { start, end: start + max };
}
