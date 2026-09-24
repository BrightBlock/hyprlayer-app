import { useMemo } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ChevronDown } from "lucide-react";

import type { AppSettings, Bootstrap, TabTitleSettings } from "@/lib/bindings";
import { claudeModels, modelOptions } from "@/lib/domain/models";
import { IS_MACOS } from "@/lib/platform";
import {
  useClaudeProbe,
  useOpencodeCatalog,
  useSetAppSettings,
} from "@/lib/queries";
import { useUpdate } from "@/lib/updater";
import { ICON } from "@/components/lucide";
import { SettingsCard } from "@/components/SettingsCard";
import { Button, ErrorNote, SettingRow, Toggle } from "@/components/ui";

import { HarnessPicker } from "./HarnessPicker";
import { HyprlayerCard } from "./HyprlayerCard";
import { quickSettings, withConfirmQuit, withDefaultHarness } from "./settingsPatches";
import { titleModelFromValue, titleModelGroups, titleModelValue } from "./titleModel";
import { UpdatesCard } from "./updates";

export function GeneralSettings({ data }: { data: Bootstrap }) {
  const currentVersion = useUpdate((s) => s.currentVersion);
  return (
    <>
      <div className="flex items-baseline justify-between gap-4">
        <h1 className="text-[19px] font-bold tracking-tight text-ink">General</h1>
        <span className="font-mono text-xs text-ink-dim">Hyprlayer {currentVersion || "—"}</span>
      </div>
      {!IS_MACOS && <UpdatesCard />}
      <HyprlayerCard data={data} />
      <DefaultHarnessCard settings={data.settings} />
      <TabTitlesCard settings={data.settings} />
      <QuitCard settings={data.settings} />
      <SetupWizardCard />
    </>
  );
}

function DefaultHarnessCard({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  return (
    <SettingsCard
      title="Default harness"
      description="The harness new chats and the Commit & Ship actions start on. Set each harness's model on its own tab under Harnesses."
    >
      {save.isError && (
        <div className="mb-3">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}
      <HarnessPicker
        variant="radio"
        value={quickSettings(settings).defaultHarness}
        onChange={(key) => save.mutate(withDefaultHarness(settings, key))}
      />
    </SettingsCard>
  );
}

function TabTitlesCard({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const catalog = useOpencodeCatalog();
  const claudeCatalog = claudeModels(useClaudeProbe().data?.models);
  const enabled = settings.tabTitles?.enabled ?? true;
  const pinned = settings.tabTitles?.model ?? null;
  const defaultHarness = settings.providers?.defaultHarness;
  const opencodeNames = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of catalog.data?.providers ?? [])
      for (const m of p.models) map.set(`${p.id}/${m.id}`, m.name);
    return map;
  }, [catalog.data]);
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
    defaultHarness,
  });

  function patch(next: Partial<TabTitleSettings>) {
    save.mutate({ ...settings, tabTitles: { enabled, model: pinned, ...next } });
  }

  return (
    <SettingsCard
      title="Tab titles"
      description="Names each agent tab from its first prompt. Manual renames stick."
    >
      {save.isError && (
        <div className="mb-3">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}
      <div className="divide-y divide-edge-2">
        <SettingRow
          title="Auto-generate tab titles"
          control={
            <Toggle
              aria-label="Auto-generate tab titles"
              checked={enabled}
              onChange={(v) => patch({ enabled: v })}
            />
          }
        >
          Off keeps the default “Agent N” name.
        </SettingRow>
        <SettingRow
          title="Title model"
          control={
            <div className="relative w-[190px]">
              <select
                aria-label="Title model"
                disabled={!enabled}
                value={titleModelValue(pinned)}
                onChange={(e) => patch({ model: titleModelFromValue(e.target.value) })}
                className="h-7 w-full appearance-none truncate rounded-control border border-edge bg-panel-2 pl-2.5 pr-7 text-xs text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60 disabled:cursor-not-allowed disabled:opacity-50"
              >
                {groups.map((g) =>
                  g.label ? (
                    <optgroup key={g.label} label={g.label}>
                      {g.options.map((o) => (
                        <option key={o.value} value={o.value}>
                          {o.label}
                        </option>
                      ))}
                    </optgroup>
                  ) : (
                    g.options.map((o) => (
                      <option key={o.value || "follow"} value={o.value}>
                        {o.label}
                      </option>
                    ))
                  ),
                )}
              </select>
              <ChevronDown
                {...ICON}
                size={12}
                className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-ink-4"
              />
            </div>
          }
        >
          {defaultHarness === "codex"
            ? "Defaults to GPT-5.4-Mini, matching your default harness. Pick a model to pin it."
            : "Defaults to Claude Haiku, and follows your default harness. Pick a model to pin it."}
        </SettingRow>
      </div>
    </SettingsCard>
  );
}

function QuitCard({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  return (
    <SettingsCard title="Quitting">
      {save.isError && (
        <div className="mb-3">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}
      <SettingRow
        title="Confirm before quitting"
        control={
          <Toggle
            aria-label="Confirm before quitting"
            checked={quickSettings(settings).confirmQuit}
            onChange={(v) => save.mutate(withConfirmQuit(settings, v))}
          />
        }
      >
        Ask before {IS_MACOS ? "⌘Q or closing the window" : "closing the window"} stops running
        agents, terminals, and scripts.
      </SettingRow>
    </SettingsCard>
  );
}

function SetupWizardCard() {
  const navigate = useNavigate();
  return (
    <SettingsCard
      title="Setup wizard"
      description="Re-run the guided setup to repair the Claude and Codex agent links or change your thoughts repository."
    >
      <Button onClick={() => void navigate({ to: "/onboarding" })}>Re-run setup</Button>
    </SettingsCard>
  );
}
