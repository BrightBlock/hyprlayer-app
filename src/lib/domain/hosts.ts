import type { ConnectionStatus, Host, HostsFile } from "@/lib/bindings";

export type { Host, HostsFile };

// The local server's profile name. A host may not take it: Phase 4 namespaces
// localStorage by the active destination and leaves `local` un-prefixed, so a
// host called `local` would silently share this machine's tabs and pins.
export const LOCAL_PROFILE = "local";

// ssh accepts its own `ssh://` form; any other scheme means a URL was pasted.
const URL_SCHEME = /^[a-z][a-z\d+.-]*:\/\//i;

export function validateDestination(destination: string): string | null {
  const trimmed = destination.trim();
  if (!trimmed) return null;
  if (/\s/.test(trimmed)) return "cannot contain spaces";
  // The destination is ssh's bare argv (supervisor.rs), so this would be an option.
  if (trimmed.startsWith("-")) return 'cannot start with "-" — ssh would read it as an option';
  if (URL_SCHEME.test(trimmed) && !/^ssh:\/\//i.test(trimmed)) {
    return "must be an SSH destination like user@host, not a URL";
  }
  if (trimmed === LOCAL_PROFILE) return `cannot be "${LOCAL_PROFILE}" — that name is this machine`;
  return null;
}

export function validateHostList(
  rows: { label: string; destination: string }[],
): Map<number, { label?: string; destination?: string }> {
  const errors = new Map<number, { label?: string; destination?: string }>();
  const put = (i: number, patch: { label?: string; destination?: string }) =>
    errors.set(i, { ...errors.get(i), ...patch });
  const seen = new Map<string, number>();
  rows.forEach((row, i) => {
    const destination = row.destination.trim();
    if (!destination) {
      put(i, { destination: "SSH destination required." });
      return;
    }
    const why = validateDestination(destination);
    if (why) {
      put(i, { destination: `Destination ${why}.` });
      return;
    }
    if (seen.has(destination)) put(i, { destination: "Already used by another host." });
    else seen.set(destination, i);
  });
  return errors;
}

/** A blank label becomes the destination: the backend names a host by its label in every error. */
export function toHost(destination: string, label: string): Host {
  const trimmed = destination.trim();
  return { destination: trimmed, label: label.trim() || trimmed };
}

export function hostName(host: Host): string {
  return host.label || host.destination;
}

/** Null for this app's own local server. */
export function connectionName(connection: ConnectionStatus | null | undefined): string | null {
  return connection?.host ? hostName(connection.host) : (connection?.localServerName ?? null);
}

/** One test of a host this session, keyed by destination. */
export type HostProbeResult = { ok: boolean; code?: string; release?: string | null; at: number };

export type ServerState = "ok" | "unreachable" | "unknown";

export type ServerRow = {
  /** `LOCAL_PROFILE` for this machine, else the destination. */
  key: string;
  /** Null for this machine. */
  host: Host | null;
  name: string;
  isDefault: boolean;
  isConnected: boolean;
  state: ServerState;
  /** The server's release version, as its live connection or last probe reported it. */
  release: string | null;
  failureCode?: string;
};

export function serverRows(
  file: HostsFile | undefined,
  connection: ConnectionStatus | undefined,
  probes: Record<string, HostProbeResult>,
): ServerRow[] {
  const hosts = file?.hosts ?? [];
  // As `hosts.rs` `target`: an active destination that names no listed host counts as none.
  const active = hosts.some((h) => h.destination === file?.active) ? file?.active : null;
  const connectedKey = connection ? (connection.host?.destination ?? LOCAL_PROFILE) : null;

  const row = (host: Host | null): ServerRow => {
    const key = host?.destination ?? LOCAL_PROFILE;
    const base = {
      key,
      host,
      name: host ? hostName(host) : "This machine",
      isDefault: (host?.destination ?? null) === active,
      isConnected: key === connectedKey,
    };
    // The live connection outranks any probe of the host it is connected to.
    if (base.isConnected && connection) {
      return connection.state === "connected"
        ? { ...base, state: "ok", release: connection.serverRelease }
        : { ...base, state: "unreachable", release: null, failureCode: connection.error?.code };
    }
    const probe = host ? probes[host.destination] : undefined;
    if (!probe) return { ...base, state: "unknown", release: null };
    return probe.ok
      ? { ...base, state: "ok", release: probe.release ?? null }
      : { ...base, state: "unreachable", release: null, failureCode: probe.code };
  };

  return [row(null), ...hosts.map((host) => row(host))];
}

export function serverSummary(rows: ServerRow[]): { total: number; unreachable: number } {
  return { total: rows.length, unreachable: rows.filter((r) => r.state === "unreachable").length };
}
