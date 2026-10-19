import { useState, type ReactNode } from "react";

import type {
  AppSettings,
  BinaryStatus,
  Goal,
  GoalCheck,
  GoalEndReason,
  GoalSpec,
  GoalTarget,
  Harness,
  Session,
  Stage,
} from "@/lib/bindings";
import { goalBudgetUnit } from "@/lib/domain/goals";
import { HARNESSES, HARNESS_SHORT_LABEL, newChatLaunchArgs } from "@/lib/domain/harnessing";
import { modelOption, visibleModels } from "@/lib/domain/models";
import { PERMISSION_MODES, permissionModeOption } from "@/lib/domain/permissions";
import { stageLabel } from "@/lib/domain/stages";
import {
  closeGoalSheet,
  notifyGoalStarted,
  useGoalSheet,
  type GoalSheetRequest,
} from "@/lib/goalSheet";
import {
  useBootstrap,
  useDeriveGoal,
  useGoal,
  useResumeGoal,
  useStartGoal,
  useUpdateGoal,
  useWorkstreamSessions,
} from "@/lib/queries";
import { Icon } from "./Icon";
import {
  Button,
  cx,
  Dialog,
  DropdownSelect,
  ErrorNote,
  SegmentedControl,
  Spinner,
  TextArea,
  TextInput,
  Toggle,
  type DropdownOption,
} from "./ui";

const GATE_RULE = "A goal needs at least one command, evaluator or validation check.";
const GOAL_STAGES: readonly Stage[] = ["general", "research", "implement"];
const COMMAND_TIMEOUT_SECS = 600;
const MAX_ITERATIONS = 200;
const MIN_ACTIVE_SECS = 60;
const MAX_ACTIVE_SECS = 172_800;
// The server's own defaults (crates/hyprlayer-core/src/settings.rs) for a profile that omits them.
const DEFAULT_COST_USD = 10;
const DEFAULT_TOKENS = 2_000_000;

type Unit = "usd" | "tokens";
type LimitField = "iterations" | "budget" | "time";
type CheckRow = { id: number; check: GoalCheck; on: boolean; added: boolean };
type Form = {
  condition: string;
  workPrompt: string;
  rows: CheckRow[];
  iterations: string;
  costUsd: string;
  tokens: string;
  activeMinutes: string;
  runIn: "this" | "new";
  harness: Harness;
  model: string | null;
  permissionKey: string;
};

const requestKeys = new WeakMap<GoalSheetRequest, number>();
let lastRequestKey = 0;

// A fresh request remounts the sheet, so a second open never inherits the first one's edits.
function requestKey(request: GoalSheetRequest): number {
  let key = requestKeys.get(request);
  if (key === undefined) {
    key = ++lastRequestKey;
    requestKeys.set(request, key);
  }
  return key;
}

export function GoalSheetHost() {
  const request = useGoalSheet((s) => s.request);
  return request ? <GoalSheet key={requestKey(request)} request={request} /> : null;
}

function specOf(goal: Goal): GoalSpec {
  const { condition, workPrompt, source, sourceRef, sourcePhase, checks, limits } = goal;
  return { condition, workPrompt, source, sourceRef, sourcePhase, checks, limits };
}

function blankSpec(request: GoalSheetRequest, settings: AppSettings | undefined): GoalSpec {
  return {
    condition: "",
    workPrompt: "",
    source: request.source,
    sourceRef: null,
    sourcePhase: null,
    checks: [{ kind: "evaluator", model: null }],
    limits: {
      maxIterations: settings?.goals?.maxIterations ?? 10,
      maxCostUsd: null,
      maxTokens: null,
      maxActiveSecs: settings?.goals?.maxActiveSecs ?? 3600,
    },
  };
}

function unitOf(harness: Harness): Unit {
  return harness === "codex" ? "tokens" : "usd";
}

function evaluatorBinary(model: string | null | undefined, settings: AppSettings | undefined): Harness {
  const tag = model?.trim() || settings?.goals?.evaluatorModel?.trim();
  if (!tag) return settings?.providers?.defaultHarness === "codex" ? "codex" : "claude";
  if (tag.startsWith("codex:")) return "codex";
  if (tag.startsWith("opencode:")) return "opencode";
  return "claude";
}

