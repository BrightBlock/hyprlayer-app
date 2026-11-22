import { useState } from "react";

import type { AgentControlSettings as Limits, AppSettings, LeadRuleScope } from "@/lib/bindings";
import type { AgentControlLevel } from "@/lib/domain/agentControl";
import { claudeModels, effortsForModel, modelOptions } from "@/lib/domain/models";
import { basename } from "@/lib/path";
import { useClaudeProbe, useSetAppSettings } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { SettingsCard, SettingsCardTitle } from "@/components/SettingsCard";
import {
  Button,
  Card,
  DropdownSelect,
  ErrorNote,
  IconButton,
  SettingRow,
  TextArea,
  TextInput,
  Toggle,
} from "@/components/ui";

import { NumberField } from "./NumberField";
import { agentControlSettings, leadSettings, withAgentControl, withLead, type Lead } from "./settingsPatches";

const LEVELS: { value: AgentControlLevel; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "read", label: "Read" },
  { value: "full", label: "Full" },
];

const SCOPES: { value: LeadRuleScope; label: string }[] = [
  { value: "inherit_and_file", label: "Inherit + file" },
  { value: "inherit_only", label: "Inherit only" },
];

const limit = (n: number) => (Number.isInteger(n) && n >= 1 && n <= 50 ? n : null);

export function AgentControlSettings({ settings, project }: { settings: AppSettings; project?: string }) {
  const save = useSetAppSettings();
  const current = agentControlSettings(settings);
  const lead = leadSettings(settings);
  const patch = (next: Partial<Limits>) => save.mutate(withAgentControl(settings, next));
  const patchLead = (next: Partial<Lead>) => save.mutate(withLead(settings, next));

  return (
    <>
      <Card>
        <SettingsCardTitle>Agent control</SettingsCardTitle>

        {save.isError && (
          <div className="mt-3">
            <ErrorNote error={save.error} onDismiss={() => save.reset()} />
          </div>
        )}

        <div className="mt-3 divide-y divide-edge/50">
          <SettingRow
            title="Access"
            control={
              <DropdownSelect
                aria-label="Access"
                value={current.level}
                onChange={(v) => {
                  const next = LEVELS.find((l) => l.value === v);
                  if (next) patch({ level: next.value });
                }}
                options={LEVELS}
                className="w-[120px]"
              />
            }
          >
            Agents in bypass mode get Full unless this is Off.
          </SettingRow>
          <SettingRow
            title="Max depth"
            control={
              <NumberField
                label="Max depth"
                value={current.maxDepth}
                parse={limit}
                onCommit={(maxDepth) => patch({ maxDepth })}
              />
            }
          />
          <SettingRow
            title="Max live children"
            control={
              <NumberField
                label="Max live children"
                value={current.maxLiveChildren}
                parse={limit}
                onCommit={(maxLiveChildren) => patch({ maxLiveChildren })}
              />
            }
          />
          <SettingRow
            title="Max agent hops"
            control={
              <NumberField
                label="Max agent hops"
                value={current.maxAgentHops}
                parse={limit}
                onCommit={(maxAgentHops) => patch({ maxAgentHops })}
              />
            }
          />
        </div>
      </Card>

      {lead && (
        <>
          <LeadCard lead={lead} everyAgentLimit={current.maxLiveChildren} onPatch={patchLead} />
          <GuardrailsCard lead={lead} project={project} onPatch={patchLead} />
        </>
      )}
    </>
  );
}

function LeadCard({
  lead,
  everyAgentLimit,
  onPatch,
}: {
  lead: Lead;
  everyAgentLimit: number;
  onPatch: (next: Partial<Lead>) => void;
}) {
  return (
    <SettingsCard
      title="Lead Engineer"
      description="One per project, on its main checkout. It starts, checks and lands work in the project's workstreams, inside the guardrails below."
    >
      <div className="divide-y divide-edge/50">
        <SettingRow
          title="Start a Lead on every project"
          control={
            <Toggle
              aria-label="Start a Lead on every project"
              checked={lead.enabled}
              onChange={(enabled) => onPatch({ enabled })}
            />
          }
        >
          Off leaves main as a plain workstream.
        </SettingRow>
        <SettingRow title="Model" control={<LeadModelPicker lead={lead} onPatch={onPatch} />}>
          The Lead plans and judges; its agents use their own stage's model.
        </SettingRow>
        <SettingRow
          title="Max live children"
          control={
            <NumberField
              label="Lead max live children"
              value={lead.maxLiveChildren}
              parse={limit}
              onCommit={(maxLiveChildren) => onPatch({ maxLiveChildren })}
            />
          }
        >
          The Lead's own limit, above every agent's {everyAgentLimit}: a validator runs while the builder keeps
          going.
        </SettingRow>
      </div>
    </SettingsCard>
  );
}

