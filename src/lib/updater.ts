// In-app self-update state. One place that knows whether a newer Hyprlayer
// exists and how to install it. The Rust updater plugin fetches the
// ed25519-signed manifest from brightblock.ai/hyprlayer/updates/latest.json,
// verifies it against the public key baked into the bundle, and
// downloadAndInstall() swaps the app in place — then relaunch() restarts into
// the new version.
//
// Surfaces that consume this store:
//   - Toast (bottom-right)    — the single update nudge: a "Restart to update"
//     toast, shown by the background probe and the explicit "Check for Updates".
//   - Settings → Updates card — current version + manual "Check now".
//
// The changelog is a separate, human-authored file fetched over plain HTTP
// (CSP is null in tauri.conf.json, so no allow-listing needed). It's optional
// polish: if the fetch fails the updater still works, just without notes.

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

import { useToast } from "./toast";

const MANIFEST_BASE = "https://brightblock.ai/hyprlayer/updates";
const CHANGELOG_URL = `${MANIFEST_BASE}/changelog.json`;
const CHECK_INTERVAL_MS = 3 * 60 * 60 * 1000; // 3h

/** One per-version entry in changelog.json, newest first. Authored at release
 *  time by scripts/release.sh and copied to brightblock.ai by CI. */
export interface ChangelogEntry {
  version: string;
  /** ISO date (YYYY-MM-DD), stamped by release.sh. */
  date: string;
  /** One short sentence (≤15 words) — renders in the narrow sidebar. */
  summary: string;
  /** Plain-text bullets describing the changes. */
  notes?: string[];
}

type Installing = "downloading" | "installing" | null;

// The live Tauri Update handle, kept OUT of reactive state. It's a stateful SDK
// object (carries downloadAndInstall() / a resource id) that only install()
// ever touches, so the store exposes just the version string the UI needs —
// which also lets the dev mock set a version without faking the whole type.
let pendingUpdate: Update | null = null;

interface UpdateState {
  /** Running app version (getVersion()). Empty until resolved. */
  currentVersion: string;
  /** Version of the pending update, or null when up to date. */
  pendingVersion: string | null;
  /** Per-version changelog, newest first. null until fetched. */
  changelog: ChangelogEntry[] | null;
  /** Non-null while an install is in flight. */
  installing: Installing;
  /** Version whose update pill the user dismissed (×). Persisted. */
  dismissedVersion: string;

  /** Idempotent. Call once from main.tsx. */
  init: () => void;
  /** Manual "check for updates" (Settings). Resolves with the outcome so the
   *  caller can show feedback. In dev (no signed release) always "uptodate". */
  checkNow: () => Promise<"available" | "uptodate" | "error">;
  install: () => Promise<void>;
  dismissUpdate: () => void;
}

let _initialized = false;
// Version we've already auto-toasted this session, so the 3h re-probe doesn't
// re-pop the same nudge. The explicit menu check ignores this (always shows).
let toastedVersion: string | null = null;

async function fetchChangelog(set: (p: Partial<UpdateState>) => void) {
  try {
    const res = await fetch(CHANGELOG_URL, { cache: "no-cache" });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const json = await res.json();
    const versions: ChangelogEntry[] = Array.isArray(json?.versions) ? json.versions : [];
    set({ changelog: versions });
  } catch (e) {
    // No changelog is non-fatal — the update still installs.
    console.warn("[updater] changelog fetch failed:", e);
  }
}

