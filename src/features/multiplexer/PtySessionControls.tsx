import { useMemo } from "react";

import { BrandMark } from "@/components/BrandMark";
import type { OptionMenuGroup, OptionMenuItem } from "@/components/OptionMenuList";
import { commands, type Harness } from "@/lib/bindings";
import { errorMessage, unwrap } from "@/lib/ipc";
import {
  EFFORTS,
  claudeModels,
  codexEffortOption,
  codexEfforts,
  effortsForModel,
  modelDisplayName,
  resolveCodexModelIds,
  visibleModels,
} from "@/lib/domain/models";
import { PERMISSION_MODES } from "@/lib/domain/permissions";
import { formatTokens } from "@/lib/format";
import { useBootstrap, useClaudeProbe, useCodexProbe } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import {
  controlsKey,
  usePublishSessionControls,
  type SessionControlActions,
  type SessionControlsSnapshot,
} from "@/features/session/sessionControls";
import { engineBypassArgs } from "./engines";
import type { AgentRuntimeState } from "./multiplexerStore";

const PTY_HARNESS: Partial<Record<string, Harness>> = { "claude-code": "claude", codex: "codex" };

const BYPASS = "bypassPermissions";
/** What the relaunch toggle can reach: the engine's bypass flag, on or off. */
const TOGGLE_CHOICES = ["default", BYPASS];

const toItem = (e: { key: string; label: string }): OptionMenuItem => ({ key: e.key, label: e.label });

function reportFailure(title: string): (e: unknown) => void {
  return (e) => useToast.getState().show(title, errorMessage(e), { tone: "bad" });
}

type ModelControls = Pick<SessionControlsSnapshot, "model" | "effort">;

/** What a relaunch carries over, since claude's session-only changes die with its process. */
export type RelaunchKeep = { model?: string; permissionMode?: string; effort?: string };
/** What a model relaunch starts with: the picked model in the session's mode and effort. */
export type ModelRelaunch = RelaunchKeep & { model: string };