function LeadModelPicker({ lead, onPatch }: { lead: Lead; onPatch: (next: Partial<Lead>) => void }) {
  const catalog = claudeModels(useClaudeProbe().data?.models);
  const models = modelOptions(catalog, (m) => m.flag ?? "").filter((o) => !o.disabled);
  const value = lead.model ?? "";
  if (!models.some((o) => o.value === value)) models.push({ value, label: value });
  const selected = catalog.find((m) => (m.flag ?? "") === value);
  const efforts = selected ? effortsForModel(selected) : [];
  return (
    <div className="flex items-center gap-2">
      <DropdownSelect
        aria-label="Lead model"
        value={value}
        onChange={(v) => onPatch({ model: v || null, effort: null })}
        options={models}
        className="w-[160px]"
      />
      {efforts.length > 0 && (
        <DropdownSelect
          aria-label="Lead effort"
          value={lead.effort ?? ""}
          onChange={(v) => onPatch({ effort: v || null })}
          options={[{ value: "", label: "Default" }, ...efforts.map((e) => ({ value: e.key, label: e.label }))]}
          className="w-[120px]"
        />
      )}
    </div>
  );
}

function GuardrailsCard({
  lead,
  project,
  onPatch,
}: {
  lead: Lead;
  project?: string;
  onPatch: (next: Partial<Lead>) => void;
}) {
  const [adding, setAdding] = useState(false);
  const scope = (project && lead.projects[project]) || "inherit_only";
  const setRule = (id: string, enabled: boolean) =>
    onPatch({ rules: lead.rules.map((r) => (r.id === id ? { ...r, enabled } : r)) });

  return (
    <SettingsCard
      title="Guardrails"
      description={
        <>
          Rules the Lead follows and writes into every brief it gives an agent. A project can add its own in{" "}
          <span className="font-mono">.claude/agents/lead-engineer.md</span>.
        </>
      }
      actions={
        <Button size="sm" disabled={adding} onClick={() => setAdding(true)}>
          Add rule
        </Button>
      }
    >
      <div className="divide-y divide-edge/50">
        {lead.rules.map((rule) => (
          <SettingRow
            key={rule.id}
            title={rule.title}
            control={
              <div className="flex items-center gap-1">
                <Toggle aria-label={rule.title} checked={rule.enabled} onChange={(on) => setRule(rule.id, on)} />
                <IconButton
                  aria-label={`Remove ${rule.title}`}
                  onClick={() => onPatch({ rules: lead.rules.filter((r) => r.id !== rule.id) })}
                >
                  <Icon name="trash" size="sm" />
                </IconButton>
              </div>
            }
          >
            {rule.text}
          </SettingRow>
        ))}
        {adding && (
          <RuleDraft
            onCancel={() => setAdding(false)}
            onAdd={(title, text) => {
              onPatch({ rules: [...lead.rules, { id: crypto.randomUUID(), title, text, enabled: true }] });
              setAdding(false);
            }}
          />
        )}
        {project && (
          <SettingRow
            title="This project"
            control={
              <DropdownSelect
                aria-label="This project's guardrails"
                value={scope}
                onChange={(v) => {
                  const next = SCOPES.find((s) => s.value === v);
                  if (next) onPatch({ projects: { ...lead.projects, [project]: next.value } });
                }}
                options={SCOPES}
                className="w-[140px]"
              />
            }
          >
            {basename(project)}{" "}
            {scope === "inherit_only"
              ? "follows these rules and ignores its own file."
              : "follows these rules plus its own file, if it has one."}
          </SettingRow>
        )}
      </div>
    </SettingsCard>
  );
}

function RuleDraft({ onAdd, onCancel }: { onAdd: (title: string, text: string) => void; onCancel: () => void }) {
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const ready = title.trim() !== "" && text.trim() !== "";
  return (
    <div className="space-y-2 py-3.5">
      <TextInput
        aria-label="Rule title"
        placeholder="Title"
        autoFocus
        value={title}
        onChange={(e) => setTitle(e.target.value)}
      />
      <TextArea
        aria-label="Rule text"
        placeholder="What the Lead does, or never does"
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="flex justify-end gap-2">
        <Button size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={!ready} onClick={() => onAdd(title.trim(), text.trim())}>
          Add
        </Button>
      </div>
    </div>
  );
}
