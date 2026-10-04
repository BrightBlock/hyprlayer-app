import { Badge, IconButton, RefreshButton, Spinner } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { formatRelativeTime } from "@/lib/format";

import { AgentUpdateFailed, AgentUpdateReport } from "./AgentBundleDialogs";
import { useBundleUpdate, useBundleUpdateStore } from "./bundleUpdate";

export function BundleFooter() {
  const { chip: state, updating, checking, refresh, startUpdate } = useBundleUpdate();

  if (updating) {
    return (
      <div className="flex shrink-0 items-center gap-2 text-ui text-ink-dim">
        <span>Updating agents…</span>
        <IconButton disabled title="Updating agents…" aria-label="Updating agents">
          <Spinner size="sm" />
        </IconButton>
      </div>
    );
  }

  if (state.kind === "updateAvailable") {
    const to = state.to;
    return (
      <div className="flex shrink-0 items-center gap-2">
        <Badge tone="warn">New agents available</Badge>
        <span className="hidden text-meta text-ink-dim sm:inline">
          {state.from ? `${state.from} → ${to}` : to}
        </span>
        <IconButton
          onClick={startUpdate}
          title={`Update agents to ${to}`}
          aria-label={`Update agents to ${to}`}
        >
          <Icon name="download" className="text-accent" />
        </IconButton>
      </div>
    );
  }

  if (state.kind === "unknown") {
    return (
      <RefreshButton
        onClick={refresh}
        busy={checking}
        label="Check for agent updates"
        title={state.detail}
        className="shrink-0 text-ui"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-ink-dim/50" />
        <span className="whitespace-nowrap">Bundle unknown</span>
      </RefreshButton>
    );
  }

  const detail = [
    state.label,
    state.lastCheckedMs !== null ? `checked ${formatRelativeTime(state.lastCheckedMs)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  if (state.frozen) {
    return (
      <RefreshButton
        onClick={refresh}
        busy={checking}
        label="Check for agent updates"
        title={`Update the hyprlayer CLI for current skills${detail ? ` · ${detail}` : ""}`}
        className="shrink-0 text-ui"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" />
        <span className="whitespace-nowrap">CLI out of date</span>
      </RefreshButton>
    );
  }

  return (
    <RefreshButton
      onClick={refresh}
      busy={checking}
      label="Check for agent updates"
      title={detail ? `Agent bundle ${detail} · click to check for updates` : undefined}
      className="min-w-0 shrink-0 text-ui"
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok" />
      <span className="truncate whitespace-nowrap">
        {state.label ? `Up to date · ${state.label}` : "Up to date"}
      </span>
    </RefreshButton>
  );
}

export function BundleUpdateDialogs() {
  const { report, failure, setReport, setFailure } = useBundleUpdateStore();
  const { startUpdate } = useBundleUpdate();
  return (
    <>
      <AgentUpdateReport report={report} open={report !== null} onClose={() => setReport(null)} />
      <AgentUpdateFailed
        failure={failure}
        open={failure !== null}
        onClose={() => setFailure(null)}
        onRetry={() => {
          setFailure(null);
          startUpdate();
        }}
      />
    </>
  );
}
