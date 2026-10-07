import { useRef, useState, type FormEvent, type KeyboardEvent } from "react";

import type { ConnectionStatus, Host, HostsFile, ProvisionStep } from "@/lib/bindings";
import {
  connectionName,
  serverRows,
  toHost,
  validateDestination,
  type ServerRow,
} from "@/lib/domain/hosts";
import {
  useAllWorkstreams,
  useConnectionStatus,
  useHostProbes,
  useHosts,
  useSaveHosts,
} from "@/lib/queries";
import { Icon, type IconName } from "@/components/Icon";
import { SettingsCard } from "@/components/SettingsCard";
import { Button, ConfirmDialog, cx, ErrorNote, Popover, TextInput } from "@/components/ui";
import { HOST_KEY_CHANGED, signInAction } from "./failures";
import { WorkStatus } from "./HostList";
import { openHostsUi } from "./hostsUi";
import { retryConnection } from "./reconnect";
import { Remedy } from "./Remedy";
import { NO_SERVER, STEP_LABEL } from "./serverInstall";
import { ServerSummary, worktreeCount } from "./ServerSummary";
import { SwitchServerConfirm, useServerSwitch } from "./SwitchServerConfirm";
import { useHostChecks, type HostCheck } from "./useHostChecks";

const EMPTY: HostsFile = { hosts: [], active: null };

/** Null for a blank destination: the empty field has nothing to be wrong about yet. */
function destinationProblem(destination: string, hosts: Host[], own?: string): string | null {
  const trimmed = destination.trim();
  if (!trimmed) return null;
  const why = validateDestination(trimmed);
  if (why) return `Destination ${why}.`;
  return hosts.some((h) => h.destination === trimmed && h.destination !== own)
    ? "Another server already uses this destination."
    : null;
}

type Draft = { destination: string; label: string };

function ConnectionInfo() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button
        ref={ref}
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label="How servers connect"
        aria-expanded={open}
        className="grid h-6 w-6 place-items-center rounded-control text-ink-dim hover:bg-edge/60 hover:text-ink"
      >
        <Icon name="info" size="sm" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={ref}
        align="end"
        className="w-80 space-y-2 p-3 text-ui leading-prose text-ink-dim"
      >
        <p>This machine's server is found automatically.</p>
        <p>
          Hyprlayer runs <code className="font-mono text-ink-2">ssh &lt;destination&gt;</code> with
          your SSH config and agent, so keys, ports and jump hosts come from ~/.ssh/config. It
          stores only the destination and label.
        </p>
        <p>
          When ssh needs a password or a new host's key trusted, Hyprlayer asks here and stores
          neither. One connection per host is shared for 10 minutes after its last use, unless your
          config sets its own ControlPath.
        </p>
        <p>If the host has no Hyprlayer server, Test offers to install one.</p>
      </Popover>
    </>
  );
}

