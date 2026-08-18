// In-app self-update state: whether a newer Hyprlayer exists and how to install
// it. The Rust updater plugin fetches the ed25519-signed manifest from
// brightblock.ai/hyprlayer/updates/latest.json and verifies it against the
// bundled public key. downloadAndInstall() swaps the app in place; relaunch()
// restarts into the new version.
//
// Linux is notify-only. Every Linux install is a distro package owned by
// pacman/apt/dnf, so nothing may write over /usr/bin. The manifest carries no
// linux-x86_64 key, and version discovery comes from changelog.json instead.
// See docs/RELEASING.md.
//
// Two surfaces consume this store:
//   - Toast (bottom-right):    the "Restart to update" nudge, from both the
//     background probe and an explicit "Check for Updates".
//   - Settings → Updates card: current version plus a manual "Check now".
//
// The changelog is a separate human-authored file fetched over plain HTTP (CSP
// is null in tauri.conf.json). Off Linux it is optional: if the fetch fails the
// updater still works, without notes.

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

import { IS_LINUX } from "./platform";
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
  /** One short sentence (≤15 words), renders in the narrow sidebar. */
  summary: string;
  /** Plain-text bullets describing the changes. */
  notes?: string[];
}

type Installing = "downloading" | "installing" | null;

// The live Tauri Update handle, kept OUT of reactive state. It's a stateful SDK
// object (carries downloadAndInstall() / a resource id) that only install()
// ever touches, so the store exposes just the version string the UI needs,
// which also lets the dev mock set a version without faking the whole type.
let pendingUpdate: Update | null = null;

/** Whether this build can install an update over itself. False on Linux, where
 *  the package manager owns the files, the UI points at it instead of offering
 *  a button that would fail. */
export const CAN_SELF_INSTALL = !IS_LINUX;

/** Is `a` a newer release than `b`? Both are the strict `x.y.z` that
 *  scripts/release.sh produces; anything else is treated as "not newer" so a
 *  malformed changelog can't nag. */
export function isNewerVersion(a: string, b: string): boolean {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  if (pa.length !== 3 || pb.length !== 3) return false;
  if ([...pa, ...pb].some((n) => !Number.isInteger(n) || n < 0)) return false;
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] > pb[i];
  }
  return false;
}

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

type Set = (p: Partial<UpdateState>) => void;
type Get = () => UpdateState;

/** The pending version, or null when up to date.
 *
 *  Off Linux this is the signed manifest via the updater plugin, and it leaves
 *  `pendingUpdate` armed for install(). On Linux there is no manifest entry to
 *  check, so the changelog's newest entry is the signal, discovery only, since
 *  the actual install goes through the distro's own verified channel. */
async function findPending(set: Set, get: Get): Promise<string | null> {
  if (!IS_LINUX) {
    pendingUpdate = await check();
    return pendingUpdate?.version ?? null;
  }
  await fetchChangelog(set);
  // getVersion() rather than state.currentVersion: init() resolves that
  // asynchronously and the first probe can beat it.
  const current = await getVersion().catch(() => "");
  const latest = get().changelog?.[0]?.version;
  return current && latest && isNewerVersion(latest, current) ? latest : null;
}

async function fetchChangelog(set: Set) {
  try {
    const res = await fetch(CHANGELOG_URL, { cache: "no-cache" });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const json = await res.json();
    const versions: ChangelogEntry[] = Array.isArray(json?.versions) ? json.versions : [];
    set({ changelog: versions });
  } catch (e) {
    // No changelog is non-fatal, the update still installs.
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
        // there's no manifest to hit, skip the network entirely. Set
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

        // Off Linux the changelog is decoration fetched alongside the manifest
        // check; on Linux findPending() awaits it as the check itself.
        if (!IS_LINUX) void fetchChangelog(set);

        const probe = async () => {
          try {
            const v = await findPending(set, get);
            set({ pendingVersion: v });
            // Nudge once per session for a freshly-found version the user hasn't
            // already dismissed. The explicit menu check always re-shows it.
            if (v && v !== toastedVersion && v !== get().dismissedVersion) {
              toastedVersion = v;
              showUpdateToast();
            }
          } catch (e) {
            // Network down / malformed manifest, no toast, no harm.
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
          // in dev, install() is a safe no-op (there's no real Update handle).
          if (import.meta.env.VITE_MOCK_UPDATE) {
            set({ currentVersion: "0.1.0", pendingVersion: "9.9.9" });
            return "available";
          }
          return "uptodate";
        }
        if (!IS_LINUX) void fetchChangelog(set); // Linux: findPending awaits it
        try {
          const v = await findPending(set, get);
          set({ pendingVersion: v });
          return v ? "available" : "uptodate";
        } catch (e) {
          console.warn("[updater] manual check failed:", e);
          return "error";
        }
      },

      install: async () => {
        // No-op on Linux: pacman/apt/dnf own the install, and the UI offers no
        // button. This is the belt to that suspenders.
        if (!CAN_SELF_INSTALL || !pendingUpdate || get().installing) return;
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
 *  "Restart to update" action, the single update surface now that the sidebar
 *  pill is gone. Used by the background probe and the explicit "Check for
 *  Updates" menu action. No-op when nothing is pending.
 *
 *  On Linux it carries no action: the same nudge, but the install belongs to the
 *  package manager. */
export function showUpdateToast(): void {
  const { pendingVersion, changelog, install, dismissUpdate } = useUpdate.getState();
  if (!pendingVersion) return;
  const summary = changelog?.find((e) => e.version === pendingVersion)?.summary;
  const fallback = CAN_SELF_INSTALL
    ? `Hyprlayer v${pendingVersion} is ready to install.`
    : `Hyprlayer v${pendingVersion} is out. Update it with your package manager.`;
  const message = summary
    ? `v${pendingVersion}: ${summary}${CAN_SELF_INSTALL ? "" : " Update it with your package manager."}`
    : fallback;
  useToast.getState().show("Update available", message, {
      tone: "ok",
      action: CAN_SELF_INSTALL
        ? { label: "Restart to update", busyLabel: "Updating…", onClick: () => install() }
        : undefined,
      onDismiss: dismissUpdate,
    },
  );
}
