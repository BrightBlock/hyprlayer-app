import { useState, type ReactNode } from "react";

import {
  Button,
  DropdownSelect,
  ErrorNote,
  SegmentedControl,
  Spinner,
  TextInput,
  type DropdownOption,
} from "@/components/ui";
import type { ShellTab } from "@/features/session/workstreamTabs";
import { goalBudgetUnit, type GoalSpec } from "@/lib/domain/goals";
import { HARNESS_SHORT_LABEL } from "@/lib/domain/harnessing";
import { claudeModels } from "@/lib/domain/models";
import { permissionModeOption, stagePermissionMode } from "@/lib/domain/permissions";
import { plural } from "@/lib/format";
import { openGoalSheet, type GoalSheetRequest } from "@/lib/goalSheet";
import { useBootstrap, useClaudeProbe, useDeriveGoal } from "@/lib/queries";

import { approvePlanUntilMet, planBuildAgent } from "./planDocActions";
import type { PlanTracker } from "./planTracker";

type Scope = "all" | "range" | "one";
type LimitEdits = { iterations?: string; budget?: string; minutes?: string };

function inScope(labels: string[], scope: Scope, from: string, to: string, one: string): string[] {
  if (scope === "all") return [];
  if (scope === "one") return [one];
  const [a, b] = [labels.indexOf(from), labels.indexOf(to)];
  return labels.slice(Math.min(a, b), Math.max(a, b) + 1);
}

function withLimits(spec: GoalSpec, edits: LimitEdits): GoalSpec {
  const usd = goalBudgetUnit(spec) === "usd";
  const budget = edits.budget === undefined ? null : Number(edits.budget);
  return {
    ...spec,
    limits: {
      maxIterations: edits.iterations === undefined ? spec.limits.maxIterations : Number(edits.iterations),
      maxCostUsd: usd ? (budget ?? spec.limits.maxCostUsd) : null,
      maxTokens: usd ? null : (budget ?? spec.limits.maxTokens),
      maxActiveSecs:
        edits.minutes === undefined ? spec.limits.maxActiveSecs : Math.round(Number(edits.minutes) * 60),
    },
  };
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <span className="pt-[3px] text-ui text-ink-dim">{label}</span>
      <div className="min-w-0">{children}</div>
    </>
  );
}

function LimitInput({
  label,
  value,
  onChange,
  suffix,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  suffix?: string;
}) {
  return (
    <label className="flex items-center gap-1.5 text-ui text-ink-dim">
      {label}
      <TextInput
        type="number"
        min={0}
        aria-label={label}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-6 w-16 px-1.5 py-0 text-meta"
      />
      {suffix}
    </label>
  );
}

function PhasePicker({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: DropdownOption[];
  onChange: (value: string) => void;
}) {
  return (
    <span className="w-24">
      <DropdownSelect
        aria-label={label}
        value={value}
        options={options}
        onChange={onChange}
        className="h-6 py-0 text-ui"
      />
    </span>
  );
}

