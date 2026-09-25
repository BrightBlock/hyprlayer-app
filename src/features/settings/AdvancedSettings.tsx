import { useState } from "react";

import type { AppSettings, MultiplexerSettings } from "@/lib/bindings";
import { useSetAppSettings } from "@/lib/queries";
import { copyDiagnostics, revealLogs } from "@/lib/diagnostics";
import {
  DEFAULT_ENGINE_ID,
  enabledEngines,
  resolveMultiplexer,
  type ResolvedEngine,
} from "@/features/multiplexer/engines";
import { EngineGlyph } from "@/components/EngineGlyph";
import { Icon } from "@/components/Icon";
import { SETTINGS_CARD_DESCRIPTION, SETTINGS_CARD_TITLE } from "@/components/SettingsCard";
import {
  Button,
  Card,
  cx,
  Dialog,
  DropdownSelect,
  ErrorNote,
  Field,
  SettingRow,
  TextInput,
  Toggle,
} from "@/components/ui";

function EngineChip({
  engine,
  onToggle,
  onRemove,
}: {
  engine: ResolvedEngine;
  onToggle: () => void;
  onRemove?: () => void;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-ui ${
        engine.enabled ? "border-edge bg-panel-2 text-ink" : "border-edge/60 text-ink-dim"
      }`}
    >
      <EngineGlyph glyph={engine.glyph} accentToken={engine.accentToken} />
      <button type="button" onClick={onToggle} className="hover:text-ink" aria-pressed={engine.enabled}>
        {engine.label}
      </button>
      {engine.enabled && <Icon name="check" size="xs" className="text-accent" />}
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          title={`Remove ${engine.label}`}
          aria-label={`Remove ${engine.label}`}
          className="text-ink-dim hover:text-bad"
        >
          <Icon name="close" size="xs" />
        </button>
      )}
    </span>
  );
}

export function AdvancedSettings({ settings }: { settings: AppSettings }) {
  const cfg = resolveMultiplexer(settings);
  const save = useSetAppSettings();
  const [addOpen, setAddOpen] = useState(false);
  const [draft, setDraft] = useState({ label: "", command: "", args: "" });

  function patchMux(next: Partial<MultiplexerSettings>) {
    save.mutate({ ...settings, multiplexer: { ...cfg, ...next } });
  }
  function toggleEngine(id: string) {
    patchMux({ engines: cfg.engines.map((e) => (e.id === id ? { ...e, enabled: !e.enabled } : e)) });
  }
  function removeEngine(id: string) {
    const engines = cfg.engines.filter((e) => e.id !== id);
    const next: Partial<MultiplexerSettings> = { engines };
    if (cfg.defaultEngineId === id) {
      next.defaultEngineId = engines.find((e) => e.enabled)?.id ?? DEFAULT_ENGINE_ID;
    }
    patchMux(next);
  }
  function addEngine() {
    const command = draft.command.trim();
    if (!command) return;
    const id = globalThis.crypto?.randomUUID?.() ?? `cli-${command}`;
    const engine: ResolvedEngine = {
      id,
      label: draft.label.trim() || command,
      glyph: "▸",
      accentToken: "--color-accent",
      command,
      args: draft.args.trim() ? draft.args.trim().split(/\s+/) : [],
      enabled: true,
      builtin: false,
    };
    patchMux({ engines: [...cfg.engines, engine] });
    setDraft({ label: "", command: "", args: "" });
    setAddOpen(false);
  }

  return (
    <>
      <div>
        <h1 className="text-h3 text-ink">Advanced</h1>
        <p className={cx("mt-0.5", SETTINGS_CARD_DESCRIPTION)}>
          Power-user features. Some change how Hyprlayer runs.
        </p>
      </div>

      {save.isError && <ErrorNote error={save.error} onDismiss={() => save.reset()} />}

      <div className="overflow-hidden rounded-lg border border-accent/40 bg-accent/5">
        <div className="flex items-center gap-3 p-4">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-accent/15 text-accent">
            <Icon name="multiplexer" />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className={SETTINGS_CARD_TITLE}>Multiplexer mode</span>
              <span className="rounded border border-accent/30 px-1.5 py-0.5 text-tag text-accent">
                ADVANCED
              </span>
            </div>
            <p className={cx("mt-1 max-w-xl", SETTINGS_CARD_DESCRIPTION)}>
              Run agents as native CLIs in their own persistent PTYs. With this on, a new agent tab
              launches a CLI instead of the native chat. The <span className="text-ink">+</span>{" "}
              on the agent tab strip lets you choose which.
            </p>
          </div>
          <Toggle
            aria-label="Multiplexer mode"
            checked={cfg.enabled}
            onChange={(v) => patchMux({ enabled: v })}
          />
        </div>

        {cfg.enabled && (
          <div className="divide-y divide-edge/50 border-t border-accent/20 px-4">
            <div className="flex items-start justify-between gap-4 py-3.5">
              <div className="min-w-0">
                <div className="text-body text-ink">Available engines</div>
                <p className="mt-0.5 text-ui text-ink-dim">Which CLIs appear in the + menu.</p>
              </div>
              <div className="flex max-w-[60%] flex-wrap items-center justify-end gap-1.5">
                {cfg.engines.map((engine) => (
                  <EngineChip
                    key={engine.id}
                    engine={engine}
                    onToggle={() => toggleEngine(engine.id)}
                    onRemove={engine.builtin ? undefined : () => removeEngine(engine.id)}
                  />
                ))}
                <button
                  type="button"
                  onClick={() => setAddOpen(true)}
                  className="inline-flex items-center gap-1 rounded-md border border-dashed border-edge px-2 py-1 text-ui text-ink-dim hover:text-ink"
                >
                  <Icon name="add" size="xs" /> Add CLI…
                </button>
              </div>
            </div>

            <SettingRow
              title="Default engine"
              control={
                <DropdownSelect
                  aria-label="Default engine"
                  className="min-w-44"
                  value={cfg.defaultEngineId}
                  onChange={(v) => patchMux({ defaultEngineId: v })}
                  options={enabledEngines(cfg).map((engine) => ({
                    value: engine.id,
                    label: engine.label,
                  }))}
                />
              }
            >
              What a new agent tab launches with (⌘T).
            </SettingRow>

            <SettingRow
              title="Notify when an agent is blocked"
              control={
                <Toggle
                  aria-label="Notify when an agent is blocked"
                  checked={cfg.notifyOnBlocked}
                  onChange={(v) => patchMux({ notifyOnBlocked: v })}
                />
              }
            >
              Badge the tab + system notification when a CLI needs your input.
            </SettingRow>
          </div>
        )}
      </div>

      <Card>
        <div className="divide-y divide-edge/50">
          <SettingRow
            title="Persist agents across restarts"
            control={
              <Toggle
                aria-label="Persist agents across restarts"
                checked={cfg.persistAcrossRestarts}
                onChange={(v) => patchMux({ persistAcrossRestarts: v })}
              />
            }
          >
            Reattach running PTYs when Hyprlayer relaunches.
          </SettingRow>
          <SettingRow
            title="Developer tools"
            control={
              <Toggle
                aria-label="Developer tools"
                checked={settings.devTools ?? false}
                onChange={(v) => save.mutate({ ...settings, devTools: v })}
              />
            }
          >
            Web inspector, PTY byte log, IPC trace.
          </SettingRow>
          <SettingRow
            title="Logs"
            control={
              <div className="flex gap-2">
                <Button onClick={revealLogs}>Reveal logs</Button>
                <Button onClick={() => void copyDiagnostics()}>Copy diagnostics</Button>
              </div>
            }
          >
            App and server logs. Diagnostics mask home paths.
          </SettingRow>
        </div>
      </Card>

      <Dialog open={addOpen} onClose={() => setAddOpen(false)} title="Add a CLI engine">
        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            addEngine();
          }}
        >
          <Field label="Name">
            <TextInput
              value={draft.label}
              onChange={(e) => setDraft({ ...draft, label: e.target.value })}
              placeholder="My CLI"
              autoFocus
            />
          </Field>
          <Field label="Command">
            <TextInput
              value={draft.command}
              onChange={(e) => setDraft({ ...draft, command: e.target.value })}
              placeholder="my-cli"
            />
          </Field>
          <Field label="Arguments">
            <TextInput
              value={draft.args}
              onChange={(e) => setDraft({ ...draft, args: e.target.value })}
              placeholder="--flag value"
            />
          </Field>
          <div className="flex justify-end gap-2">
            <Button type="button" onClick={() => setAddOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={!draft.command.trim()}>
              Add
            </Button>
          </div>
        </form>
      </Dialog>
    </>
  );
}
