
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

import { log } from "./log";
import { flushPersistedWrites } from "./persistedStorage";
import { IS_LINUX } from "./platform";
import { useToast } from "./toast";

const MANIFEST_BASE = "https://brightblock.ai/hyprlayer/updates";
const CHANGELOG_URL = `${MANIFEST_BASE}/changelog.json`;
const CHECK_INTERVAL_MS = 3 * 60 * 60 * 1000;

export interface ChangelogEntry {
  version: string;
  date: string;
  summary: string;
  notes?: string[];
}

type Installing = "downloading" | "installing" | null;

let pendingUpdate: Update | null = null;

export const CAN_SELF_INSTALL = !IS_LINUX;

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
  currentVersion: string;
  pendingVersion: string | null;
  changelog: ChangelogEntry[] | null;
  installing: Installing;
  dismissedVersion: string;
  lastChecked: number | null;

  init: () => void;
  checkNow: () => Promise<"available" | "uptodate" | "error">;
  install: () => Promise<void>;
  dismissUpdate: () => void;
}

let _initialized = false;
let toastedVersion: string | null = null;

type Set = (p: Partial<UpdateState>) => void;
type Get = () => UpdateState;

async function findPending(set: Set, get: Get): Promise<string | null> {
  if (!IS_LINUX) {
    pendingUpdate = await check();
    return pendingUpdate?.version ?? null;
  }
  await fetchChangelog(set);
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
    log.warn("updater", "changelog fetch failed", e);
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
      lastChecked: null,

      init: () => {
        if (_initialized) return;
        _initialized = true;

        getVersion()
          .then((v) => set({ currentVersion: v }))
          .catch(() => {
          });

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
            toastedVersion = "9.9.9";
            showUpdateToast();
          }
          return;
        }

        if (!IS_LINUX) void fetchChangelog(set);

        const probe = async () => {
          try {
            const v = await findPending(set, get);
            set({ pendingVersion: v, lastChecked: Date.now() });
            if (v && v !== toastedVersion && v !== get().dismissedVersion) {
              toastedVersion = v;
              log.info("updater", `version ${v} is available`);
              showUpdateToast();
            }
          } catch (e) {
            log.warn("updater", "check failed", e);
          }
        };
        void probe();
        window.setInterval(() => void probe(), CHECK_INTERVAL_MS);
      },

      checkNow: async () => {
        if (import.meta.env.DEV) {
          if (import.meta.env.VITE_MOCK_UPDATE) {
            set({ currentVersion: "0.1.0", pendingVersion: "9.9.9", lastChecked: Date.now() });
            return "available";
          }
          set({ lastChecked: Date.now() });
          return "uptodate";
        }
        if (!IS_LINUX) void fetchChangelog(set);
        try {
          const v = await findPending(set, get);
          set({ pendingVersion: v, lastChecked: Date.now() });
          return v ? "available" : "uptodate";
        } catch (e) {
          log.warn("updater", "manual check failed", e);
          return "error";
        }
      },

      install: async () => {
        if (!CAN_SELF_INSTALL || !pendingUpdate || get().installing) return;
        set({ installing: "downloading" });
        log.info("updater", `installing ${pendingUpdate.version}`);
        try {
          await pendingUpdate.downloadAndInstall();
          set({ installing: "installing" });
          flushPersistedWrites();
          log.info("updater", "installed; relaunching");
          await relaunch();
        } catch (e) {
          log.error("updater", `installing ${pendingUpdate.version} failed`, e);
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
      partialize: (s) => ({ dismissedVersion: s.dismissedVersion, lastChecked: s.lastChecked }),
    },
  ),
);

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
