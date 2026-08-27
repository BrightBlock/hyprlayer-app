import { useNavigate } from "@tanstack/react-router";

import { Button, CliOutput, Dialog } from "@/components/ui";

import type { BundleFailure, BundleReport } from "./agents";

export function AgentUpdateReport({
  report,
  open,
  onClose,
}: {
  report: BundleReport | null;
  open: boolean;
  onClose: () => void;
}) {
  if (!report) return null;
  const destination = report.to ?? "the requested bundle";
  const sourceMessage =
    report.source === "releaseAsset"
      ? `Claude Code and Codex now point to ${destination}.`
      : report.source === "fallback"
        ? "Claude Code and Codex were linked to the repository fallback bundle."
        : "Claude Code and Codex were relinked, but the CLI did not identify the bundle source.";
  const caution = report.source !== "releaseAsset";

  return (
    <Dialog open={open} onClose={onClose} title="Agents updated" size="lg">
      <div className="flex flex-col gap-3.5">
        {report.to !== null && (
          <p className="font-mono text-xs text-ink-dim">
            {report.from ? `${report.from} → ` : ""}
            {report.to}
          </p>
        )}

        <p
          className={`rounded-md border px-2.5 py-2 text-sm ${
            caution
              ? "border-warn/30 bg-warn/10 text-warn"
              : "border-ok/30 bg-ok/10 text-ok"
          }`}
        >
          {sourceMessage}
        </p>

        {caution && <CliOutput text={report.stdout} />}

        <div className="flex justify-end">
          <Button onClick={onClose}>Done</Button>
        </div>
      </div>
    </Dialog>
  );
}

export function AgentUpdateFailed({
  failure,
  open,
  onClose,
  onRetry,
}: {
  failure: BundleFailure | null;
  open: boolean;
  onClose: () => void;
  onRetry: () => void;
}) {
  const navigate = useNavigate();
  if (!failure) return null;

  return (
    <Dialog open={open} onClose={onClose} title="Agent update failed" size="lg">
      <div className="flex flex-col gap-3.5">
        <p className="text-sm text-ink-dim">
          Your agents are most likely unchanged
          {failure.stillOn ? (
            <>
              , still on <span className="font-mono">{failure.stillOn}</span>
            </>
          ) : null}
          . This is usually a network problem or an older CLI that cannot provision the bundle
          pair.
        </p>

        <details className="group">
          <summary className="cursor-pointer select-none text-xs text-ink-dim hover:text-ink">
            Technical details
          </summary>
          <CliOutput text={failure.message} />
        </details>

        <div className="flex justify-end gap-2">
          <Button
            onClick={() => {
              onClose();
              void navigate({ to: "/settings", search: { section: "general" } });
            }}
          >
            Open AI settings
          </Button>
          <Button variant="primary" onClick={onRetry}>
            Try again
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
