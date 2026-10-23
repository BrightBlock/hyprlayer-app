import { useState } from "react";

import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";
import { selectAgentTab } from "@/features/search/selectAgentTab";
import type { Goal, GoalCheck, GoalCheckOutcome, GoalCheckResult, GoalIteration } from "@/lib/bindings";
import { plural } from "@/lib/format";
import { STEP_ROW, StepChevron } from "./stepRow";

const TAIL_LINES = 6;

function CheckTail({ tail }: { tail: string }) {
  const [all, setAll] = useState(false);
  const lines = tail.trimEnd().split("\n");
  const clipped = lines.length > TAIL_LINES;
  return (
    <div className="mt-1 pl-5">
      <pre className="selectable max-h-96 overflow-auto rounded bg-surface/60 p-2 text-meta text-ink-dim">
        {all || !clipped ? tail.trimEnd() : lines.slice(-TAIL_LINES).join("\n")}
      </pre>
      {clipped && (
        <button
          type="button"
          onClick={() => setAll((v) => !v)}
          className="mt-1 rounded px-1 py-0.5 text-ui-sm text-ink-dim hover:bg-edge/60 hover:text-ink focus-visible:bg-edge/60 focus-visible:text-ink"
        >
          {all ? "Show less" : "Show all"}
        </button>
      )}
    </div>
  );
}

const CHECK_KIND_LABEL = { plan_boxes: "Plan boxes", evaluator: "Evaluator", validation: "Validation" } as const;

// The result carries no counts: the server writes one finding per tail line, failing checks prefixed.
function validationCounts(tail: string): string | null {
  const findings = tail.split("\n").filter((line) => line.trim());
  const failing = findings.filter((line) => line.startsWith("Failed check: ")).length;
  const issues = findings.length - failing;
  const parts = [issues > 0 && plural(issues, "issue"), failing > 0 && plural(failing, "failing check")];
  return parts.filter(Boolean).join(" · ") || null;
}

function CheckResultRow({ result, workstreamId }: { result: GoalCheckResult; workstreamId: string }) {
  const [open, setOpen] = useState(false);
  const pass = result.outcome === "pass";
  const validation = result.kind === "validation";
  const counts = validation ? validationCounts(result.outputTail) : null;
  const detail =
    result.exitCode !== null ? `exit ${result.exitCode}` : result.outcome === "error" ? "error" : counts;
  const tail = !pass && result.outputTail ? result.outputTail : null;
  const validateSessionId = validation ? (result.sessionId ?? null) : null;
  return (
    <div data-testid={validation ? "goal-check-validation" : undefined}>
      <div className="flex min-w-0 items-center gap-2">
        <Icon
          name={pass ? "check" : "failed"}
          size="xs"
          className={pass ? "shrink-0 text-ok" : "shrink-0 text-bad-ink"}
        />
        <span className="min-w-0 truncate text-ink" title={result.label}>
          {result.label}
        </span>
        {detail && <span className="shrink-0 font-mono text-meta text-ink-dim">{detail}</span>}
        {(validateSessionId || tail) && (
          <span className="ml-auto flex shrink-0 items-center gap-1">
            {validateSessionId && (
              <button
                type="button"
                data-testid="goal-check-open-validate"
                onClick={() => void selectAgentTab(workstreamId, validateSessionId, { frozen: true })}
                className="rounded px-1 py-0.5 text-ui-sm text-ink-dim hover:bg-edge/60 hover:text-ink focus-visible:bg-edge/60 focus-visible:text-ink"
              >
                Open
              </button>
            )}
            {tail && (
              <button
                type="button"
                onClick={() => setOpen((v) => !v)}
                aria-expanded={open}
                className="rounded px-1 py-0.5 text-ui-sm text-ink-dim hover:bg-edge/60 hover:text-ink focus-visible:bg-edge/60 focus-visible:text-ink"
              >
                {open ? "Hide output" : "Output"}
              </button>
            )}
          </span>
        )}
      </div>
      {result.reason && (
        <p className="selectable mt-0.5 pl-5 text-ink-dim">
          {result.kind === "evaluator" ? `“${result.reason}”` : result.reason}
        </p>
      )}
      {open && tail && <CheckTail tail={tail} />}
    </div>
  );
}

