import type { ConnectionStatus, WorkstreamView, WorktreeMode } from "@/lib/bindings";
import type { ServerRow } from "@/lib/domain/hosts";
import { cx, Spinner } from "@/components/ui";
import { shortFailure } from "./failures";
import type { HostCheck } from "./useHostChecks";

const DOT: Record<ServerRow["state"], string> = {
  ok: "bg-ok",
  unreachable: "bg-warn",
  unknown: "bg-ink-4",
};

const STATE_LABEL: Record<ServerRow["state"], string> = {
  ok: "Reachable",
  unreachable: "Unreachable",
  unknown: "Not tested",
};

// `inplace` works in the checkout itself and `folder` has no git; neither owns a worktree.
const WORKTREE_MODES: ReadonlySet<WorktreeMode> = new Set(["isolated", "existing"]);

export function worktreeCount(workstreams: WorkstreamView[] | undefined): number | null {
  return workstreams?.filter((v) => WORKTREE_MODES.has(v.item.worktreeMode)).length ?? null;
}

const plural = (n: number, noun: string) => `${n} ${n === 1 ? noun : `${noun}s`}`;

/** A server's dot, name and detail line, as the Servers page and the status-bar menu show it. */
export function ServerSummary({
  row,
  check,
  connection,
  worktrees,
  testing = false,
}: {
  row: ServerRow;
  check: HostCheck | undefined;
  connection: ConnectionStatus | undefined;
  worktrees: number | null;
  testing?: boolean;
}) {
  const unreachable = row.state === "unreachable";
  const where = row.host?.destination ?? "this machine";
  // A check's own words beat its code: an "Install failed" is still `remote_cli_missing`.
  const failure = !row.isConnected && check && !check.ok ? check : null;
  const reason = failure?.short ?? (row.failureCode ? shortFailure(row.failureCode) : "unreachable");
  // TODO(servers): latency, last seen, other servers' worktrees — HostProbe is only {ok, serverVersion, error}, nothing persisted.
  const detail = unreachable
    ? `${where} · ${reason}`
    : row.state === "ok" && worktrees !== null
      ? `${where} · ${plural(worktrees, "worktree")}`
      : where;
  const tooltip = row.isConnected ? connection?.error?.message : failure?.detail;

  return (
    <>
      <span className="flex h-2.5 w-2.5 shrink-0 items-center justify-center">
        {testing ? (
          <Spinner size={10} />
        ) : (
          <span
            role="img"
            aria-label={row.isConnected && row.state === "ok" ? "Connected" : STATE_LABEL[row.state]}
            className={cx("h-2 w-2 rounded-full", DOT[row.state])}
          />
        )}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13px] text-ink">{row.name}</span>
          {row.isDefault && (
            <span className="shrink-0 rounded-control bg-accent-soft px-1.5 font-mono text-[9.5px] font-semibold text-ink">
              DEFAULT
            </span>
          )}
        </div>
        <div
          className={cx("truncate font-mono text-2xs", unreachable ? "text-warn-ink" : "text-ink-dim")}
          title={tooltip}
        >
          {detail}
        </div>
      </div>
    </>
  );
}
