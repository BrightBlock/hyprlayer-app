/**
 * The two dialogs an `updateAgentBundle` run can end in: a report naming
 * what changed, with a way back for anything of the user's own the update
 * replaced, or a failure carrying the CLI's own error and a path to debug
 * it. `AgentLibrary` owns the mutation and the SHA bookkeeping; these render
 * the result.
 */
import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";

import { Badge, type BadgeTone, Button, Dialog } from "@/components/ui";
import { commands, type AgentBundleChange } from "@/lib/bindings";
import { errorMessage, unwrap } from "@/lib/ipc";
import { shortSha } from "@/lib/format";
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
  // Which row's restore is in flight, by name — at most one at a time, a
  // report lists each agent once.
  const [restoring, setRestoring] = useState<string | null>(null);
  if (!report) return null;
  // Gated on "anything was overwritten", the same predicate the restore
  // buttons use: `diff_against_backup` cannot tell an upstream change from a
  // reverted local edit and reports both as `updated` (see its doc comment),
  // so keying this off a would-be `clobbered` label would hide it from
  // exactly the user who just lost an edit.
  const anyOverwritten = report.agents.some((c) => c.change !== "added");

  const restore = async (name: string) => {
    setRestoring(name);
    try {
      await unwrap(commands.restoreAgentFromBackup(name));
      // The list behind this dialog, every row's dot, and any open editor
      // draft are all served from this cache. Without the invalidation a
      // restore is invisible, and saving the still-stale draft writes the
      // bundle's copy straight back over the file just recovered.
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
    <Dialog open={open} onClose={onClose} title="Agents updated" className="max-w-lg">
      <div className="flex flex-col gap-3.5">
        {report.toSha !== null && (
          <p className="font-mono text-xs text-ink-dim">
            {report.fromSha ? `${shortSha(report.fromSha)} → ` : ""}
            {shortSha(report.toSha)}
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
                  {/* Offered for anything that was not `added`, because that
                      is exactly the set whose previous contents might have
                      been the user's own. Keying it on a per-file "this was
                      yours" signal would be better, and the CLI does not
                      emit one. */}
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

        {/* `settings.json` gets its own line rather than a row in the list
            above: it is not an agent, it has no restore button (this dialog
            restores agents), and it is the one file in the bundle whose
            previous contents were entirely the user's — permissions, env,
            hooks, statusLine. Saying so is the difference between a silent
            overwrite and a recoverable one. */}
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
    <Dialog open={open} onClose={onClose} title="Agent update failed" className="max-w-lg">
      <div className="flex flex-col gap-3.5">
        {/* "Most likely unchanged", not "unchanged": the CLI stages the whole
            bundle and refuses to touch the destination unless the download
            looks complete, so a failed download changes nothing — but the
            300s timeout kills the CLI outright, and a kill landing during its
            sync leaves a partly-written tree that this dialog cannot detect.
            Claiming certainty we do not have is worse than the hedge. */}
        <p className="text-sm text-ink-dim">
          Your agents are most likely unchanged
          {failure.stillOnSha ? (
            <>
              , still on <span className="font-mono">{shortSha(failure.stillOnSha)}</span>
            </>
          ) : null}
          . This is usually a network problem, or an AI tool that still needs configuring.
        </p>

        {/* The CLI's own words, folded away rather than made the headline: the
            two things a user can actually DO about it are the buttons below,
            and neither needs them to read a stack of tool output. Kept
            available (not dropped) because for a network or rate-limit
            failure it is the only signal that says which of the two it was. */}
        <details className="group">
          <summary className="cursor-pointer select-none text-xs text-ink-dim hover:text-ink">
            Technical details
          </summary>
          <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-edge bg-panel-2 p-2.5 font-mono text-xs text-ink-dim">
            {failure.message}
          </pre>
        </details>

        <div className="flex justify-end gap-2">
          {/* Straight to the AI tool card, which owns Reconfigure. `useNavigate`
              is the router LIBRARY, not the app's router singleton, so this
              stays inside ADR-0003's import direction (same as
              features/search/SearchPalette.tsx). */}
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
