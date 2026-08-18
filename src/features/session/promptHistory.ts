/**
 * Pure logic for current-session prompt history: extract the user's submitted
 * prompts from the live transcript, and a shell-style navigation state machine
 * (Alt+↑/↓) over them. No React, no DOM, exhaustively unit-tested.
 */
import type { TranscriptItem } from "./transcript";

/** How many of the most recent prompts the history keeps. Bounded so a long
 *  session's history can't grow without limit (shell `HISTSIZE`); older prompts
 *  fall off the back, and the stack visual stays small. */
export const MAX_HISTORY = 15;

/** The session's user-submitted prompts, oldest→newest, as plain strings,
 *  capped to the most recent [`MAX_HISTORY`]. Source of truth is the live
 *  transcript (current-session scope). Exact consecutive repeats are collapsed
 *  (bash `ignoredups`) so re-sending the same message doesn't pad the stack. */
export function userPrompts(items: TranscriptItem[]): string[] {
  const out: string[] = [];
  for (const it of items) {
    if (it.kind !== "user") continue;
    if (out.length > 0 && out[out.length - 1] === it.text) continue;
    out.push(it.text);
  }
  // Keep only the most recent MAX_HISTORY; older prompts age out the back.
  return out.length > MAX_HISTORY ? out.slice(out.length - MAX_HISTORY) : out;
}

/** Navigation state. `index === null` = parked on the live draft (not
 *  navigating). `stash` holds the in-progress draft saved on entry, restored
 *  when the user steps forward past the newest entry. */
export type HistoryNav = { index: number | null; stash: string };
export const IDLE_NAV: HistoryNav = { index: null, stash: "" };

/** The next nav state plus the draft text to show, or null for a no-op. */
export type NavResult = { nav: HistoryNav; draft: string } | null;

/** Step toward older prompts (Alt+↑). Entering navigation stashes the live
 *  draft and jumps to the most recent prompt. Null at the oldest / no history. */
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

/** Step toward newer prompts (Alt+↓). Past the newest, exit navigation and
 *  restore the stashed draft. Null when not navigating. */
export function recallNext(prompts: string[], nav: HistoryNav, _draft: string): NavResult {
  if (nav.index === null) return null;
  if (nav.index >= prompts.length - 1) {
    return { nav: IDLE_NAV, draft: nav.stash };
  }
  const index = nav.index + 1;
  return { nav: { ...nav, index }, draft: prompts[index] };
}

/** Recall a specific entry (click-to-recall). Stashes the live draft on first
 *  entry into navigation, like recallPrev. */
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

/** Which slice of `count` lines the stack shows: the most-recent `max`, shifted
 *  up only as far as needed to keep an out-of-window `active` line visible.
 *  Returns a half-open [start, end) range. */
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
