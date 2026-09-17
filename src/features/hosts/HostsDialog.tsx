import { useCallback, useEffect, useRef, useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";

import { type ConnectionState, type Host, type HostsFile, type ProvisionStep } from "@/lib/bindings";
import { errorMessage, errorRemedy } from "@/lib/ipc";
import { flushPersistedWrites } from "@/lib/persistedStorage";
import { useConnectionStatus, useHostProvisioning, useHosts, useSaveHosts } from "@/lib/queries";
import {
  Badge,
  Button,
  ConfirmDialog,
  Dialog,
  ErrorNote,
  Spinner,
  type BadgeTone,
} from "@/components/ui";
import { closeHostsUi, useHostsUiStore } from "./hostsUi";
import { HostList } from "./HostList";
import { retryConnection } from "./reconnect";
import { runProvision, STEP_LABEL } from "./serverInstall";

const EMPTY: HostsFile = { hosts: [], active: null };

const STATE_BADGE: Record<ConnectionState, { tone: BadgeTone; label: string }> = {
  connected: { tone: "ok", label: "Connected" },
  failed: { tone: "bad", label: "Not connected" },
  lost: { tone: "warn", label: "Connection lost" },
};

// Blocking mode has no dismissal: Escape still consumes into the LIFO command
// stack and the backdrop still swallows its mousedown, both into this no-op.
const NOOP = () => {};

function relaunchNow(): void {
  // A relaunch fires neither pagehide nor visibilitychange, so up to a second of
  // debounced writes would be lost without this.
  flushPersistedWrites();
  void relaunch();
}

/**
 * Installing an executable on someone else's machine is a real action, so it
 * names the version and the step it is on (plan Phase 7 §4). The host is named
 * once, in the dialog header above — not again on every line here.
 */
function ServerInstall({ host }: { host: Host }) {
  const info = useHostProvisioning(host);
  const [step, setStep] = useState<ProvisionStep | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState(false);
  const started = useRef(false);

  const data = info.data ?? null;

  const start = useCallback(() => {
    started.current = true;
    setError(null);
    setStep("probe");
    void (async () => {
      try {
        await runProvision(host, setStep);
        setDone(true);
        // The failure the header is showing is the one this install just fixed;
        // adopting that costs a reconnect, not a relaunch.
        void retryConnection();
      } catch (e) {
        setError(e);
        setStep(null);
      }
    })();
  }, [host]);

  // The first install is a trust decision; a reinstall, or a version bump on a
  // host that already carries one of our servers, is not — those proceed.
  useEffect(() => {
    if (!data || data.state === "ready" || started.current) return;
    if (data.installedVersions.length > 0) start();
  }, [data, start]);

  if (!data || data.state === "ready") return null;

  // Only when it says something the button does not: that a same-version binary
  // is being replaced. Which sessions a version change ends belongs to the
  // switch confirm, which is where the user chooses to end them.
  const note =
    data.state === "stale"
      ? `A different build of ${data.version} is installed; it will be replaced.`
      : null;

  return (
    <div className="space-y-3">
      {note && <p className="text-sm text-ink-dim">{note}</p>}
      {done ? (
        <p className="text-sm text-ink">Installed. Relaunch to connect.</p>
      ) : step ? (
        <p className="flex items-center gap-2 text-sm text-ink-dim">
          <Spinner /> {STEP_LABEL[step]}…
        </p>
      ) : (
        <Button variant="primary" onClick={start}>
          Install server {data.version}
        </Button>
      )}
      <ErrorNote error={error} />
    </div>
  );
}

export function HostsDialog() {
  const status = useConnectionStatus();
  const hosts = useHosts();
  const save = useSaveHosts();
  const uiOpen = useHostsUiStore((s) => s.open);
  // The file a confirmed switch would write; null while nothing is pending.
  const [pending, setPending] = useState<HostsFile | null>(null);

  // There is no initialData, so `data` is undefined on the first render of every
  // launch; gating on `isSuccess` is what keeps a reasonless "Not connected"
  // modal from flashing before the first connection_status resolves.
  const blocking = status.isSuccess && status.data.state !== "connected";
  // Deliberate mode (opened from the rail indicator while connected) is
  // dismissible; blocking mode never is, regardless of how it got opened.
  if (!blocking && !uiOpen) return null;

  const connection = status.data ?? null;
  const file = hosts.data ?? EMPTY;
  const badge = connection ? STATE_BADGE[connection.state] : STATE_BADGE.connected;
  const hostName = connection?.host ? connection.host.label || connection.host.destination : null;
  const where = hostName ?? "This machine";
  const message = connection?.error ? errorMessage(connection.error) : null;
  // `remote_cli_missing` is the one failure provisioning answers — Phase 1 named
  // it as the trigger this phase would give it.
  const showInstall = Boolean(connection?.host) && connection?.error?.code === "remote_cli_missing";
  // Its remedy reads "Install it from the Hyprlayer server panel", which is the
  // panel rendered directly below — a call to action pointing at itself.
  const remedy = connection?.error && !showInstall ? errorRemedy(connection.error) : null;
  const localFailed = blocking && connection?.host === null;
  const usingLocal = (file.active ?? null) === null;
  const leaving = hostName ?? "this machine";
  const target = pending?.active
    ? ((pending.hosts ?? []).find((h) => h.destination === pending.active) ?? null)
    : null;
  const joining = target ? target.label || target.destination : "this machine";

  async function confirmSwitch() {
    const next = pending;
    if (!next) return;
    try {
      await save.mutateAsync(next);
    } catch {
      setPending(null); // the dialog's own ErrorNote carries save.error
      return;
    }
    setPending(null);
    relaunchNow();
  }

  return (
    <>
      <Dialog
        open
        onClose={blocking ? NOOP : closeHostsUi}
        closable={!blocking}
        size="2xl"
        align="top"
        seams
        title="Hyprlayer server"
        ariaLabel="Hyprlayer server"
        footer={
          <div className="flex items-center justify-between gap-3">
            <Button
              onClick={() => setPending({ ...file, active: null })}
              disabled={localFailed || usingLocal || save.isPending}
            >
              Use this machine
            </Button>
            <Button variant="primary" onClick={relaunchNow}>
              Relaunch
            </Button>
          </div>
        }
      >
        <div className="space-y-5">
          <div>
            <div className="flex items-center gap-2">
              <Badge tone={badge.tone}>{badge.label}</Badge>
              <span className="min-w-0 truncate text-sm text-ink">{where}</span>
            </div>
            {message && <p className="mt-2 text-sm text-ink-dim">{message}</p>}
            {remedy && <p className="mt-1 text-sm text-ink-dim">{remedy}</p>}
          </div>

          {showInstall && connection?.host && <ServerInstall host={connection.host} />}

          <HostList
            value={file}
            onChange={(next) => save.mutate(next)}
            onSelect={setPending}
            disabled={save.isPending}
          />

          <ErrorNote error={save.error} />
        </div>
      </Dialog>
      <ConfirmDialog
        open={pending !== null}
        onClose={() => setPending(null)}
        title="Switch server?"
        confirmLabel="Switch and restart"
        onConfirm={() => void confirmSwitch()}
        busy={save.isPending}
      >
        <p className="text-sm leading-relaxed text-ink-dim">
          Hyprlayer will restart on <span className="text-ink">{joining}</span>.
        </p>
        {connection?.state !== "failed" && (
          <p className="text-sm leading-relaxed text-ink-dim">
            Agents and terminals on {leaving} keep running — they live on the server, not in this
            window, and are still there when you switch back.
          </p>
        )}
      </ConfirmDialog>
    </>
  );
}
