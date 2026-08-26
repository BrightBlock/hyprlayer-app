import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";

import { Badge, type BadgeTone, Button, CliOutput, Dialog } from "@/components/ui";
import { commands, type AgentBundleChange } from "@/lib/bindings";
import { errorMessage, unwrap } from "@/lib/ipc";
import { useToast } from "@/lib/toast";

import type { BundleFailure, BundleReport } from "./agents";

const CHANGE_BADGE: Record<AgentBundleChange["change"], { tone: BadgeTone; label: string }> = {
  added: { tone: "ok", label: "New" },
  updated: { tone: "dim", label: "Updated" },
};

export function AgentUpdateReport({
  report,
  open,
  onClose,
}: {
  report: BundleReport | null;
  open: boolean;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const [restoring, setRestoring] = useState<string | null>(null);
  if (!report) return null;
  const anyOverwritten = report.agents.some((c) => c.change !== "added");

  const restore = async (name: string) => {
    setRestoring(name);
    try {
      await unwrap(commands.restoreAgentFromBackup(name));
      await queryClient.invalidateQueries({ queryKey: ["agent-files"] });
      useToast.getState().show("Agents", `“${name}” restored from backup`);
    } catch (e) {
      useToast.getState().show("Restore failed", errorMessage(e), { tone: "bad" });
    } finally {
      setRestoring(null);
    }
  };

  const alsoChanged = [
    report.settingsChanged ? "settings.json" : null,
    report.skillsChanged > 0
      ? `${report.skillsChanged} skill file${report.skillsChanged === 1 ? "" : "s"}`
      : null,
  ].filter(Boolean);

  return (
    <Dialog open={open} onClose={onClose} title="Agents updated" size="lg">
      <div className="flex flex-col gap-3.5">
        {report.to !== null && (
          <p className="font-mono text-xs text-ink-dim">
            {report.from ? `${report.from} → ` : ""}
            {report.to}
          </p>
        )}

        {report.agents.length === 0 && alsoChanged.length === 0 ? (
          <p className="text-sm text-ink-dim">
            Nothing changed — the library already matched the bundle.
          </p>
        ) : (
          <ul className="flex max-h-72 flex-col gap-1 overflow-y-auto">
            {report.agents.map((change) => {
              const badge = CHANGE_BADGE[change.change];
              return (
                <li
                  key={change.path}
                  className="flex items-center gap-2 rounded-md border border-edge/60 px-2.5 py-1.5 text-sm"
                >
                  <span className="min-w-0 flex-1 truncate text-ink" title={change.path}>
                    {change.name}
                  </span>
                  <Badge tone={badge.tone}>{badge.label}</Badge>
                  {change.change !== "added" && (
                    <Button
                      className="shrink-0"
                      busy={restoring === change.name}
                      onClick={() => void restore(change.name)}
                    >
                      Restore mine
                    </Button>
                  )}
                </li>
              );
            })}
          </ul>
        )}

        {alsoChanged.length > 0 && (
          <p className="text-sm text-ink-dim">
            The update also replaced {alsoChanged.join(" and ")}.
          </p>
        )}

        {(anyOverwritten || alsoChanged.length > 0) && (
          <p className="rounded-md border border-warn/30 bg-warn/10 px-2.5 py-2 text-xs text-warn">
            The previous contents of everything above are saved at{" "}
            <code className="font-mono break-all">{report.backupPath}</code>. If any of it replaced
            an edit of your own, restore it now — the next update overwrites that copy.
          </p>
        )}

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
          . This is usually a network problem, or an AI tool that still needs configuring.
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