function binaryProblem(binaries: BinaryStatus[] | undefined, kind: Harness): string | null {
  const state = binaries?.find((b) => b.kind === kind)?.state;
  if (state?.state === "missing") return `${kind} is not installed`;
  if (state?.state === "tooOld") return `${kind} needs ${state.min} or newer`;
  return null;
}

function hitLimit(reason: GoalEndReason | null | undefined): LimitField | null {
  if (reason === "iteration_limit") return "iterations";
  if (reason === "budget_limit" || reason === "token_limit") return "budget";
  if (reason === "time_limit") return "time";
  return null;
}

function claudeModelOptions(settings: AppSettings | undefined): DropdownOption[] {
  return visibleModels(settings?.providers?.claude?.visibleModels)
    .filter((m) => !m.disabled)
    .map((m) => ({ value: m.key, label: m.hint ? `${m.label} (${m.hint})` : m.label }));
}

function modelChoices(settings: AppSettings | undefined, harness: Harness): DropdownOption[] {
  const providers = settings?.providers;
  if (harness === "claude") return claudeModelOptions(settings);
  const fallback = newChatLaunchArgs(settings, { harness }).model;
  const ids =
    harness === "codex"
      ? (providers?.codex?.selectedModels ?? [])
      : [
          ...(providers?.opencode?.selectedModels ?? []),
          ...(providers?.ollama?.selectedModels ?? []).map((id) => `ollama/${id}`),
        ];
  return [...new Set([...(fallback ? [fallback] : []), ...ids])].map((id) => ({ value: id, label: id }));
}

function defaultModel(settings: AppSettings | undefined, harness: Harness): string | null {
  if (harness === "claude") return modelOption(settings?.providers?.claude?.defaultModel ?? undefined).key;
  return newChatLaunchArgs(settings, { harness }).model;
}

function newSessionLaunch(settings: AppSettings | undefined, form: Form) {
  return newChatLaunchArgs(settings, {
    harness: form.harness,
    permissionMode: form.permissionKey,
    ...(form.harness === "claude" && form.model ? { model: form.model } : {}),
    ...(form.harness === "codex" && form.model ? { codexModel: form.model } : {}),
    ...(form.harness === "opencode" && form.model ? { opencodeModel: form.model } : {}),
  });
}

function initialForm(
  spec: GoalSpec,
  request: GoalSheetRequest,
  settings: AppSettings | undefined,
): Form {
  const rows = spec.checks.map((check, id) => ({ id, check, on: true, added: false }));
  if (!spec.checks.some((c) => c.kind === "evaluator")) {
    rows.push({ id: rows.length, check: { kind: "evaluator", model: null }, on: false, added: false });
  }
  if (request.withValidation && request.planPath && !spec.checks.some((c) => c.kind === "validation")) {
    rows.push({ id: rows.length, check: { kind: "validation", planPath: request.planPath }, on: true, added: false });
  }
  const launch = newChatLaunchArgs(settings, null);
  const defaults = settings?.goals;
  return {
    condition: spec.condition,
    workPrompt: spec.workPrompt,
    rows,
    iterations: String(spec.limits.maxIterations),
    costUsd: String(spec.limits.maxCostUsd ?? defaults?.maxCostUsd ?? DEFAULT_COST_USD),
    tokens: String(spec.limits.maxTokens ?? defaults?.maxTokens ?? DEFAULT_TOKENS),
    activeMinutes: String(Math.round((spec.limits.maxActiveSecs / 60) * 100) / 100),
    runIn: "sessionId" in request.target ? "this" : "new",
    harness: launch.harness,
    model: defaultModel(settings, launch.harness),
    permissionKey: PERMISSION_MODES.find((p) => p.flag === launch.permissionMode)?.key ?? "auto",
  };
}

