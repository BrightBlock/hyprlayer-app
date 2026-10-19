import { useMemo, useState } from "react";

import type { AppSettings, GoalSettings } from "@/lib/bindings";
import { claudeModels, modelOptions } from "@/lib/domain/models";
import { useClaudeProbe, useOpencodeCatalog, useSetAppSettings } from "@/lib/queries";
import { SettingsCardTitle } from "@/components/SettingsCard";
import { Card, DropdownSelect, ErrorNote, SettingRow, TextInput } from "@/components/ui";

import { goalSettings, withGoals } from "./settingsPatches";
import { titleModelFromValue, titleModelGroups, titleModelValue } from "./titleModel";

export function GoalsSettings({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const goals = goalSettings(settings);
  const patch = (next: Partial<GoalSettings>) => save.mutate(withGoals(settings, next));

  return (
    <Card>
      <SettingsCardTitle>Goals</SettingsCardTitle>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}

      <div className="mt-3 divide-y divide-edge/50">
        <SettingRow
          title="Iterations"
          control={
            <NumberField
              label="Iterations"
              value={goals.maxIterations}
              parse={(n) => (Number.isInteger(n) && n >= 1 && n <= 200 ? n : null)}
              onCommit={(maxIterations) => patch({ maxIterations })}
            />
          }
        />
        <SettingRow
          title="Budget ($)"
          control={
            <NumberField
              label="Budget ($)"
              step="0.01"
              value={goals.maxCostUsd ?? 10}
              parse={(n) => (n > 0 ? n : null)}
              onCommit={(maxCostUsd) => patch({ maxCostUsd })}
            />
          }
        />
        <SettingRow
          title="Tokens (Codex)"
          control={
            <NumberField
              label="Tokens (Codex)"
              value={goals.maxTokens}
              wide
              parse={(n) => (Number.isInteger(n) && n > 0 ? n : null)}
              onCommit={(maxTokens) => patch({ maxTokens })}
            />
          }
        >
          Uncached input plus output
        </SettingRow>
        <SettingRow
          title="Active minutes"
          control={
            <NumberField
              label="Active minutes"
              value={goals.maxActiveSecs / 60}
              parse={(n) => {
                const secs = Math.round(n * 60);
                return secs >= 60 && secs <= 172_800 ? secs : null;
              }}
              onCommit={(maxActiveSecs) => patch({ maxActiveSecs })}
            />
          }
        />
        <SettingRow
          title="Evaluator model"
          control={<EvaluatorModelPicker settings={settings} onChange={(evaluatorModel) => patch({ evaluatorModel })} />}
        />
      </div>
    </Card>
  );
}

// Commits on blur or Enter; an out-of-range entry snaps back to the saved value.
function NumberField({
  label,
  value,
  parse,
  onCommit,
  step,
  wide,
}: {
  label: string;
  value: number;
  parse: (n: number) => number | null;
  onCommit: (n: number) => void;
  step?: string;
  wide?: boolean;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft === null) return;
    const next = draft.trim() === "" ? null : parse(Number(draft));
    setDraft(null);
    if (next !== null && next !== value) onCommit(next);
  };
  return (
    <TextInput
      type="number"
      aria-label={label}
      step={step}
      value={draft ?? String(value)}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
      className={wide ? "w-[140px]" : "w-[96px]"}
    />
  );
}

function EvaluatorModelPicker({
  settings,
  onChange,
}: {
  settings: AppSettings;
  onChange: (model: string | null) => void;
}) {
  const catalog = useOpencodeCatalog();
  const claudeCatalog = claudeModels(useClaudeProbe().data?.models);
  const pinned = settings.goals?.evaluatorModel ?? null;
  const opencodeNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of catalog.data?.providers ?? [])
      for (const m of p.models) map.set(`${p.id}/${m.id}`, m.name);
    return map;
  }, [catalog.data]);
  // The evaluator follows the same default as tab titles (`effective_evaluator_model` in core).
  const groups = titleModelGroups({
    claude: modelOptions(claudeCatalog, (m) => `claude:${m.flag ?? ""}`)
      .filter((o) => !o.disabled)
      .map(({ value, label }) => ({ value, label })),
    codex: settings.providers?.codex?.selectedModels ?? [],
    opencode: (settings.providers?.opencode?.selectedModels ?? []).map((id) => ({
      value: `opencode:${id}`,
      label: opencodeNames.get(id) ?? id,
    })),
    pinned,
    defaultHarness: settings.providers?.defaultHarness,
  });
  return (
    <DropdownSelect
      aria-label="Evaluator model"
      value={titleModelValue(pinned)}
      onChange={(v) => onChange(titleModelFromValue(v))}
      options={groups.flatMap((g) => g.options.map((o) => ({ ...o, group: g.label || undefined })))}
      className="w-[190px]"
    />
  );
}
