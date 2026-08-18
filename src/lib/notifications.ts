/**
 * Native OS toasts for "agent needs your attention". The caller decides
 * *whether* to fire, based on whether the user is looking at the session. This
 * module decides *how*: settings, OS permission, dispatch.
 *
 * Two macOS adaptations:
 *  - The plugin's `sound` rides legacy `NSUserNotification`, which Apple
 *    silences on 13+. We play it out-of-band via the `play_sound` command
 *    instead, so the banner stays silent and the sound stays reliable.
 *  - `onAction` is mobile-only on 2.3.3, so desktop gets no banner-click
 *    event. We keep a small ring of recent `(workstreamId, projectPath,
 *    firedAt)` intents and route on the focus edge: regaining focus within
 *    `ROUTE_WINDOW_MS` of a fire jumps to that workstream. Cmd-tabbing back
 *    without clicking still routes, but only against a fresh intent.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import { commands, type AppSettings, type WorkstreamView } from "./bindings";
import { unwrap } from "./ipc";
import { IS_MACOS } from "./platform";
import { queryClient } from "./queryClient";
import { goToWorkstream } from "./appNav";

type NotifyArgs = {
  workstreamId: string;
  projectPath: string;
  workstreamTitle: string;
  stage: string;
};

const DEFAULT_CONFIG = { enabled: true, sound: true } as const;
/** Window between a notification fire and a focus-edge in which we'll route
 *  to the originating workstream. Wider than human reaction time, narrower
 *  than "I came back for unrelated reasons." */
const ROUTE_WINDOW_MS = 15_000;

let cachedSettings: AppSettings | null = null;
let permissionPromise: Promise<boolean> | null = null;
let focusListenerInstalled = false;

type Intent = { projectPath: string; workstreamId: string; firedAt: number };
let lastIntent: Intent | null = null;

/** Drop the in-memory settings copy so the next notification refetches. Call
 *  this after `setAppSettings` succeeds. */
export function invalidateNotificationSettingsCache(): void {
  cachedSettings = null;
}

async function getSettings(): Promise<AppSettings> {
  if (cachedSettings) return cachedSettings;
  const bs = queryClient.getQueryData<{ settings: AppSettings }>(["bootstrap"]);
  if (bs?.settings) {
    cachedSettings = bs.settings;
    return bs.settings;
  }
  const settings = await unwrap(commands.getAppSettings());
  cachedSettings = settings;
  return settings;
}

async function ensurePermission(): Promise<boolean> {
  if (await isPermissionGranted()) return true;
  if (!permissionPromise) {
    permissionPromise = requestPermission()
      .then((p) => p === "granted")
      .catch(() => false);
  }
  return permissionPromise;
}

/** Play the OS notification sound. macOS uses `afplay` (see `play_sound` in
 *  src-tauri); other OSes get it via the plugin's `sound` field at send time. */
async function playSound(): Promise<void> {
  if (IS_MACOS) {
    try {
      await unwrap(commands.playSound());
    } catch {
      /* afplay failed, swallow, banner already fired */
    }
  }
}

/**
 * Install a window-focus listener once. On focus-edge transitions (false→true)
 * we route to the most recent intent if it's still fresh and the route still
 * resolves. Called from main.tsx at startup.
 */
export function setupNotificationActions(): void {
  if (focusListenerInstalled) return;
  focusListenerInstalled = true;
  const win = getCurrentWindow();
  void win.onFocusChanged(({ payload: focused }) => {
    if (!focused) return;
    const intent = lastIntent;
    if (!intent) return;
    if (Date.now() - intent.firedAt > ROUTE_WINDOW_MS) {
      lastIntent = null;
      return;
    }
    lastIntent = null;
    goToWorkstream({
      projectPath: intent.projectPath,
      workstreamId: intent.workstreamId,
    });
  });
}

/**
 * Fire a needs-you toast. Caller is responsible for deciding the user can't
 * already see the session that paused; this function just does the
 * settings + permission + OS-side dispatch.
 *
 * On macOS the visual banner goes through the plugin (silent), and the sound
 * is played separately via `afplay`. Stamps `lastIntent` so the focus-edge
 * listener can route the user back when they cmd-tab/click in.
 */
export async function notifyNeedsAttention(args: NotifyArgs): Promise<void> {
  try {
    const settings = await getSettings();
    const cfg = settings.notifications ?? DEFAULT_CONFIG;
    if (!cfg.enabled) return;
    if (!(await ensurePermission())) return;

    lastIntent = {
      projectPath: args.projectPath,
      workstreamId: args.workstreamId,
      firedAt: Date.now(),
    };

    sendNotification({
      title: `${args.workstreamTitle} needs you`,
      body: `Agent paused in ${args.stage} . Waiting for your input.`,
      // On non-macOS the plugin delivers the sound reliably; on macOS we
      // play it out-of-band below (the plugin's sound field is silent there).
      sound: !IS_MACOS && (cfg.sound ?? DEFAULT_CONFIG.sound) ? "default" : undefined,
    });
    if (cfg.sound ?? DEFAULT_CONFIG.sound) void playSound();
  } catch (e) {
    console.warn("notification dispatch failed", e);
  }
}

/** Fire a sample toast on demand, bypasses the enabled/visibility gates so
 *  the Settings "Send test" button can validate OS-level delivery (signing,
 *  TCC grant, focus mode) independently of the dispatch flow. Still respects
 *  the OS permission grant, requests it if missing. */
export async function sendTestNotification(): Promise<{
  sent: boolean;
  reason?: "permission_denied" | "error";
}> {
  try {
    if (!(await ensurePermission())) return { sent: false, reason: "permission_denied" };
    sendNotification({
      title: "Hyprlayer test toast",
      body: "If you see this, notifications are wired up correctly.",
      sound: !IS_MACOS ? "default" : undefined,
    });
    void playSound();
    return { sent: true };
  } catch (e) {
    console.warn("test notification failed", e);
    return { sent: false, reason: "error" };
  }
}

/** Look up the {projectPath, title} for a workstream from the react-query cache.
 *  Returns null if neither the "@all" list nor any project-scoped list holds it. */
export function findWorkstreamForNotify(
  workstreamId: string,
): { projectPath: string; title: string } | null {
  const all = queryClient.getQueryData<WorkstreamView[]>(["workstreams", "@all"]);
  if (all) {
    const hit = all.find((w) => w.item.id === workstreamId);
    if (hit) return { projectPath: hit.item.projectPath, title: hit.item.title };
  }
  for (const q of queryClient.getQueryCache().findAll({ queryKey: ["workstreams"] })) {
    const data = q.state.data as WorkstreamView[] | undefined;
    if (!data) continue;
    const hit = data.find((w) => w.item.id === workstreamId);
    if (hit) return { projectPath: hit.item.projectPath, title: hit.item.title };
  }
  return null;
}