function GoalSheet({ request }: { request: GoalSheetRequest }) {
  const bootstrap = useBootstrap();
  const settings = bootstrap.data?.settings;
  const sessionId = "sessionId" in request.target ? request.target.sessionId : null;
  const sessions = useWorkstreamSessions(request.workstreamId, sessionId !== null);
  const session = sessions.data?.find((s) => s.id === sessionId) ?? null;
  const goalQuery = useGoal(request.mode === "start" ? null : (request.goalId ?? null));
  const goal = goalQuery.data?.goal ?? null;

  const needsDerive = request.mode === "start" && !request.spec;
  const blank = needsDerive && request.source === "composer" && !request.text?.trim();
  const deriveReady = !bootstrap.isLoading && !(sessionId !== null && sessions.isLoading);
  const derive = useDeriveGoal(
    needsDerive && !blank && deriveReady
      ? {
          workstreamId: request.workstreamId,
          source: request.source,
          harness: session?.harness ?? (sessionId ? null : newChatLaunchArgs(settings, null).harness),
          text: request.text ?? null,
          planPath: request.planPath ?? null,
          phases: request.phases ?? [],
          researchPath: request.researchPath ?? null,
          findings: request.findings ?? [],
        }
      : null,
  );

  const spec =
    request.mode === "start"
      ? (request.spec ?? (blank ? blankSpec(request, settings) : (derive.data ?? null)))
      : goal
        ? specOf(goal)
        : null;
  const loadError = request.mode === "start" ? derive.error : goalQuery.error;

  const [form, setForm] = useState<Form | null>(() =>
    spec && !bootstrap.isLoading ? initialForm(spec, request, settings) : null,
  );
  if (form === null && spec && !bootstrap.isLoading) setForm(initialForm(spec, request, settings));

  return (
    <GoalForm
      request={request}
      spec={form ? spec : null}
      loadError={loadError}
      goal={goal}
      session={session}
      settings={settings}
      binaries={bootstrap.data?.binaries}
      form={form}
      setForm={(next) => setForm((f) => (f ? next(f) : f))}
    />
  );
}

