import { HostMeters, hostRows } from "@/components/HostMeters";
import { Icon } from "@/components/Icon";
import { Badge } from "@/components/ui";
import type { ConnectionStatus } from "@/lib/bindings";
import { connectionName } from "@/lib/domain/hosts";
import { useActiveSessions, useAllWorkstreams, useHostStats } from "@/lib/queries";

/** The connected server's host and load, at the top of the Server menu while stats are shown. */
export function ServerHostStats({ connection }: { connection: ConnectionStatus | undefined }) {
  const stats = useHostStats().data;
  const agents = useActiveSessions().data?.length;
  const workstreams = useAllWorkstreams().data?.length;
  const name = connectionName(connection) ?? "This machine";
  const host = connection?.host;

  return (
    <section aria-label={`Server stats for ${name}`}>
      <div className="flex flex-col gap-[3px] px-3 pb-2.5 pt-1">
        <div className="flex min-w-0 items-center gap-[7px]">
          <Icon name="server" size="sm" className="text-ink-dim" />
          <span className="truncate text-ui font-semibold text-ink">{name}</span>
          {host && <Badge>remote</Badge>}
          {stats && (
            <span className="ml-auto shrink-0 font-mono text-meta tabular-nums text-ink-dim">{stats.cpus} cores</span>
          )}
        </div>
        {host && host.label && host.label !== host.destination && (
          <span className="truncate pl-[21px] font-mono text-meta text-ink-dim">{host.destination}</span>
        )}
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 px-3 pb-3 font-mono text-meta tabular-nums">
        <Stat label="Agents running" value={agents} />
        <Stat label="Workstreams" value={workstreams} />
      </dl>
      {stats &&
        (stats.cpuPercent == null ? (
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 px-3 pb-3 font-mono text-meta tabular-nums">
            {hostRows(stats).map(([label, value]) => (
              <Stat key={label} label={label} value={value} />
            ))}
          </dl>
        ) : (
          // The Lead panel's settle on mount would replay on every open; this menu holds still.
          <div data-still-motion>
            <HostMeters stats={stats} cpuPercent={stats.cpuPercent} />
          </div>
        ))}
    </section>
  );
}

function Stat({ label, value }: { label: string; value: string | number | undefined }) {
  return (
    <>
      <dt className="text-ink-dim">{label}</dt>
      <dd className="text-right text-ink">{value ?? "–"}</dd>
    </>
  );
}
