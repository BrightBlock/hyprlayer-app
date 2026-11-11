import { create } from "zustand";

import type { Harness } from "./bindings";
import { currentAppLocation, goToHref, openSettings } from "./appNav";

/** Where an auth card's "Open settings" came from, so a sign-in there goes back. */
export type PendingReauth = {
  sessionId: string;
  harness: Harness;
  returnTo: string | null;
};

type ReauthState = {
  pending: PendingReauth | null;
  /** Successful sign-ins per harness, however settings was reached. */
  signIns: Partial<Record<Harness, number>>;
};

export const useReauth = create<ReauthState>(() => ({ pending: null, signIns: {} }));

// Per session: the sign-in count when its open auth card was first shown.
const cardShownAt = new Map<string, number>();

export function beginReauth(sessionId: string, harness: Harness): void {
  useReauth.setState({
    pending: { sessionId, harness, returnTo: currentAppLocation()?.href ?? null },
  });
  openSettings("harnesses");
}

export function reauthSucceeded(harness: Harness): void {
  const { pending, signIns } = useReauth.getState();
  const back = pending?.harness === harness ? pending.returnTo : null;
  useReauth.setState({
    signIns: { ...signIns, [harness]: (signIns[harness] ?? 0) + 1 },
    pending: pending?.harness === harness ? null : pending,
  });
  if (back) goToHref(back);
}

/** Leaves settings without signing in; the card's Retry still works from there. */
export function returnFromReauth(): void {
  const { pending } = useReauth.getState();
  useReauth.setState({ pending: null });
  if (pending?.returnTo) goToHref(pending.returnTo);
}

/**
 * True once per sign-in made while this session's card was showing, so it retries once however
 * many views watch; a card first shown after the sign-in waits for Retry.
 */
export function signedInSinceCard(sessionId: string, harness: Harness): boolean {
  const now = useReauth.getState().signIns[harness] ?? 0;
  const shown = cardShownAt.get(sessionId);
  cardShownAt.set(sessionId, now);
  return shown !== undefined && now > shown;
}

/** The card closed: a later one counts afresh, and "Back to agent" stops pointing here. */
export function forgetCard(sessionId: string): void {
  cardShownAt.delete(sessionId);
  if (useReauth.getState().pending?.sessionId === sessionId) useReauth.setState({ pending: null });
}