function GoalForm({
  request,
  spec,
  loadError,
  goal,
  session,
  settings,
  binaries,
  form,
  setForm,
}: {
  request: GoalSheetRequest;
  spec: GoalSpec | null;
  loadError: unknown;
  goal: Goal | null;
  session: Session | null;
  settings: AppSettings | undefined;
  binaries: BinaryStatus[] | undefined;
  form: Form | null;
  setForm: (update: (form: Form) => Form) => void;
}) {
  const start = useStartGoal();
  const update = useUpdateGoal();
  const resume = useResumeGoal();
  const [error, setError] = useState<unknown>(null);
  const busy = start.isPending || update.isPending || resume.isPending;
  const title =
    request.mode === "edit" ? "Edit goal" : request.mode === "extend" ? "Extend goal" : "Start a goal";

  // The same Dialog element in both states, so it does not replay its entrance when the spec lands.
  if (!form || !spec) {
    return (
      <Dialog
        open
        onClose={closeGoalSheet}
        title={title}
        ariaLabel={title}
        size="lg"
        closable
        seams
        footer={
          <div className="flex justify-end">
            <Button onClick={closeGoalSheet}>Cancel</Button>
          </div>
        }
      >
        {loadError != null ? (
          <ErrorNote error={loadError} />
        ) : (
          <div className="flex justify-center py-8">
            <Spinner />
          </div>
        )}
      </Dialog>
    );
  }

  const patch = (next: Partial<Form>) => setForm((f) => ({ ...f, ...next }));
  const patchRow = (id: number, next: (row: CheckRow) => CheckRow) =>
    setForm((f) => ({ ...f, rows: f.rows.map((r) => (r.id === id ? next(r) : r)) }));

  const newSession = request.mode === "start" && form.runIn === "new";
  const harness: Harness | null = newSession ? form.harness : (session?.harness ?? null);
  const unit: Unit = goal ? goalBudgetUnit(goal) : harness ? unitOf(harness) : goalBudgetUnit(spec);
  const requestedStage = "newSession" in request.target ? request.target.newSession.stage : null;
  const stage: Stage =
    requestedStage ?? (session && GOAL_STAGES.includes(session.stage) ? session.stage : "general");

  const evaluatorProblem = (check: GoalCheck) =>
    check.kind === "evaluator" ? binaryProblem(binaries, evaluatorBinary(check.model, settings)) : null;
  const included = (row: CheckRow) =>
    row.on &&
    !evaluatorProblem(row.check) &&
    !(row.check.kind === "command" && !row.check.command.trim());
  const checks = form.rows.filter(included).map((r) => r.check);

  const iterations = Number(form.iterations);
  const costUsd = Number(form.costUsd);
  const tokens = Number(form.tokens);
  const activeSecs = Math.round(Number(form.activeMinutes) * 60);
  const limits = {
    maxIterations: iterations,
    maxCostUsd: unit === "usd" ? costUsd : null,
    maxTokens: unit === "tokens" ? tokens : null,
    maxActiveSecs: activeSecs,
  };

  const blocked = (() => {
    if (!checks.some((c) => c.kind === "command" || c.kind === "evaluator" || c.kind === "validation")) {
      return GATE_RULE;
    }
    if (!form.condition.trim()) return "A goal needs a condition.";
    if (!form.workPrompt.trim()) return "A goal needs a work prompt.";
    if (checks.some((c) => c.kind === "command" && !(c.timeoutSecs >= 1))) {
      return "A command check needs a timeout of a second or more.";
    }
    if (!Number.isInteger(iterations) || iterations < 1 || iterations > MAX_ITERATIONS) {
      return `Iterations must be between 1 and ${MAX_ITERATIONS}.`;
    }
    if (unit === "usd" && !(costUsd > 0)) return "The budget must be more than $0.";
    if (unit === "tokens" && !(Number.isInteger(tokens) && tokens > 0)) {
      return "The token cap must be more than 0.";
    }
    if (!(activeSecs >= MIN_ACTIVE_SECS && activeSecs <= MAX_ACTIVE_SECS)) {
      return "Active time must be between 1 minute and 48 hours.";
    }
    if (newSession && !GOAL_STAGES.includes(stage)) {
      return "Goals run in General, Research or Implement sessions.";
    }
    return null;
  })();

  const builtSpec: GoalSpec = {
    ...spec,
    condition: form.condition.trim(),
    workPrompt: form.workPrompt.trim(),
    checks,
    limits,
  };

  const target = (): GoalTarget => {
    if (!newSession && "sessionId" in request.target) {
      return { kind: "session", sessionId: request.target.sessionId };
    }
    const launch = newSessionLaunch(settings, form);
    return {
      kind: "new_session",
      workstreamId: request.workstreamId,
      stage,
      harness: launch.harness,
      model: launch.model,
      effort: launch.effort,
      permissionMode: launch.permissionMode,
      attachedDocs: "newSession" in request.target ? (request.target.newSession.attachedDocs ?? []) : [],
    };
  };

  const submit = async () => {
    if (blocked || busy) return;
    setError(null);
    try {
      if (request.mode === "start") {
        const started = await start.mutateAsync({ target: target(), spec: builtSpec });
        closeGoalSheet();
        const permissionMode = form.permissionKey;
        notifyGoalStarted(
          started,
          !newSession
            ? request
            : "sessionId" in request.target
              ? { ...request, target: { newSession: { stage, permissionMode } } }
              : { ...request, target: { newSession: { ...request.target.newSession, permissionMode } } },
        );
        return;
      }
      const goalId = request.goalId!;
      const { condition, workPrompt } = builtSpec;
      await update.mutateAsync({ goalId, patch: { condition, workPrompt, checks, limits } });
      if (request.mode === "extend") await resume.mutateAsync(goalId);
      closeGoalSheet();
    } catch (e) {
      setError(e);
    }
  };

  const hit = request.mode === "extend" ? hitLimit(goal?.endReason) : null;
  const submitLabel =
    request.mode === "edit" ? "Save" : request.mode === "extend" ? "Extend and resume" : "Start goal";

  return (
    <Dialog
      open
      onClose={closeGoalSheet}
      title={title}
      ariaLabel={title}
      size="lg"
      closable
      seams
      footer={
        <div className="flex items-center justify-end gap-2">
          {request.mode === "edit" && (
            <span className="mr-auto text-ui text-ink-dim">Applies after the current iteration</span>
          )}
          <Button onClick={closeGoalSheet} disabled={busy}>
            Cancel
          </Button>
          <Button
            variant="primary"
            data-testid="goal-sheet-submit"
            busy={busy}
            disabled={blocked !== null}
            title={blocked ?? undefined}
            onClick={() => void submit()}
          >
            {submitLabel}
          </Button>
        </div>
      }
    >
      <div className="space-y-4">
        <SourceLine spec={spec} />

        <Section label="Condition">
          <TextArea
            aria-label="Condition"
            value={form.condition}
            onChange={(e) => patch({ condition: e.target.value })}
          />
        </Section>

        <Section label="Work prompt">
          <TextArea
            aria-label="Work prompt"
            value={form.workPrompt}
            onChange={(e) => patch({ workPrompt: e.target.value })}
          />
        </Section>

        <Section label="Checks">
          <div className="space-y-1">
            {form.rows.map((row) => (
              <CheckLine
                key={row.id}
                row={row}
                problem={evaluatorProblem(row.check)}
                evaluatorModel={settings?.goals?.evaluatorModel ?? null}
                onToggle={(on) => patchRow(row.id, (r) => ({ ...r, on }))}
                onEdit={(check) => patchRow(row.id, (r) => ({ ...r, check }))}
              />
            ))}
            {(request.manual ?? []).map((criterion) => (
              <p key={criterion} className="pl-[46px] text-ui text-ink-dim">
                Not checked (manual): {criterion}
              </p>
            ))}
          </div>
          <Button
            size="sm"
            className="mt-2"
            onClick={() =>
              setForm((f) => ({
                ...f,
                rows: [
                  ...f.rows,
                  {
                    id: Math.max(-1, ...f.rows.map((r) => r.id)) + 1,
                    check: { kind: "command", command: "", timeoutSecs: COMMAND_TIMEOUT_SECS },
                    on: true,
                    added: true,
                  },
                ],
              }))
            }
          >
            <Icon name="add" size="xs" />
            Add command
          </Button>
        </Section>

        <Section label="Limits">
          <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
            <LimitInput
              label="Iterations"
              value={form.iterations}
              hit={hit === "iterations"}
              onChange={(iterations) => patch({ iterations })}
            />
            {unit === "usd" ? (
              <LimitInput
                label="Budget $"
                ariaLabel="Budget"
                step="0.01"
                value={form.costUsd}
                hit={hit === "budget"}
                onChange={(costUsd) => patch({ costUsd })}
              />
            ) : (
              <LimitInput
                label="Tokens"
                value={form.tokens}
                wide
                hit={hit === "budget"}
                onChange={(tokens) => patch({ tokens })}
              />
            )}
            <LimitInput
              label="Active time"
              ariaLabel="Active time (minutes)"
              suffix="min"
              value={form.activeMinutes}
              hit={hit === "time"}
              onChange={(activeMinutes) => patch({ activeMinutes })}
            />
          </div>
        </Section>

        {request.mode === "start" && (
          <RunIn
            request={request}
            session={session}
            settings={settings}
            form={form}
            patch={patch}
          />
        )}

        <p className="text-ui text-ink-dim">Permission and question asks pause the goal until you answer.</p>

        {error != null && <ErrorNote error={error} onDismiss={() => setError(null)} />}
      </div>
    </Dialog>
  );
}