export function ServersSettings() {
  const file = useHosts().data ?? EMPTY;
  const connection = useConnectionStatus().data;
  const save = useSaveHosts();
  const workstreams = useAllWorkstreams().data;
  const probes = useHostProbes();
  const checks = useHostChecks(file.active ?? null, probes);
  const serverSwitch = useServerSwitch(save);
  const [draft, setDraft] = useState<Draft>({ destination: "", label: "" });
  const [editing, setEditing] = useState<(Draft & { key: string }) | null>(null);
  const [removing, setRemoving] = useState<ServerRow | null>(null);
  const [retrying, setRetrying] = useState(false);

  const hosts = file.hosts ?? [];
  const rows = serverRows(file, connection, checks.checks);
  const probing = checks.testing !== null || checks.installing !== null || retrying;
  // The live connection already answers for the server it is connected to.
  const testable = rows.flatMap((r) =>
    r.host && !(r.isConnected && r.state === "ok") ? [r.host] : [],
  );
  const worktrees = worktreeCount(workstreams);

  const addProblem = destinationProblem(draft.destination, hosts);
  const canAdd = !!draft.destination.trim() && !addProblem && !save.isPending;

  async function add(e: FormEvent) {
    e.preventDefault();
    if (!canAdd) return;
    const host = toHost(draft.destination, draft.label);
    try {
      await save.mutateAsync({ ...file, hosts: [...hosts, host] });
    } catch {
      return; // the page's ErrorNote carries save.error
    }
    setDraft({ destination: "", label: "" });
    void checks.runTest(host);
  }

  async function saveEdit() {
    if (!editing) return;
    const next = hosts.map((h) =>
      h.destination === editing.key ? toHost(editing.destination, editing.label) : h,
    );
    try {
      await save.mutateAsync({ ...file, hosts: next });
    } catch {
      return;
    }
    setEditing(null);
  }

  async function remove(row: ServerRow) {
    try {
      await save.mutateAsync({ ...file, hosts: hosts.filter((h) => h.destination !== row.key) });
    } catch {
      // the page's ErrorNote carries save.error
    } finally {
      setRemoving(null);
    }
  }

  async function retry(row: ServerRow) {
    if (row.host && !row.isConnected) {
      await checks.runTest(row.host);
      return;
    }
    setRetrying(true);
    try {
      await retryConnection();
    } finally {
      setRetrying(false);
    }
  }

  return (
    <div className="space-y-3.5">
      <ErrorNote error={save.error} onDismiss={() => save.reset()} />

      <SettingsCard title="Add a Hyprlayer server" actions={<ConnectionInfo />}>
        <form onSubmit={(e) => void add(e)} className="space-y-2.5">
          <div className="flex items-center gap-2">
            <TextInput
              aria-label="SSH destination"
              placeholder="you@devbox.local"
              className={cx("flex-1 font-mono", addProblem && "border-bad/60 focus:border-bad")}
              value={draft.destination}
              disabled={save.isPending}
              onChange={(e) => setDraft({ ...draft, destination: e.target.value })}
            />
            <TextInput
              aria-label="Label"
              placeholder="Label"
              className="w-[170px] shrink-0"
              value={draft.label}
              disabled={save.isPending}
              onChange={(e) => setDraft({ ...draft, label: e.target.value })}
            />
            <Button type="submit" variant="primary" disabled={!canAdd}>
              Add server
            </Button>
          </div>
          {addProblem && (
            <p role="alert" className="text-ui text-bad-ink">
              {addProblem}
            </p>
          )}
        </form>
      </SettingsCard>

      <SettingsCard flush>
        <div className="flex items-center justify-between gap-3 px-4 py-3">
          <h2 className="lbl">Your servers</h2>
          <Button
            size="sm"
            busy={checks.testingAll}
            disabled={probing || testable.length === 0}
            onClick={() => void checks.testAll(testable)}
          >
            Test all
          </Button>
        </div>
        <ul aria-label="Servers">
          {rows.map((row) =>
            editing?.key === row.key && row.host ? (
              <ServerEditor
                key={row.key}
                row={row}
                draft={editing}
                problem={
                  editing.destination.trim()
                    ? destinationProblem(editing.destination, hosts, row.key)
                    : "SSH destination required."
                }
                saving={save.isPending}
                onChange={(next) => setEditing({ ...editing, ...next })}
                onSave={() => void saveEdit()}
                onCancel={() => setEditing(null)}
              />
            ) : (
              <ServerItem
                key={row.key}
                row={row}
                check={row.host ? checks.checks[row.key] : undefined}
                connection={connection}
                worktrees={row.isConnected ? worktrees : null}
                testing={checks.testing === row.key || (row.isConnected && retrying)}
                installStep={
                  checks.installing?.destination === row.key ? checks.installing.step : null
                }
                probing={probing}
                saving={save.isPending}
                onSetDefault={() =>
                  serverSwitch.request({ ...file, active: row.host?.destination ?? null, localProfile: null })
                }
                onEdit={() =>
                  row.host &&
                  setEditing({ key: row.key, destination: row.host.destination, label: row.host.label })
                }
                onRemove={() => setRemoving(row)}
                onRetry={() => void retry(row)}
                onInstall={() => row.host && void checks.runInstall(row.host)}
              />
            ),
          )}
        </ul>
      </SettingsCard>

      <ConfirmDialog
        open={removing !== null}
        onClose={() => setRemoving(null)}
        title="Remove server?"
        confirmLabel="Remove"
        confirmVariant="danger"
        onConfirm={() => removing && void remove(removing)}
        busy={save.isPending}
      >
        <p className="text-body leading-prose text-ink-dim">
          Hyprlayer forgets <span className="text-ink">{removing?.name}</span>. Nothing on it is
          stopped or deleted; adding it again brings its layout back.
        </p>
      </ConfirmDialog>

      <SwitchServerConfirm
        pending={serverSwitch.pending}
        leaving={connectionName(connection) ?? "this machine"}
        live={connection?.state !== "failed"}
        busy={save.isPending}
        onCancel={serverSwitch.cancel}
        onConfirm={() => void serverSwitch.confirm()}
      />
    </div>
  );
}

function RowIcon({
  label,
  icon,
  danger = false,
  disabled,
  onClick,
}: {
  label: string;
  icon: IconName;
  danger?: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className={cx(
        "grid h-6 w-6 place-items-center rounded-control disabled:opacity-50",
        danger
          ? "text-bad-ink hover:bg-bad/10"
          : "text-ink-dim hover:bg-edge/60 hover:text-ink",
      )}
    >
      <Icon name={icon} size="sm" />
    </button>
  );
}

