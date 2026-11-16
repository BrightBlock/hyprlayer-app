
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import { commands, type AppSettings, type WorkstreamView } from "./bindings";
import { unwrap } from "./ipc";
import { log } from "./log";
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
const ROUTE_WINDOW_MS = 15_000;

let cachedSettings: AppSettings | null = null;
let permissionPromise: Promise<boolean> | null = null;
let focusListenerInstalled = false;

type Intent = { projectPath: string; workstreamId: string; firedAt: number };
let lastIntent: Intent | null = null;

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

async function playSound(): Promise<void> {
  if (IS_MACOS) {
    try {
      await unwrap(commands.playSound());
    } catch {
    }
  }
}

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

async function dispatch(
  target: { workstreamId: string; projectPath: string },
  title: string,
  body: string,
): Promise<void> {
  try {
    const settings = await getSettings();
    const cfg = settings.notifications ?? DEFAULT_CONFIG;
    if (!cfg.enabled) return;
    if (!(await ensurePermission())) return;

    lastIntent = {
      projectPath: target.projectPath,
      workstreamId: target.workstreamId,
      firedAt: Date.now(),
    };

    sendNotification({
      title,
      body,
      sound: !IS_MACOS && (cfg.sound ?? DEFAULT_CONFIG.sound) ? "default" : undefined,
    });
    if (cfg.sound ?? DEFAULT_CONFIG.sound) void playSound();
  } catch (e) {
    log.warn("notifications", "dispatch failed", e);
  }
}

export async function notifyNeedsAttention(args: NotifyArgs): Promise<void> {
  await dispatch(
    args,
    `${args.workstreamTitle} needs you`,
    `Agent paused in ${args.stage} . Waiting for your input.`,
  );
}

export async function notifyGoal(args: {
  workstreamId: string;
  projectPath: string;
  workstreamTitle: string;
  title: string;
  body: string;
}): Promise<void> {
  await dispatch(args, `${args.workstreamTitle}: ${args.title}`, args.body);
}

/** An agent's own `notify_user` or `request_attention`; the workstream names where it came from. */
export async function notifyFromAgent(args: {
  workstreamId: string;
  projectPath: string;
  workstreamTitle: string;
  title: string;
  body: string;
}): Promise<void> {
  await dispatch(args, `${args.workstreamTitle}: ${args.title}`, args.body);
}

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
    log.warn("notifications", "test notification failed", e);
    return { sent: false, reason: "error" };
  }
}

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
