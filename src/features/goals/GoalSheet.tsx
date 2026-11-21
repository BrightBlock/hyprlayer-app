import { useMemo, useState, type ReactNode } from "react";

import { Icon, type IconName } from "@/components/Icon";
import {
  Button,
  cx,
  Dialog,
  DropdownSelect,
  ErrorNote,
  rowClass,
  SegmentedControl,
  Spinner,
  TextArea,
  TextInput,
  Toggle,
  type DropdownOption,
} from "@/components/ui";
import { parsePlanDoc, type PlanDocPhase } from "@/features/plan/planDoc";
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
  useDoc,
  useGoal,
  useResumeGoal,
  useStartGoal,
  useUpdateGoal,
  useWorkstreamSessions,
} from "@/lib/queries";

import { checkPhases, comparePhaseIds, dirInside, normalizeDir, phaseAllows } from "./goalChecks";

const GATE_RULE = "A goal needs at least one command, evaluator or validation check.";
const OUTSIDE_RULE = "A check's directory must be inside the worktree.";
const GOAL_STAGES: readonly Stage[] = ["general", "research", "implement"];
const COMMAND_TIMEOUT_SECS = 600;
const MAX_ITERATIONS = 200;
const MIN_ACTIVE_SECS = 60;
const MAX_ACTIVE_SECS = 172_800;
// The server's own defaults (crates/hyprlayer-core/src/settings.rs) for a profile that omits them.
const DEFAULT_COST_USD = 10;
const DEFAULT_TOKENS = 2_000_000;
// The toggle's width plus its gap, so a card's body lines up under its label.
const CARD_INDENT = "ml-[44px]";

type Unit = "usd" | "tokens";
type LimitField = "iterations" | "budget" | "time";
type CommandCheck = Extract<GoalCheck, { kind: "command" }>;
type CheckRow = { id: number; check: GoalCheck; on: boolean; added: boolean };
type Form = {
  condition: string;
  workPrompt: string;
  rows: CheckRow[];
  phaseOff: string[];
  iterations: string;
  costUsd: string;
  tokens: string;
  activeMinutes: string;
  runIn: "this" | "new";
  harness: Harness;
  model: string | null;
  permissionKey: string;
};
type Page = { kind: "goal" } | { kind: "phase"; id: string } | { kind: "other" } | { kind: "manual" };
type SourcePlan = { title: string; phases: PlanDocPhase[] };
type SheetPhase = { id: string; name: string | null; done: boolean };
type ManualItem = { phase: string | null; title: string };

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
    phaseOff: [],
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

// A goal from a plan reads it for phase names, finished phases and manual criteria.
function useSourcePlan(spec: GoalSpec | null): SourcePlan | null {
  const fromPlan = spec?.source === "plan" || spec?.source === "plan_phase" || spec?.source === "validation";
  const path = (fromPlan && spec?.sourceRef) || "";
  const content = useDoc(path).data?.content;
  return useMemo(() => {
    if (!path || content === undefined) return null;
    const plan = parsePlanDoc(content, path);
    return { title: plan.title, phases: plan.phases };
  }, [path, content]);
}

function phaseDone(phase: PlanDocPhase): boolean {
  const boxes = [...phase.tasks, ...phase.criteria.filter((c) => c.group !== "manual")];
  return boxes.length > 0 && boxes.every((b) => b.checked);
}

