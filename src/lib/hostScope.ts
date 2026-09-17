import { LOCAL_PROFILE } from "@/lib/domain/hosts";

declare global {
  interface Window {
    __HYPRLAYER_PROFILE__?: string;
  }
}

/** The active host's destination, injected by the `hyprlayer-profile` plugin's
    init script before any module runs; absent outside the webview. */
export const PROFILE_ID: string =
  (typeof window === "undefined" ? undefined : window.__HYPRLAYER_PROFILE__) ?? LOCAL_PROFILE;

// The server-scoped keys, in the order the plan lists them. The four
// machine-local ones (theme, ui, updates, diff-mode) are deliberately absent.
const SERVER_SCOPED_KEYS = [
  "hyprlayer:last-route",
  "hyprlayer:workstream-tabs",
  "hyprlayer:written-docs",
  "hyprlayer:scripts-dock",
  "hyprlayer:pinned-workstreams",
  "hyprlayer:library-scope",
  "hyprlayer:project-tree",
];

const NAMESPACE = `hyprlayer:${PROFILE_ID}:`;

// Machine-local and never scoped: it records that namespacing has taken effect
// on this machine, which is a fact about the browser store, not about a server.
const ADOPTED_KEY = "hyprlayer:profile-namespaced";

/** The local profile *is* the un-prefixed name, so nothing moves for a machine
    that has only ever used the local server. */
export function scopedKey(name: string): string {
  if (PROFILE_ID === LOCAL_PROFILE) return name;
  return NAMESPACE + name.replace(/^hyprlayer:/, "");
}

/** First launch after namespacing landed: a hand-configured host has been
    writing its state into the un-prefixed keys, which now belong to the local
    profile. Move them across — leaving copies behind would hand the remote's
    workstream ids and pins to this machine.

    The marker makes this a one-shot decision about the *upgrade*, not a
    per-profile one. A per-profile "namespace is empty" guard would re-run on
    the first switch from local to any remote and move that local profile's
    tabs into the remote, which is the opposite of the intent. */
export function adoptUnprefixedKeys(): void {
  try {
    if (localStorage.getItem(ADOPTED_KEY)) return;
    localStorage.setItem(ADOPTED_KEY, "1");
    if (PROFILE_ID === LOCAL_PROFILE) return;
    for (const name of SERVER_SCOPED_KEYS) {
      const value = localStorage.getItem(name);
      if (value === null) continue;
      localStorage.setItem(scopedKey(name), value);
      localStorage.removeItem(name);
    }
  } catch {
    // localStorage can throw outright; an un-adopted profile beats no app.
  }
}

// Not callable from main.tsx: every store hydrates during the import graph, so
// a later call would land after all seven had read an empty namespace.
adoptUnprefixedKeys();