/** Publishes a PTY agent's controls to the status bar, and carries out its changes. */
export function PtySessionControls({
  workstreamId,
  shellId,
  engineId,
  agent,
  onRestart,
  onBypass,
  onModelRelaunch,
}: {
  workstreamId: string;
  shellId: number;
  engineId: string;
  agent: AgentRuntimeState | undefined;
  onRestart: (keep: RelaunchKeep) => void;
  onBypass: (on: boolean, keep: RelaunchKeep) => void;
  onModelRelaunch: (relaunch: ModelRelaunch) => void;
}) {
  const harness = PTY_HARNESS[engineId] ?? null;
  const status = agent?.hooked ? agent.status : undefined;
  const reports = !!harness && !!status?.model;
  const claude = harness === "claude";
  const providers = useBootstrap().data?.settings.providers;
  const claudeLive = useClaudeProbe(reports && claude).data?.models;
  const codexLive = useCodexProbe(reports && harness === "codex").data?.models;
  const visibleClaude = providers?.claude?.visibleModels;
  const selectedCodex = providers?.codex?.selectedModels;
  const claudeCatalog = useMemo(() => claudeModels(claudeLive), [claudeLive]);

  const model = status?.model ?? null;
  const reportedEffort = status?.effort ?? null;
  const models = useMemo<ModelControls>(() => {
    if (!model || !harness) return { model: null, effort: null };
    let groups: OptionMenuGroup[];
    let activeKey: string;
    let label: string;
    let effortItems: OptionMenuItem[];
    let effortLabel: string | undefined;
    if (claude) {
      const current = claudeCatalog.find((m) => m.resolvedModel === model);
      const icon = <BrandMark brand="claude" size="sm" />;
      groups = [
        {
          key: "claude",
          label: "Claude Code",
          icon,
          items: visibleModels(visibleClaude, claudeCatalog).map((m) => ({
            key: `claude:${m.key}`,
            label: m.label,
            hint: m.hint,
            icon,
          })),
        },
      ];
      activeKey = `claude:${current?.key ?? model}`;
      label = modelDisplayName(model) ?? model;
      // Ultracode is a Tab toggle in claude's /effort, which the driver never presses.
      effortItems = (current ? effortsForModel(current) : EFFORTS)
        .filter((e) => e.key !== "ultracode")
        .map(toItem);
      effortLabel = EFFORTS.find((e) => e.key === reportedEffort)?.label;
    } else {
      const icon = <BrandMark brand="openai" size="sm" />;
      const ids = resolveCodexModelIds(selectedCodex, codexLive ?? []);
      groups = [
        {
          key: "codex",
          label: "Codex",
          icon,
          items: ids.map((id) => ({ key: `codex:${id}`, label: id, icon })),
        },
      ];
      activeKey = `codex:${model}`;
      label = model;
      effortItems = codexEfforts(codexLive?.find((m) => m.id === model)?.supportedEfforts).map(toItem);
      effortLabel = reportedEffort ? codexEffortOption(reportedEffort).label : undefined;
    }
    return {
      model: { label, hint: null, activeKey, groups },
      // An unknown key shows as the CLI wrote it, never as a fallback tier.
      effort: reportedEffort
        ? { label: effortLabel ?? reportedEffort, activeKey: reportedEffort, items: effortItems }
        : null,
    };
  }, [claude, harness, model, reportedEffort, claudeCatalog, codexLive, visibleClaude, selectedCodex]);

  const terminalId = agent?.terminalId;
  const bypassOn = !!agent?.bypass;
  const reportedMode = status?.permissionMode;
  const controllable = !!status?.controllable && !!terminalId;
  const driven = controllable && PERMISSION_MODES.some((p) => p.key === reportedMode);
  // A hooked agent shows only a mode its CLI reported: none yet is no pill, never a default.
  const toggles = !driven && (!status || !!reportedMode) && engineBypassArgs(engineId).length > 0;
  // The CLI's own report when it is one of the two, else the launch flag the toggle flips.
  const toggleKey =
    reportedMode === "default" || reportedMode === BYPASS ? reportedMode : bypassOn ? BYPASS : "default";
  const context = status?.context;
  // The bypass flag alone sets the mode a toggle relaunch lands in.
  const keepSettings = { model: model ?? undefined, effort: reportedEffort ?? undefined };

  const snapshot: SessionControlsSnapshot = {
    harness,
    // A blocked agent is asking its user something, which a keystroke from the bar would answer.
    interactive: controllable && agent?.state !== "blocked",
    ...models,
    permissionKey: driven ? (reportedMode ?? null) : toggles ? toggleKey : null,
    permissionChoices: toggles ? TOGGLE_CHOICES : undefined,
    planning: !!status?.planning,
    context: context
      ? {
          tokens: context.tokens,
          window: context.window,
          percent: context.percent ?? undefined,
          title: `Context ${formatTokens(context.tokens)} / ${formatTokens(context.window)}`,
        }
      : null,
    goal: null,
  };

  // Rebuilt each render; the published copy calls through to the latest, so this never republishes.
  const actions: SessionControlActions = {
    pickModel: (namespaced) => {
      if (!terminalId) return;
      const key = namespaced.slice(namespaced.indexOf(":") + 1);
      // claude's picker matches the model's name, so send the id it resolves to; codex's key is its id.
      const option = claude ? claudeCatalog.find((m) => m.key === key) : undefined;
      const picked = option?.resolvedModel ?? option?.flag ?? key;
      // Read before the switch: claude moves effort to the new model's own when it switches.
      const keep = {
        permissionMode: reportedMode ?? undefined,
        effort:
          reportedEffort && (!option?.efforts || option.efforts.includes(reportedEffort)) ? reportedEffort : undefined,
      };
      void unwrap(commands.ptyAgentSetModel(terminalId, picked))
        .then((outcome) => {
          if (outcome === "relaunch") onModelRelaunch({ model: picked, ...keep });
        })
        .catch(reportFailure("Model not changed"));
    },
    pickEffort: (key) => {
      if (!terminalId) return;
      void unwrap(commands.ptyAgentSetEffort(terminalId, key)).catch(reportFailure("Effort not changed"));
    },
    setPermissionMode: (key) => {
      // claude's shift+tab ring holds Bypass only when it launched with the flag, so only a relaunch reaches it.
      if (terminalId && controllable && !(claude && key === BYPASS && !bypassOn)) {
        void unwrap(commands.ptyAgentSetPermissionMode(terminalId, key)).catch(
          reportFailure("Permission mode not changed"),
        );
        return;
      }
      if (TOGGLE_CHOICES.includes(key) && key !== toggleKey) onBypass(key === BYPASS, keepSettings);
    },
    openHarnessSettings: () => {},
    restart: () => onRestart({ ...keepSettings, permissionMode: reportedMode ?? undefined }),
  };

  usePublishSessionControls(controlsKey(workstreamId, shellId), snapshot, actions);
  return null;
}