// The draft footer's "Build until met": a derived goal, approved and started in one click.
export function BuildUntilMet({
  workstreamId,
  planPath,
  tracker,
  shell,
  onError,
}: {
  workstreamId: string;
  planPath: string;
  tracker: PlanTracker;
  shell: ShellTab;
  onError: (error: unknown) => void;
}) {
  const settings = useBootstrap().data?.settings;
  const catalog = claudeModels(useClaudeProbe().data?.models);
  const { launch, permissionKey } = planBuildAgent(settings, shell, catalog);

  const labels = tracker.phases.map((p) => p.label).filter(Boolean);
  const first = tracker.phases.find((p) => p.state !== "done")?.label || labels[0] || "";
  const [scope, setScope] = useState<Scope>("all");
  const [from, setFrom] = useState(first);
  const [to, setTo] = useState(labels[labels.length - 1] ?? "");
  const [one, setOne] = useState(first);
  const [edits, setEdits] = useState<LimitEdits>({});
  const [starting, setStarting] = useState(false);

  const phases = inScope(labels, scope, from, to, one);
  const derive = useDeriveGoal({ workstreamId, source: "plan", harness: launch.harness, planPath, phases });
  const spec = derive.data ? withLimits(derive.data, edits) : null;
  const commands = spec?.checks.flatMap((c) => (c.kind === "command" ? [c.command] : [])) ?? [];
  const manual = tracker.phases
    .filter((p) => phases.length === 0 || phases.includes(p.label))
    .flatMap((p) => p.tasks.filter((t) => t.check && t.group === "manual").map((t) => t.title));

  const sheetRequest = (s: GoalSpec): GoalSheetRequest => ({
    mode: "start",
    workstreamId,
    source: s.source,
    planPath,
    phases,
    manual,
    spec: s,
    target: { newSession: { stage: "implement", attachedDocs: [planPath] } },
  });

  const phaseOptions = labels.map((l) => ({ value: l, label: `Phase ${l}` }));
  const usd = spec ? goalBudgetUnit(spec) === "usd" : true;
  const agent = [
    HARNESS_SHORT_LABEL[launch.harness],
    launch.model,
    launch.harness === "opencode" ? null : permissionModeOption(permissionKey ?? stagePermissionMode("implement")).label,
  ].filter(Boolean);

  return (
    <div data-testid="build-until-met" className="w-full space-y-2.5">
      <div className="grid grid-cols-[72px_minmax(0,1fr)] gap-x-3 gap-y-2.5">
        <Row label="Scope">
          <div className="flex flex-wrap items-center gap-2">
            <SegmentedControl
              aria-label="Scope"
              value={scope}
              options={[
                { value: "all", label: "All phases" },
                { value: "range", label: "Phases" },
                { value: "one", label: "Phase" },
              ]}
              onChange={setScope}
            />
            {scope === "range" && (
              <span className="flex items-center gap-1 text-ui text-ink-dim">
                <PhasePicker label="From phase" value={from} options={phaseOptions} onChange={setFrom} />
                –
                <PhasePicker label="To phase" value={to} options={phaseOptions} onChange={setTo} />
              </span>
            )}
            {scope === "one" && (
              <PhasePicker label="Phase in scope" value={one} options={phaseOptions} onChange={setOne} />
            )}
          </div>
        </Row>

        <Row label="Met when">
          <p className="text-ui text-ink-2">
            Every in-scope automated command passes and every non-manual box is ticked
            {commands.length > 0 ? ":" : "."}
          </p>
          {derive.error != null ? (
            <ErrorNote error={derive.error} />
          ) : !spec ? (
            <div className="py-1">
              <Spinner size="xs" />
            </div>
          ) : (
            <>
              {commands.length > 0 && (
                <ul className="mt-1 max-h-32 space-y-0.5 overflow-y-auto">
                  {commands.map((command) => (
                    <li key={command} className="selectable break-all font-mono text-meta text-ink">
                      {command}
                    </li>
                  ))}
                </ul>
              )}
              <Button size="sm" className="mt-1.5" onClick={() => openGoalSheet(sheetRequest(spec))}>
                Review {plural(commands.length, "command")}
              </Button>
            </>
          )}
        </Row>

        {manual.length > 0 && (
          <p className="col-span-2 text-ui text-ink-dim">
            Manual checks stay with you: {manual.map((m) => `"${m}"`).join(", ")}
          </p>
        )}

        <Row label="Limits">
          {spec && (
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
              <LimitInput
                label="Iterations"
                value={edits.iterations ?? String(spec.limits.maxIterations)}
                onChange={(iterations) => setEdits((e) => ({ ...e, iterations }))}
              />
              <LimitInput
                label={usd ? "Budget $" : "Tokens"}
                value={edits.budget ?? String((usd ? spec.limits.maxCostUsd : spec.limits.maxTokens) ?? "")}
                onChange={(budget) => setEdits((e) => ({ ...e, budget }))}
              />
              <LimitInput
                label="Active time"
                suffix="min"
                value={edits.minutes ?? String(Math.round((spec.limits.maxActiveSecs / 60) * 100) / 100)}
                onChange={(minutes) => setEdits((e) => ({ ...e, minutes }))}
              />
            </div>
          )}
        </Row>

        <Row label="Agent">
          <div className="flex items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-ui text-ink-2">{agent.join(" · ")}</span>
            <Button size="sm" disabled={!spec} onClick={() => spec && openGoalSheet(sheetRequest(spec))}>
              Change
            </Button>
          </div>
        </Row>
      </div>


      <div className="flex justify-end">
        <Button
          variant="primary"
          size="sm"
          data-testid="approve-until-met"
          className="h-[26px] text-ui-sm font-semibold"
          busy={starting}
          disabled={!spec}
          onClick={() => {
            if (!spec) return;
            setStarting(true);
            approvePlanUntilMet(workstreamId, planPath, shell, spec)
              .catch(onError)
              .finally(() => setStarting(false));
          }}
        >
          Approve &amp; build until met
        </Button>
      </div>
    </div>
  );
}
