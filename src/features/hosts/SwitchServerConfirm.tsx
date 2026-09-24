import { useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";

import type { HostsFile, LocalServer } from "@/lib/bindings";
import { hostName } from "@/lib/domain/hosts";
import { flushPersistedWrites } from "@/lib/persistedStorage";
import { ConfirmDialog } from "@/components/ui";

export function relaunchNow(): void {
  // A relaunch fires neither pagehide nor visibilitychange, so up to a second of
  // debounced writes would be lost without this.
  flushPersistedWrites();
  void relaunch();
}

type SaveHosts = { mutateAsync: (file: HostsFile) => Promise<unknown> };

/** Takes the caller's save so its ErrorNote reports a failed switch too. */
export function useServerSwitch(save: SaveHosts) {
  // The file a confirmed switch would write; null while nothing is pending.
  const [pending, setPending] = useState<HostsFile | null>(null);

  async function confirm() {
    const next = pending;
    if (!next) return;
    try {
      await save.mutateAsync(next);
    } catch {
      setPending(null); // the caller's ErrorNote carries save.error
      return;
    }
    setPending(null);
    relaunchNow();
  }

  return {
    pending,
    request: (next: HostsFile) => setPending(next),
    cancel: () => setPending(null),
    confirm,
  };
}

export function SwitchServerConfirm({
  pending,
  leaving,
  live,
  localServers = [],
  busy,
  onCancel,
  onConfirm,
}: {
  pending: HostsFile | null;
  leaving: string;
  /** Whether the server being left can still have agents running on it. */
  live: boolean;
  localServers?: LocalServer[];
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const target = pending?.active
    ? ((pending.hosts ?? []).find((h) => h.destination === pending.active) ?? null)
    : null;
  const joining = target
    ? hostName(target)
    : pending?.localProfile
      ? (localServers.find((s) => s.dataDir === pending.localProfile)?.name ??
        "another server on this machine")
      : "this machine";

  return (
    <ConfirmDialog
      open={pending !== null}
      onClose={onCancel}
      title="Switch server?"
      confirmLabel="Switch and restart"
      onConfirm={onConfirm}
      busy={busy}
    >
      <p className="text-sm leading-relaxed text-ink-dim">
        Hyprlayer will restart on <span className="text-ink">{joining}</span>.
      </p>
      {live && (
        <p className="text-sm leading-relaxed text-ink-dim">
          Agents and terminals on {leaving} keep running — they live on the server, not in this
          window, and are still there when you switch back.
        </p>
      )}
    </ConfirmDialog>
  );
}
