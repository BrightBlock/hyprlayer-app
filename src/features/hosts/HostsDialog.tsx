import { useCallback, useEffect, useRef, useState } from "react";

import {
  commands,
  type ConnectionState,
  type Host,
  type HostsFile,
} from "@/lib/bindings";
import { errorMessage, errorRemedy, unwrap } from "@/lib/ipc";
import {
  useConnectionStatus,
  useHostProvisioning,
  useHosts,
  useLocalServers,
  useSaveHosts,
} from "@/lib/queries";
import {
  Badge,
  Button,
  ConfirmDialog,
  Dialog,
  ErrorNote,
  type BadgeTone,
} from "@/components/ui";
import { closeHostsUi, useHostsUiStore } from "./hostsUi";
import { HostList, LocalServerList, WorkStatus } from "./HostList";
import { retryConnection } from "./reconnect";
import { cancelPlannedRestart, planRestart } from "./plannedRestart";
import { useServerAutoUpdate } from "./serverAutoUpdate";
import { runProvision, runUpdate, SERVER_OUTDATED, STEP_LABEL } from "./serverInstall";
import { relaunchNow, SwitchServerConfirm, useServerSwitch } from "./SwitchServerConfirm";

const EMPTY: HostsFile = { hosts: [], active: null };

const STATE_BADGE: Record<ConnectionState, { tone: BadgeTone; label: string }> = {
  connected: { tone: "ok", label: "Connected" },
  failed: { tone: "bad", label: "Not connected" },
  lost: { tone: "warn", label: "Connection lost" },
};

// Blocking mode has no dismissal: Escape still consumes into the LIFO command
// stack and the backdrop still swallows its mousedown, both into this no-op.
const NOOP = () => {};

/**
 * Installing an executable on someone else's machine is a real action, so it
 * names the version and the step it is on (plan Phase 7 §4). The host is named
 * once, in the dialog header above — not again on every line here.
 */
function ServerInstall({ host, onWork }: { host: Host; onWork: (work: string | null) => void }) {
  const info = useHostProvisioning(host);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState(false);
  const started = useRef(false);

  const data = info.data ?? null;

  const start = useCallback(() => {
    started.current = true;
    setError(null);
    setRunning(true);
    onWork(STEP_LABEL.probe);
    void (async () => {
      try {
        await runProvision(host, (step) => onWork(STEP_LABEL[step]));
        setDone(true);
        onWork("Connecting");
        // The failure the header is showing is the one this install just fixed;
        // adopting that costs a reconnect, not a relaunch.
        await retryConnection();
      } catch (e) {
        setError(e);
      } finally {
        setRunning(false);
        onWork(null);
      }
    })();
  }, [host, onWork]);

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
      {note && <p className="text-body text-ink-dim">{note}</p>}
      {done ? (
        <p className="text-body text-ink">Installed. Relaunch to connect.</p>
      ) : running ? null : (
        <Button variant="primary" onClick={start}>
          Install server {data.version}
        </Button>
      )}
      <ErrorNote error={error} />
    </div>
  );
}

/**
 * Locally the newer binary is already beside the app, so updating is a
 * restart; a host may need it installed first. Offered, never done on connect,
 * because it ends every agent and terminal on that server.
 */
