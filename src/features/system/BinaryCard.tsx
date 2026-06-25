import { open } from "@tauri-apps/plugin-dialog";

import type { BinaryStatus } from "../../lib/bindings";
import { errorMessage } from "../../lib/ipc";
import { useSetBinaryOverride } from "../../lib/queries";
import { Badge, Button } from "../../components/ui";

const DESCRIPTIONS: Record<BinaryStatus["kind"], string> = {
  hyprlayer: "Manages thoughts storage, agent skills, and project mappings.",
  claude: "Runs the AI agent sessions for research, planning, and implementation.",
  opencode: "Optional — the OpenCode harness for agent sessions across many providers.",
  git: "Required for worktrees, commits, and thoughts syncing.",
  gh: "Optional — used to create and track pull requests.",
};

export function BinaryCard({ status }: { status: BinaryStatus }) {
  const override = useSetBinaryOverride();

  const tone = status.state.state === "ok" ? "ok" : status.optional ? "warn" : "bad";
  const label =
    status.state.state === "ok" ? "Found" : status.state.state === "tooOld" ? "Too old" : "Missing";

  async function pickBinary() {
    const picked = await open({ multiple: false, directory: false, title: `Locate ${status.name}` });
    if (typeof picked === "string") {
      override.mutate({ kind: status.kind, path: picked });
    }
  }

  return (
    <div className="flex items-start justify-between gap-4 rounded-md border border-edge bg-panel-2 p-3">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-mono text-sm text-ink">{status.name}</span>
          <Badge tone={tone}>{label}</Badge>
          {status.version && <span className="text-xs text-ink-dim">v{status.version}</span>}
          {status.overridden && <Badge tone="accent">manual path</Badge>}
        </div>
        <p className="mt-1 text-xs text-ink-dim">{DESCRIPTIONS[status.kind]}</p>
        {status.path && (
          <p className="selectable mt-1 truncate font-mono text-xs text-ink-dim" title={status.path}>
            {status.path}
          </p>
        )}
        {status.state.state === "tooOld" && (
          <p className="mt-1 text-xs text-warn">Minimum supported version: {status.state.min}</p>
        )}
        {override.isError && (
          <p className="selectable mt-1 text-xs text-bad">
            Couldn't save the override: {errorMessage(override.error)}
          </p>
        )}
      </div>
      <div className="flex shrink-0 gap-2">
        <Button onClick={() => void pickBinary()} busy={override.isPending}>
          Locate…
        </Button>
        {status.overridden && (
          <Button onClick={() => override.mutate({ kind: status.kind, path: null })}>Reset</Button>
        )}
      </div>
    </div>
  );
}