// The phases the checks name, in the plan's order when it has loaded.
function sheetPhases(rows: CheckRow[], plan: SourcePlan | null): SheetPhase[] {
  const named = new Set(rows.flatMap((row) => checkPhases(row.check)));
  const known = (plan?.phases ?? [])
    .filter((p) => named.has(p.label))
    .map((p) => ({ id: p.label, name: p.name.replace(/`/g, "") || null, done: phaseDone(p) }));
  const rest = [...named]
    .filter((id) => !known.some((p) => p.id === id))
    .sort(comparePhaseIds)
    .map((id) => ({ id, name: null, done: false }));
  return [...known, ...rest];
}

function manualItems(request: GoalSheetRequest, plan: SourcePlan | null, phases: SheetPhase[]): ManualItem[] {
  if (request.manual) return request.manual.map((title) => ({ phase: null, title }));
  const ids = new Set(phases.map((p) => p.id));
  return (plan?.phases ?? [])
    .filter((p) => ids.has(p.label))
    .flatMap((p) =>
      p.criteria.filter((c) => c.group === "manual").map((c) => ({ phase: p.label, title: c.title })),
    );
}

function defaultPage(request: GoalSheetRequest, phases: SheetPhase[]): Page {
  if (request.mode !== "start") return { kind: "goal" };
  const next = phases.find((p) => !p.done) ?? phases[0];
  return next ? { kind: "phase", id: next.id } : { kind: "goal" };
}

function homeOf(check: GoalCheck): string | null {
  return checkPhases(check)[0] ?? null;
}

function checkLabel(check: CommandCheck): string {
  return check.label?.trim() || check.command.trim() || "New command";
}

function finalCheck(check: GoalCheck): GoalCheck {
  if (check.kind !== "command") return check;
  return { ...check, label: check.label?.trim() || null, cwd: normalizeDir(check.cwd), phases: check.phases ?? [] };
}

// "3/4" while some of a group's checks are off.
function onCount(rows: CheckRow[], included: (row: CheckRow) => boolean): string {
  const on = rows.filter(included).length;
  return on === rows.length ? String(on) : `${on}/${rows.length}`;
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
  const plan = useSourcePlan(spec);

  const [form, setForm] = useState<Form | null>(() =>
    spec && !bootstrap.isLoading ? initialForm(spec, request, settings) : null,
  );
  if (form === null && spec && !bootstrap.isLoading) setForm(initialForm(spec, request, settings));

  return (
    <GoalForm
      request={request}
      spec={form ? spec : null}
      plan={plan}
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
  plan,
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
  plan: SourcePlan | null;
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
  const [chosenPage, setPage] = useState<Page | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
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
        size="4xl"
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
  const editCommand = (id: number, next: Partial<CommandCheck>) =>
    patchRow(id, (r) => (r.check.kind === "command" ? { ...r, check: { ...r.check, ...next } } : r));
  const addCommand = (phase: string | null, cwd: string | null) => {
    const id = Math.max(-1, ...form.rows.map((r) => r.id)) + 1;
    const check: CommandCheck = {
      kind: "command",
      command: "",
      timeoutSecs: COMMAND_TIMEOUT_SECS,
      label: null,
      cwd,
      phases: phase ? [phase] : [],
    };
    setForm((f) => ({ ...f, rows: [...f.rows, { id, check, on: true, added: true }] }));
    setEditing(id);
  };
  const removeRow = (id: number) => setForm((f) => ({ ...f, rows: f.rows.filter((r) => r.id !== id) }));

  const newSession = request.mode === "start" && form.runIn === "new";
  const harness: Harness | null = newSession ? form.harness : (session?.harness ?? null);
  const unit: Unit = goal ? goalBudgetUnit(goal) : harness ? unitOf(harness) : goalBudgetUnit(spec);
  const requestedStage = "newSession" in request.target ? request.target.newSession.stage : null;
  const stage: Stage =
    requestedStage ?? (session && GOAL_STAGES.includes(session.stage) ? session.stage : "general");

  const off = new Set(form.phaseOff);
  const evaluatorProblem = (check: GoalCheck) =>
    check.kind === "evaluator" ? binaryProblem(binaries, evaluatorBinary(check.model, settings)) : null;
  const included = (row: CheckRow) =>
    row.on &&
    !evaluatorProblem(row.check) &&
    !(row.check.kind === "command" && !row.check.command.trim()) &&
    phaseAllows(row.check, off);
  const checks = form.rows.filter(included).map((r) => finalCheck(r.check));

  const phases = sheetPhases(form.rows, plan);
  const manual = manualItems(request, plan, phases);
  const wanted = chosenPage ?? defaultPage(request, phases);
  const page: Page =
    (wanted.kind === "phase" && !phases.some((p) => p.id === wanted.id)) ||
    (wanted.kind === "manual" && manual.length === 0)
      ? { kind: "goal" }
      : wanted;

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
    if (checks.some((c) => c.kind === "command" && c.cwd && !dirInside(c.cwd))) return OUTSIDE_RULE;
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
        const harness = form.harness;
        notifyGoalStarted(
          started,
          !newSession
            ? request
            : "sessionId" in request.target
              ? { ...request, target: { newSession: { stage, permissionMode, harness } } }
              : { ...request, target: { newSession: { ...request.target.newSession, permissionMode, harness } } },
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

  const cards = (rows: CheckRow[], phaseDir: string | null) =>
    rows.map((row) =>
      row.check.kind === "command" ? (
        <CheckCard
          key={row.id}
          row={row}
          check={row.check}
          runs={phaseAllows(row.check, off)}
          phaseDir={phaseDir}
          editing={editing === row.id}
          onToggle={(on) => patchRow(row.id, (r) => ({ ...r, on }))}
          onEdit={() => setEditing((e) => (e === row.id ? null : row.id))}
          onChange={(next) => editCommand(row.id, next)}
          onRemove={() => removeRow(row.id)}
        />
      ) : (
        <CheckLine
          key={row.id}
          row={row}
          problem={evaluatorProblem(row.check)}
          evaluatorModel={settings?.goals?.evaluatorModel ?? null}
          onToggle={(on) => patchRow(row.id, (r) => ({ ...r, on }))}
        />
      ),
    );

  let content: ReactNode;
  if (page.kind === "phase") {
    const phase = phases.find((p) => p.id === page.id)!;
    const own = form.rows.filter((r) => homeOf(r.check) === phase.id);
    const shared = form.rows.filter(
      (r) => r.check.kind === "command" && homeOf(r.check) !== phase.id && checkPhases(r.check).includes(phase.id),
    );
    const ownCommands = own.flatMap((r) => (r.check.kind === "command" ? [r.check] : []));
    const rawDirs = new Set(ownCommands.map((c) => c.cwd ?? ""));
    const commonDir = rawDirs.size === 1 ? [...rawDirs][0] : null;
    content = (
      <div className="space-y-4">
        <div className="flex items-center gap-2.5">
          <h3 className="shrink-0 text-body font-semibold text-ink">Phase {phase.id}</h3>
          <span className="min-w-0 flex-1 truncate text-ui text-ink-dim" title={phase.name ?? undefined}>
            {phase.name}
          </span>
          <Toggle
            aria-label={`Run Phase ${phase.id} checks`}
            checked={!off.has(phase.id)}
            onChange={(on) =>
              patch({ phaseOff: on ? form.phaseOff.filter((id) => id !== phase.id) : [...form.phaseOff, phase.id] })
            }
          >
            <span className="text-ui text-ink-dim">Run these checks</span>
          </Toggle>
        </div>
        {ownCommands.length > 0 && (
          <div className="flex items-center gap-2.5">
            <span className="shrink-0 text-ui text-ink-dim">Runs in</span>
            <DirInput
              aria-label={`Phase ${phase.id} directory`}
              value={commonDir ?? ""}
              placeholder={commonDir === null ? "Each check sets its own" : "Worktree root"}
              onChange={(cwd) =>
                setForm((f) => ({
                  ...f,
                  rows: f.rows.map((r) =>
                    r.check.kind === "command" && homeOf(r.check) === phase.id ? { ...r, check: { ...r.check, cwd } } : r,
                  ),
                }))
              }
            />
          </div>
        )}
        <div className="space-y-2">{cards(own, commonDir === null ? null : normalizeDir(commonDir))}</div>
        <Button size="sm" onClick={() => addCommand(phase.id, commonDir || null)}>
          <Icon name="add" size="xs" />
          Add command
        </Button>
        {shared.length > 0 && (
          <section>
            <h4 className="mb-1 text-label text-ink-dim">Shared with earlier phases</h4>
            {shared.map((row) => {
              const home = homeOf(row.check)!;
              return (
                <button
                  key={row.id}
                  type="button"
                  onClick={() => setPage({ kind: "phase", id: home })}
                  className={rowClass({ tier: "compact", className: "flex w-full gap-2 px-2 text-left text-ui" })}
                >
                  <span className={cx("min-w-0 flex-1 truncate text-ink-2", !included(row) && "opacity-50")}>
                    {checkLabel(row.check as CommandCheck)}
                  </span>
                  <span className="shrink-0 text-ui-sm text-ink-dim">Phase {home}</span>
                </button>
              );
            })}
          </section>
        )}
      </div>
    );
  } else if (page.kind === "other") {
    const rows = form.rows.filter((r) => homeOf(r.check) === null);
    const commandRows = rows.filter((r) => r.check.kind === "command");
    const otherRows = rows.filter((r) => r.check.kind !== "command");
    const addButton = (
      <Button size="sm" onClick={() => addCommand(null, null)}>
        <Icon name="add" size="xs" />
        Add command
      </Button>
    );
    content = (
      <div className="space-y-4">
        <h3 className="text-body font-semibold text-ink">{phases.length > 0 ? "Whole plan" : "Checks"}</h3>
        {phases.length > 0 && <div className="space-y-1">{cards(otherRows, null)}</div>}
        {commandRows.length > 0 && <div className="space-y-2">{cards(commandRows, null)}</div>}
        {addButton}
        {phases.length === 0 && <div className="space-y-1">{cards(otherRows, null)}</div>}
      </div>
    );
  } else if (page.kind === "manual") {
    content = (
      <div className="space-y-4">
        <h3 className="text-body font-semibold text-ink">Manual</h3>
        <ManualList items={manual} />
      </div>
    );
  } else {
    content = (
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
          <RunIn request={request} session={session} settings={settings} form={form} patch={patch} />
        )}
        <p className="text-ui text-ink-dim">Permission and question asks pause the goal until you answer.</p>
      </div>
    );
  }

  const otherRows = form.rows.filter((r) => homeOf(r.check) === null);
  return (
    <Dialog
      open
      onClose={closeGoalSheet}
      title={
        <>
          {title}
          {plan && <span className="font-normal text-ink-dim"> · {plan.title}</span>}
        </>
      }
      ariaLabel={title}
      size="4xl"
      closable
      seams
      padded={false}
      scroll={false}
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
      <div className="grid h-[38rem] min-h-0 grid-cols-[13rem_minmax(0,1fr)]">
        <nav aria-label="Goal sections" className="flex min-h-0 flex-col gap-px overflow-y-auto border-r border-edge p-2">
          <NavItem
            icon="goal"
            label="Goal"
            selected={page.kind === "goal"}
            onClick={() => setPage({ kind: "goal" })}
          />
          <div className="mx-2 mb-1.5 mt-3.5 text-label text-ink-dim">Checks · {checks.length} on</div>
          {phases.map((p) => {
            const own = form.rows.filter((r) => homeOf(r.check) === p.id);
            return (
              <NavItem
                key={p.id}
                label={`Phase ${p.id}`}
                title={p.name ?? undefined}
                count={own.length > 0 ? onCount(own, included) : undefined}
                done={p.done}
                off={off.has(p.id)}
                selected={page.kind === "phase" && page.id === p.id}
                onClick={() => setPage({ kind: "phase", id: p.id })}
              />
            );
          })}
          <NavItem
            label={phases.length > 0 ? "Whole plan" : "Checks"}
            count={otherRows.length > 0 ? onCount(otherRows, included) : undefined}
            selected={page.kind === "other"}
            onClick={() => setPage({ kind: "other" })}
          />
          {manual.length > 0 && (
            <NavItem
              label="Manual"
              count={String(manual.length)}
              selected={page.kind === "manual"}
              onClick={() => setPage({ kind: "manual" })}
            />
          )}
        </nav>
        <div className="min-h-0 overflow-y-auto px-5 py-4">
          {content}
          {error != null && (
            <div className="mt-4">
              <ErrorNote error={error} onDismiss={() => setError(null)} />
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
}

function NavItem({
  label,
  title,
  icon,
  count,
  done,
  off,
  selected,
  onClick,
}: {
  label: string;
  title?: string;
  icon?: IconName;
  count?: string;
  done?: boolean;
  off?: boolean;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-current={selected ? "page" : undefined}
      className={rowClass({
        tier: "compact",
        selected: selected ? "tint" : "none",
        className: "flex w-full shrink-0 gap-2 px-2 text-left text-ui",
      })}
    >
      {icon && <Icon name={icon} size="xs" className="text-ink-dim" />}
      <span className={cx("min-w-0 flex-1 truncate", selected ? "font-semibold text-ink" : "text-ink-2", off && "opacity-50")}>
        {label}
      </span>
      {off && <span className="rounded-control border border-edge px-1 py-0.5 text-tag text-ink-dim">Off</span>}
      {done && !off && <Icon name="check" size="xs" label="Done" className="text-ink-dim" />}
      {count && <span className={cx("font-mono text-meta text-ink-dim", off && "opacity-50")}>{count}</span>}
    </button>
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
      {from} <span className="selectable font-mono text-meta text-ink-2">{spec.sourceRef}</span>
      {spec.sourcePhase && ` · Phase ${spec.sourcePhase}`}
    </p>
  );
}

function DirInput({
  value,
  placeholder,
  onChange,
  "aria-label": ariaLabel,
}: {
  value: string;
  placeholder: string;
  onChange: (value: string) => void;
  "aria-label": string;
}) {
  return (
    <label className="flex h-7 min-w-0 flex-1 items-center gap-1.5 rounded-control border border-edge bg-panel-2 px-2 text-ink-dim focus-within:border-ink-dim/70">
      <Icon name="folder" size="xs" />
      <input
        type="text"
        aria-label={ariaLabel}
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
        className="selectable min-w-0 flex-1 bg-transparent font-mono text-meta text-ink outline-none placeholder:font-sans placeholder:text-ui placeholder:text-ink-dim/70"
      />
    </label>
  );
}

function formatTimeout(secs: number): string {
  return secs % 60 === 0 ? `${secs / 60} min` : `${secs} s`;
}

// A command with its full text, so a long one is read rather than guessed at.
function CheckCard({
  row,
  check,
  runs,
  phaseDir,
  editing,
  onToggle,
  onEdit,
  onChange,
  onRemove,
}: {
  row: CheckRow;
  check: CommandCheck;
  runs: boolean;
  phaseDir: string | null;
  editing: boolean;
  onToggle: (on: boolean) => void;
  onEdit: () => void;
  onChange: (next: Partial<CommandCheck>) => void;
  onRemove: () => void;
}) {
  const label = checkLabel(check);
  const dir = normalizeDir(check.cwd);
  const ownDir = dir !== phaseDir ? dir : null;
  return (
    <div className="rounded-card border border-edge bg-panel px-3 py-2.5">
      <div className="flex items-center gap-2.5">
        <Toggle aria-label={`Run ${label}`} checked={row.on && runs} disabled={!runs} onChange={onToggle} />
        <span
          className={cx("min-w-0 flex-1 truncate text-ui font-medium text-ink", !(runs && row.on) && "opacity-50")}
          title={label}
        >
          {label}
        </span>
        {ownDir && <span className="shrink-0 font-mono text-meta text-ink-dim">{ownDir}/</span>}
        {check.timeoutSecs !== COMMAND_TIMEOUT_SECS && (
          <span className="shrink-0 font-mono text-meta text-ink-dim">{formatTimeout(check.timeoutSecs)}</span>
        )}
        <button
          type="button"
          aria-label={`Edit ${label}`}
          aria-expanded={editing}
          onClick={onEdit}
          className={cx(
            "flex h-6 w-6 shrink-0 items-center justify-center rounded-control text-ink-dim hover:bg-edge/60 hover:text-ink focus-visible:bg-edge/60 focus-visible:text-ink",
            editing && "bg-edge/60 text-ink",
          )}
        >
          <Icon name="edit" size="xs" />
        </button>
      </div>
      {editing ? (
        <div className={cx(CARD_INDENT, "mt-2.5 grid grid-cols-[4.5rem_minmax(0,1fr)] items-center gap-x-2.5 gap-y-2")}>
          <span className="text-ui text-ink-dim">Label</span>
          <TextInput
            aria-label={`${label} label`}
            value={check.label ?? ""}
            placeholder={check.command.trim() || "Named after its command"}
            onChange={(e) => onChange({ label: e.target.value })}
            className="h-7 py-0 text-ui"
          />
          <span className="self-start pt-1.5 text-ui text-ink-dim">Command</span>
          <TextArea
            aria-label="Command"
            autoFocus={row.added}
            value={check.command}
            spellCheck={false}
            onChange={(e) => onChange({ command: e.target.value })}
            className="font-mono text-meta"
          />
          <span className="text-ui text-ink-dim">Runs in</span>
          <div className="flex min-w-0 items-center gap-2.5">
            <DirInput
              aria-label={`${label} directory`}
              value={check.cwd ?? ""}
              placeholder="Worktree root"
              onChange={(cwd) => onChange({ cwd })}
            />
            <span className="shrink-0 text-ui text-ink-dim">Timeout</span>
            <TextInput
              type="number"
              min={1}
              aria-label={`Timeout for ${label}`}
              value={String(check.timeoutSecs)}
              onChange={(e) => onChange({ timeoutSecs: Math.floor(Number(e.target.value)) })}
              className="h-7 w-20 py-0 text-meta"
            />
            <span className="shrink-0 text-ui text-ink-dim">s</span>
          </div>
          {row.added && (
            <div className="col-start-2">
              <Button size="sm" variant="danger" onClick={onRemove}>
                <Icon name="trash" size="xs" />
                Remove
              </Button>
            </div>
          )}
        </div>
      ) : (
        check.command.trim() && (
          <pre
            className={cx(
              CARD_INDENT,
              "selectable mt-2 whitespace-pre-wrap break-words rounded-control bg-panel-2 px-2.5 py-1.5 font-mono text-meta text-ink",
              !(runs && row.on) && "opacity-50",
            )}
          >
            {check.command}
          </pre>
        )
      )}
    </div>
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
}: {
  row: CheckRow;
  problem: string | null;
  evaluatorModel: string | null;
  onToggle: (on: boolean) => void;
}) {
  const { check } = row;
  let detail: ReactNode = null;
  if (check.kind === "plan_boxes") {
    detail = check.phase ? `Phase ${check.phase}'s automated boxes ticked` : "Every phase's automated boxes ticked";
  } else if (check.kind === "evaluator") {
    detail = problem ?? (check.model || evaluatorModel || "Default model");
  } else if (check.kind === "validation") {
    detail = "A fresh validate session once the rest pass";
  }
  return (
    <div className="flex min-h-8 items-center gap-2.5">
      <Toggle
        aria-label={CHECK_KIND_LABEL[check.kind]}
        checked={row.on && !problem}
        disabled={problem !== null}
        onChange={onToggle}
      />
      <span className="w-24 shrink-0 text-ui text-ink">{CHECK_KIND_LABEL[check.kind]}</span>
      <span className={cx("min-w-0 flex-1 truncate text-ui", problem ? "text-warn-ink" : "text-ink-dim")}>
        {detail}
      </span>
    </div>
  );
}

function ManualList({ items }: { items: ManualItem[] }) {
  const groups: { phase: string | null; titles: string[] }[] = [];
  for (const item of items) {
    const last = groups[groups.length - 1];
    if (last && last.phase === item.phase) last.titles.push(item.title);
    else groups.push({ phase: item.phase, titles: [item.title] });
  }
  return (
    <div className="space-y-3">
      {groups.map((group, i) => (
        <section key={`${group.phase ?? ""}:${i}`}>
          {group.phase && <h4 className="mb-1 text-label text-ink-dim">Phase {group.phase}</h4>}
          <ul className="space-y-1">
            {group.titles.map((title, j) => (
              <li key={j} className="selectable text-ui text-ink-2">
                {title}
              </li>
            ))}
          </ul>
        </section>
      ))}
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
