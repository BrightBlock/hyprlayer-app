import type { AgentControlSettings as Limits, AppSettings } from "@/lib/bindings";
import type { AgentControlLevel } from "@/lib/domain/agentControl";
import { useSetAppSettings } from "@/lib/queries";
import { SettingsCardTitle } from "@/components/SettingsCard";
import { Card, DropdownSelect, ErrorNote, SettingRow } from "@/components/ui";

import { NumberField } from "./NumberField";
import { agentControlSettings, withAgentControl } from "./settingsPatches";

const LEVELS: { value: AgentControlLevel; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "read", label: "Read" },
  { value: "full", label: "Full" },
];

export function AgentControlSettings({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const current = agentControlSettings(settings);
  const patch = (next: Partial<Limits>) => save.mutate(withAgentControl(settings, next));
  const limit = (n: number) => (Number.isInteger(n) && n >= 1 && n <= 50 ? n : null);

  return (
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
  );
}