function Section({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section>
      <h3 className="mb-1.5 text-label text-ink-dim">{label}</h3>
      {children}
    </section>
  );
}

function SourceLine({ spec }: { spec: GoalSpec }) {
  if (!spec.sourceRef) return null;
  const from =
    spec.source === "research"
      ? "From research"
      : spec.source === "validation"
        ? "From validation findings"
        : "From plan";
  return (
    <p className="text-ui text-ink-dim">
      {from} <span className="font-mono text-meta text-ink-2">{spec.sourceRef}</span>
      {spec.sourcePhase && ` · Phase ${spec.sourcePhase}`}
    </p>
  );
}

const CHECK_KIND_LABEL: Record<GoalCheck["kind"], string> = {
  command: "Command",
  plan_boxes: "Plan boxes",
  evaluator: "Evaluator",
  validation: "Validation",
};

function CheckLine({
  row,
  problem,
  evaluatorModel,
  onToggle,
  onEdit,
}: {
  row: CheckRow;
  problem: string | null;
  evaluatorModel: string | null;
  onToggle: (on: boolean) => void;
  onEdit: (check: GoalCheck) => void;
}) {
  const { check } = row;
  const name =
    check.kind === "command"
      ? `Run ${check.command || "new command"}`
      : CHECK_KIND_LABEL[check.kind];
  let detail: ReactNode;
  if (check.kind === "command") {
    detail = row.added ? (
      <TextInput
        aria-label="Command"
        autoFocus
        value={check.command}
        onChange={(e) => onEdit({ ...check, command: e.target.value })}
        className="h-7 py-0 font-mono text-meta"
      />
    ) : (
      <span className="selectable break-all font-mono text-meta text-ink">{check.command}</span>
    );
  } else if (check.kind === "plan_boxes") {
    detail = check.phase ? `Phase ${check.phase}` : "Every phase";
  } else if (check.kind === "evaluator") {
    detail = problem ?? (check.model || evaluatorModel || "Default model");
  } else {
    detail = "fresh validate session each iteration";
  }

  return (
    <div className="flex min-h-7 items-center gap-2.5">
      <Toggle
        aria-label={name}
        checked={row.on && !problem}
        disabled={problem !== null}
        onChange={onToggle}
      />
      <span className="w-20 shrink-0 text-ui text-ink-dim">{CHECK_KIND_LABEL[check.kind]}</span>
      <div className={cx("min-w-0 flex-1 text-ui", problem ? "text-warn-ink" : "text-ink-2")}>{detail}</div>
      {check.kind === "command" && (
        <label className="flex shrink-0 items-center gap-1.5 text-ui text-ink-dim">
          timeout
          <TextInput
            type="number"
            min={1}
            aria-label={`Timeout for ${check.command || "new command"}`}
            value={String(check.timeoutSecs)}
            onChange={(e) => onEdit({ ...check, timeoutSecs: Math.floor(Number(e.target.value)) })}
            className="h-7 w-16 py-0 text-meta"
          />
          s
        </label>
      )}
    </div>
  );
}