function ServerItem({
  row,
  check,
  connection,
  worktrees,
  testing,
  installStep,
  probing,
  saving,
  onSetDefault,
  onEdit,
  onRemove,
  onRetry,
  onInstall,
}: {
  row: ServerRow;
  check: HostCheck | undefined;
  connection: ConnectionStatus | undefined;
  worktrees: number | null;
  testing: boolean;
  installStep: ProvisionStep | null;
  probing: boolean;
  saving: boolean;
  onSetDefault: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onRetry: () => void;
  onInstall: () => void;
}) {
  const unreachable = row.state === "unreachable";
  const outdated = row.isConnected && connection?.state === "connected" && connection.serverOutdated;
  const canInstall = unreachable && !!row.host && row.failureCode === NO_SERVER;
  const failure = row.isConnected ? connection?.error : check && !check.ok ? check : null;
  const remedy = unreachable && failure?.code === HOST_KEY_CHANGED ? failure.remedy : null;

  return (
    <li className={cx("border-t border-edge-2 px-4 py-3", unreachable && "bg-warn/5")}>
      <div className="flex items-center gap-3">
        <ServerSummary
          row={row}
          check={check}
          connection={connection}
          worktrees={worktrees}
          testing={testing}
        />
        <div className="flex shrink-0 items-center gap-1.5">
          {installStep ? (
            <WorkStatus label={STEP_LABEL[installStep]} />
          ) : (
            <>
              {canInstall && (
                <Button
                  size="sm"
                  disabled={probing}
                  title={`Install the server on ${row.name}`}
                  onClick={onInstall}
                >
                  Install
                </Button>
              )}
              {unreachable && (
                <Button size="sm" disabled={probing} onClick={onRetry}>
                  {signInAction(row.failureCode) ?? "Retry"}
                </Button>
              )}
              {outdated && (
                <Button size="sm" onClick={openHostsUi}>
                  Update
                </Button>
              )}
              {!unreachable && !row.isDefault && (
                <RowIcon label="Set default" icon="star" disabled={saving} onClick={onSetDefault} />
              )}
              {row.host && <RowIcon label="Edit" icon="edit" disabled={saving} onClick={onEdit} />}
              {row.host && !row.isDefault && (
                <RowIcon label="Remove" icon="close" danger disabled={saving} onClick={onRemove} />
              )}
            </>
          )}
        </div>
      </div>
      {remedy && <Remedy text={remedy} className="mt-2 pl-5.5" />}
    </li>
  );
}

function ServerEditor({
  row,
  draft,
  problem,
  saving,
  onChange,
  onSave,
  onCancel,
}: {
  row: ServerRow;
  draft: Draft;
  problem: string | null;
  saving: boolean;
  onChange: (next: Partial<Draft>) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    // Escape belongs to the row here, not to the settings view behind it.
    e.stopPropagation();
    onCancel();
  };

  return (
    <li className="border-t border-edge-2 px-4 py-3">
      <form
        className="space-y-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          if (!problem && !saving) onSave();
        }}
      >
        <div className="flex items-center gap-2">
          <TextInput
            aria-label={`Destination for ${row.name}`}
            placeholder="you@devbox.local"
            className={cx("flex-1 py-1 font-mono", problem && "border-bad/60 focus:border-bad")}
            value={draft.destination}
            // The default's destination names this window's profile; changing it needs a switch.
            disabled={row.isDefault || saving}
            title={row.isDefault ? "Make another server the default to change this destination." : undefined}
            autoFocus={!row.isDefault}
            onChange={(e) => onChange({ destination: e.target.value })}
            onKeyDown={onKeyDown}
          />
          <TextInput
            aria-label={`Label for ${row.name}`}
            placeholder="Label"
            className="w-[170px] shrink-0 py-1"
            value={draft.label}
            disabled={saving}
            autoFocus={row.isDefault}
            onChange={(e) => onChange({ label: e.target.value })}
            onKeyDown={onKeyDown}
          />
          <Button type="submit" size="sm" variant="primary" disabled={!!problem || saving}>
            Save
          </Button>
          <Button type="button" size="sm" onClick={onCancel}>
            Cancel
          </Button>
        </div>
        {problem && (
          <p role="alert" className="text-ui text-bad-ink">
            {problem}
          </p>
        )}
        {!row.isDefault && (
          <p className="text-ui text-ink-dim">
            Changing the destination starts this server with a fresh layout.
          </p>
        )}
      </form>
    </li>
  );
}