function checkLabel(check: GoalCheck): string {
  return check.kind === "command" ? (check.label ?? check.command) : CHECK_KIND_LABEL[check.kind];
}

// Layers run in order and stop at the first failure, so the unrun ones are the tail.
function notRun(goal: Goal, iteration: GoalIteration): string[] {
  const ranKinds = new Set(iteration.results.map((r) => r.kind));
  const commandsRun = iteration.results.filter((r) => r.kind === "command").length;
  return [
    ...goal.checks.filter((c) => c.kind === "command").slice(commandsRun),
    ...goal.checks.filter((c) => c.kind !== "command" && !ranKinds.has(c.kind)),
  ].map(checkLabel);
}

type PhaseGroup = { phase: string | null; results: GoalCheckResult[] };

function byPhase(results: GoalCheckResult[]): PhaseGroup[] {
  const groups: PhaseGroup[] = [];
  for (const result of results) {
    const phase = result.phase ?? null;
    const group = groups.find((g) => g.phase === phase);
    if (group) group.results.push(result);
    else groups.push({ phase, results: [result] });
  }
  return groups;
}

/** Failures first, under their phase; the passes fold into one line. */
export function GoalCheckResults({ goal, iteration }: { goal: Goal; iteration: GoalIteration }) {
  const [showPassed, setShowPassed] = useState(false);
  const groups = byPhase(iteration.results);
  const phased = groups.some((g) => g.phase !== null);
  const passed = iteration.results.filter((r) => r.outcome === "pass");
  const skipped = notRun(goal, iteration);
  return (
    <div className="space-y-3 text-ui">
      {groups.map((group) => {
        const failed = group.results.filter((r) => r.outcome !== "pass");
        if (failed.length === 0) return null;
        return (
          <section key={group.phase ?? ""}>
            {phased && (
              <div className="mb-1 flex items-baseline gap-2">
                <span className="font-semibold text-ink">{group.phase ? `Phase ${group.phase}` : "Whole plan"}</span>
                <span className="text-ink-dim">
                  {failed.length} of {group.results.length} failed
                </span>
              </div>
            )}
            <div className="space-y-1.5">
              {failed.map((result, i) => (
                <CheckResultRow key={`${result.label}:${i}`} result={result} workstreamId={goal.workstreamId} />
              ))}
            </div>
          </section>
        );
      })}
      {passed.length > 0 && (
        <div>
          <button
            type="button"
            onClick={() => setShowPassed((v) => !v)}
            aria-expanded={showPassed}
            className={STEP_ROW}
          >
            <Icon name="check" size="xs" className="shrink-0 text-ok" />
            <span className="text-ink-dim">{passed.length} passed</span>
            <StepChevron open={showPassed} />
          </button>
          {showPassed && (
            <div className="mt-1 space-y-1.5">
              {passed.map((result, i) => (
                <CheckResultRow key={`${result.label}:${i}`} result={result} workstreamId={goal.workstreamId} />
              ))}
            </div>
          )}
        </div>
      )}
      {skipped.length > 0 && (
        <div className="flex min-w-0 items-center gap-2 text-ink-dim">
          <span className="w-3 shrink-0 text-center" aria-hidden>
            –
          </span>
          <span className="min-w-0 truncate">{skipped.join(", ")}</span>
          <span className="shrink-0">not run</span>
        </div>
      )}
    </div>
  );
}

const TICK: Record<GoalCheckOutcome, string> = { pass: "bg-track", fail: "bg-bad", error: "bg-warn" };

/** One tick per result, a gap between phases, so a failure reads at a glance. */
export function CheckTicks({ results }: { results: GoalCheckResult[] }) {
  const failed = results.filter((r) => r.outcome !== "pass").length;
  return (
    <span
      role="img"
      aria-label={`${failed} of ${results.length} checks failed`}
      className="flex min-w-0 items-center gap-0.5 overflow-hidden"
    >
      {results.map((result, i) => (
        <span
          key={i}
          title={`${result.label}: ${result.outcome}`}
          className={cx(
            "h-3 w-1 shrink-0 rounded-[1px]",
            TICK[result.outcome],
            i > 0 && (result.phase ?? null) !== (results[i - 1].phase ?? null) && "ml-1",
          )}
        />
      ))}
    </span>
  );
}
