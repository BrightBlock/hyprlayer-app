import { useState } from "react";

import type { AppSettings, NotificationSettings } from "@/lib/bindings";
import { useSetAppSettings } from "@/lib/queries";
import { invalidateNotificationSettingsCache, sendTestNotification } from "@/lib/notifications";
import { Button, Card, CardTitle, ErrorNote, SettingRow, Toggle } from "@/components/ui";

const DEFAULT_NOTIFICATIONS: Required<NotificationSettings> = {
  enabled: true,
  sound: true,
};

export function NotificationsSettings({ settings }: { settings: AppSettings }) {
  const raw = settings.notifications ?? {};
  const notif: Required<NotificationSettings> = {
    enabled: raw.enabled ?? DEFAULT_NOTIFICATIONS.enabled,
    sound: raw.sound ?? DEFAULT_NOTIFICATIONS.sound,
  };
  const save = useSetAppSettings();
  const [testStatus, setTestStatus] = useState<
    null | { ok: true } | { ok: false; reason: "permission_denied" | "error" }
  >(null);

  function patch(next: Partial<NotificationSettings>) {
    save.mutate(
      { ...settings, notifications: { ...notif, ...next } },
      { onSuccess: () => invalidateNotificationSettingsCache() },
    );
  }

  async function runTest() {
    setTestStatus(null);
    const result = await sendTestNotification();
    setTestStatus(result.sent ? { ok: true } : { ok: false, reason: result.reason ?? "error" });
  }

  return (
    <Card>
      <CardTitle>Notifications</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">Native OS toasts.</p>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}

      <div className="mt-3 divide-y divide-edge/50">
        <SettingRow
          title="Notify when an agent needs you"
          control={
            <Toggle
              aria-label="Notify when an agent needs you"
              checked={notif.enabled}
              onChange={(v) => patch({ enabled: v })}
            />
          }
        >
          Show a toast when a session pauses for permission or a question.
        </SettingRow>

        <SettingRow
          title="Play sound"
          control={
            <Toggle
              aria-label="Play sound"
              checked={notif.sound}
              onChange={(v) => patch({ sound: v })}
            />
          }
        />

        <SettingRow
          title="Test notification"
          control={
            <Button onClick={runTest}>Send test</Button>
          }
        >
          {testStatus === null ? undefined : testStatus.ok ? (
            <span className="text-ok">Sent. Check the top-right.</span>
          ) : testStatus.reason === "permission_denied" ? (
            <span className="text-warn-ink">
              macOS denied permission. Enable it in System Settings → Notifications.
            </span>
          ) : (
            <span className="text-bad">Dispatch failed. See the console.</span>
          )}
        </SettingRow>
      </div>
    </Card>
  );
}