export const useUpdate = create<UpdateState>()(
  persist(
    (set, get) => ({
      currentVersion: "",
      pendingVersion: null,
      changelog: null,
      installing: null,
      dismissedVersion: "",

      init: () => {
        if (_initialized) return;
        _initialized = true;

        getVersion()
          .then((v) => set({ currentVersion: v }))
          .catch(() => {
            /* getVersion only fails outside Tauri (e.g. vitest) */
          });

        // check() can't verify against a real signed release in dev, and
        // there's no manifest to hit — skip the network entirely. Set
        // VITE_MOCK_UPDATE to exercise the pill UI without a backend.
        if (import.meta.env.DEV) {
          if (import.meta.env.VITE_MOCK_UPDATE) {
            set({
              currentVersion: "0.1.0",
              pendingVersion: "9.9.9",
              changelog: [
                {
                  version: "9.9.9",
                  date: "2026-06-17",
                  summary: "A mock release so the update toast is developable in dev.",
                  notes: ["Mock note one.", "Mock note two."],
                },
              ],
            });
            // Mirror the prod probe's launch nudge so the toast is visible in dev.
            toastedVersion = "9.9.9";
            showUpdateToast();
          }
          return;
        }

        void fetchChangelog(set);

        const probe = async () => {
          try {
            pendingUpdate = await check();
            const v = pendingUpdate?.version ?? null;
            set({ pendingVersion: v });
            // Nudge once per session for a freshly-found version the user hasn't
            // already dismissed. The explicit menu check always re-shows it.
            if (v && v !== toastedVersion && v !== get().dismissedVersion) {
              toastedVersion = v;
              showUpdateToast();
            }
          } catch (e) {
            // Network down / malformed manifest — no toast, no harm.
            console.warn("[updater] check failed:", e);
          }
        };
        void probe();
        // Never cleared: the store is an app-lifetime singleton, so the probe
        // should keep running for as long as the window is open.
        window.setInterval(() => void probe(), CHECK_INTERVAL_MS);
      },

      checkNow: async () => {
        if (import.meta.env.DEV) {
          // Mirror init()'s mock so the "Restart to update" toast is developable
          // in dev — install() is a safe no-op (there's no real Update handle).
          if (import.meta.env.VITE_MOCK_UPDATE) {
            set({ currentVersion: "0.1.0", pendingVersion: "9.9.9" });
            return "available";
          }
          return "uptodate";
        }
        void fetchChangelog(set);
        try {
          pendingUpdate = await check();
          set({ pendingVersion: pendingUpdate?.version ?? null });
          return pendingUpdate ? "available" : "uptodate";
        } catch (e) {
          console.warn("[updater] manual check failed:", e);
          return "error";
        }
      },

      install: async () => {
        if (!pendingUpdate || get().installing) return;
        set({ installing: "downloading" });
        try {
          await pendingUpdate.downloadAndInstall();
          set({ installing: "installing" });
          // relaunch() spawns the freshly-installed app and exits this one.
          await relaunch();
        } catch (e) {
          console.error("[updater] install failed:", e);
          set({ installing: null });
        }
      },

      dismissUpdate: () => {
        const v = get().pendingVersion;
        if (v) set({ dismissedVersion: v });
      },
    }),
    {
      name: "hyprlayer:updates",
      version: 1,
      // Only the dismissal is durable; everything else is recomputed each run.
      partialize: (s) => ({ dismissedVersion: s.dismissedVersion }),
    },
  ),
);

/** Surface the pending update as a sticky bottom-right toast carrying a one-click
 *  "Restart to update" action — the single update surface now that the sidebar
 *  pill is gone. Used by the background probe and the explicit "Check for
 *  Updates" menu action. No-op when nothing is pending. */
export function showUpdateToast(): void {
  const { pendingVersion, changelog, install, dismissUpdate } = useUpdate.getState();
  if (!pendingVersion) return;
  const summary = changelog?.find((e) => e.version === pendingVersion)?.summary;
  useToast.getState().show(
    "Update available",
    summary ? `v${pendingVersion} — ${summary}` : `Hyprlayer v${pendingVersion} is ready to install.`,
    {
      tone: "ok",
      action: { label: "Restart to update", busyLabel: "Updating…", onClick: () => install() },
      onDismiss: dismissUpdate,
    },
  );
}