function LimitInput({
  label,
  ariaLabel,
  value,
  onChange,
  hit,
  step,
  suffix,
  wide,
}: {
  label: string;
  ariaLabel?: string;
  value: string;
  onChange: (value: string) => void;
  hit: boolean;
  step?: string;
  suffix?: string;
  wide?: boolean;
}) {
  return (
    <label className="flex items-center gap-1.5 text-ui text-ink-dim">
      {label}
      <TextInput
        type="number"
        min={0}
        step={step}
        aria-label={ariaLabel ?? label}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={cx("h-7 py-0 text-meta", wide ? "w-28" : "w-20", hit && "border-warn bg-warn/10")}
      />
      {suffix}
    </label>
  );
}

function RunIn({
  request,
  session,
  settings,
  form,
  patch,
}: {
  request: GoalSheetRequest;
  session: Session | null;
  settings: AppSettings | undefined;
  form: Form;
  patch: (next: Partial<Form>) => void;
}) {
  const canUseThis = "sessionId" in request.target;
  const models = modelChoices(settings, form.harness);
  return (
    <Section label="Run in">
      {canUseThis && (
        <SegmentedControl
          aria-label="Run in"
          value={form.runIn}
          options={[
            { value: "this", label: "This session" },
            { value: "new", label: "New session" },
          ]}
          onChange={(runIn) => patch({ runIn })}
          className="mb-2"
        />
      )}
      {form.runIn === "this" ? (
        <p className="text-ui text-ink-2">
          {session
            ? [stageLabel(session.stage), HARNESS_SHORT_LABEL[session.harness], session.model]
                .filter(Boolean)
                .join(" · ")
            : "This session"}
        </p>
      ) : (
        <div className="grid grid-cols-3 gap-2">
          <DropdownSelect
            aria-label="Harness"
            value={form.harness}
            options={HARNESSES.map((h) => ({ value: h.key, label: h.label }))}
            onChange={(key) => {
              const harness = key as Harness;
              patch({ harness, model: defaultModel(settings, harness) });
            }}
          />
          <DropdownSelect
            aria-label="Model"
            value={form.model ?? ""}
            placeholder="Default model"
            options={models}
            disabled={models.length === 0}
            onChange={(model) => patch({ model })}
          />
          {form.harness !== "opencode" && (
            <DropdownSelect
              aria-label="Permission mode"
              value={permissionModeOption(form.permissionKey).key}
              options={PERMISSION_MODES.map((p) => ({ value: p.key, label: p.label }))}
              onChange={(permissionKey) => patch({ permissionKey })}
            />
          )}
        </div>
      )}
    </Section>
  );
}