function ServerUpdate({
  host,
  connected,
  where,
  release,
  onStarted,
  onWork,
}: {
  host: Host | null;
  connected: boolean;
  where: string;
  release: string | null;
  onStarted: () => void;
  onWork: (work: string | null) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const autoUpdating = useServerAutoUpdate((s) => s.updating);
  const [error, setError] = useState<unknown>(null);
  const label = host ? "Update server" : "Restart server";

  async function update() {
    setBusy(true);
    setError(null);
    planRestart();
    try {
      if (host) {
        setConfirming(false);
        await runUpdate(host, (step) => onWork(STEP_LABEL[step]));
        onStarted();
        // A live connection drops when the server stops, and that drop starts the
        // reconnect loop; retrying here as well would hold the loop off.
        if (!connected) {
          onWork("Connecting");
          await retryConnection();
        }
      } else {
        await unwrap(commands.restartServer());
        setConfirming(false);
        onStarted();
      }
    } catch (e) {
      cancelPlannedRestart();
      setError(e);
    } finally {
      setBusy(false);
      if (host) onWork(null);
    }
  }

  return (
    <div className="space-y-3">
      {/* Blocked, the header's error already names both versions. */}
      {connected && (
        <p className="text-body text-ink-dim">
          {release ? `The server is running ${release}` : "The server is running an older version"};
          this app ships a newer one.
        </p>
      )}
      <Button
        variant="primary"
        onClick={() => setConfirming(true)}
        disabled={busy}
        busy={autoUpdating}
      >
        {autoUpdating ? "Updating…" : label}
      </Button>
      {!confirming && <ErrorNote error={error} />}
      <ConfirmDialog
        open={confirming}
        onClose={() => setConfirming(false)}
        title={host ? "Update the server?" : "Restart the server?"}
        confirmLabel={label}
        confirmVariant="danger"
        onConfirm={() => void update()}
        busy={busy}
        error={error}
        onDismissError={() => setError(null)}
      >
        <p className="text-body leading-prose text-ink-dim">
          Every agent and terminal running on <span className="text-ink">{where}</span> will stop.
          Hyprlayer reconnects once the updated server is up.
        </p>
      </ConfirmDialog>
    </div>
  );
}

export function HostsDialog() {
  const status = useConnectionStatus();
  const hosts = useHosts();
  const save = useSaveHosts();
  const uiOpen = useHostsUiStore((s) => s.open);
  const serverSwitch = useServerSwitch(save);
  const [installWork, setInstallWork] = useState<string | null>(null);
  // A restart is done once the connection has dropped and come back.
  const [restart, setRestart] = useState<"requested" | "dropped" | null>(null);
  const state = status.data?.state;
  useEffect(() => {
    if (restart === "requested" && state !== "connected") setRestart("dropped");
    else if (restart === "dropped" && state === "connected") setRestart(null);
  }, [restart, state]);
  const work = installWork ?? (restart ? "Restarting" : null);
  const requestRestart = useCallback(() => setRestart("requested"), []);

  // There is no initialData, so `data` is undefined on the first render of every
  // launch; gating on `isSuccess` is what keeps a reasonless "Not connected"
  // modal from flashing before the first connection_status resolves.
  const blocking = status.isSuccess && status.data.state !== "connected";
  const localServers = useLocalServers(blocking || uiOpen).data ?? [];
  // Deliberate mode (opened from the status bar while connected) is
  // dismissible; blocking mode never is, regardless of how it got opened.
  if (!blocking && !uiOpen) return null;

  const connection = status.data ?? null;
  const file = hosts.data ?? EMPTY;
  const badge = connection ? STATE_BADGE[connection.state] : STATE_BADGE.connected;
  const hostName = connection?.host
    ? connection.host.label || connection.host.destination
    : (connection?.localServerName ?? null);
  const where = hostName ?? "This machine";
  const message = connection?.error ? errorMessage(connection.error) : null;
  // `remote_cli_missing` is the one failure provisioning answers — Phase 1 named
  // it as the trigger this phase would give it.
  const showInstall = Boolean(connection?.host) && connection?.error?.code === "remote_cli_missing";
  const tooOld = Boolean(connection?.host) && connection?.error?.code === SERVER_OUTDATED;
  const showUpdate = (connection?.state === "connected" && connection.serverOutdated) || tooOld;
  // Its remedy reads "Install it from the Hyprlayer server panel", which is the
  // panel rendered directly below — a call to action pointing at itself.
  const remedy = connection?.error && !showInstall && !tooOld ? errorRemedy(connection.error) : null;
  const localFailed = blocking && connection?.host === null && !connection?.localServerName;
  const localProfile = file.active ? null : (file.localProfile ?? null);
  const usingLocal = (file.active ?? null) === null && localProfile === null;
  const leaving = hostName ?? "this machine";

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
              onClick={() => serverSwitch.request({ ...file, active: null, localProfile: null })}
              disabled={localFailed || usingLocal || save.isPending}
            >
              Use this machine
            </Button>
            {/* Connected, the reconnect loop already covers any later drop. */}
            {blocking && (
              <Button variant="primary" onClick={relaunchNow}>
                Relaunch
              </Button>
            )}
          </div>
        }
      >
        <div className="space-y-5">
          <div>
            <div className="flex items-center gap-2">
              <Badge tone={badge.tone}>{badge.label}</Badge>
              <span className="min-w-0 truncate text-body text-ink">{where}</span>
              {work && !connection?.host && <WorkStatus label={work} />}
            </div>
            {message && <p className="mt-2 text-body text-ink-dim">{message}</p>}
            {remedy && <p className="mt-1 text-body text-ink-dim">{remedy}</p>}
          </div>

          {showInstall && connection?.host && <ServerInstall host={connection.host} onWork={setInstallWork} />}

          {showUpdate && (
            <ServerUpdate
              host={connection?.host ?? null}
              connected={connection?.state === "connected"}
              where={where}
              release={connection?.serverRelease ?? null}
              onStarted={requestRestart}
              onWork={setInstallWork}
            />
          )}

          <LocalServerList
            servers={localServers}
            active={localProfile}
            onSelect={(dataDir) =>
              serverSwitch.request({ ...file, active: null, localProfile: dataDir })
            }
            disabled={save.isPending}
          />

          <HostList
            value={file}
            onChange={(next) => save.mutate(next)}
            onSelect={serverSwitch.request}
            activeWork={connection?.host ? work : null}
            disabled={save.isPending}
          />

          <ErrorNote error={save.error} />
        </div>
      </Dialog>
      <SwitchServerConfirm
        pending={serverSwitch.pending}
        leaving={leaving}
        live={connection?.state !== "failed"}
        localServers={localServers}
        busy={save.isPending}
        onCancel={serverSwitch.cancel}
        onConfirm={() => void serverSwitch.confirm()}
      />
    </>
  );
}
