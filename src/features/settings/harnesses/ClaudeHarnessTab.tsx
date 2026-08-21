import { useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { openPath } from "@tauri-apps/plugin-opener";

import type { AppSettings, BinaryStatus, ClaudeSettings } from "@/lib/bindings";
import {
  useClaudeAuthStatus,
  useClaudeProbe,
  useEditorTargets,
  useSetAppSettings,
} from "@/lib/queries";
import {
  DEFAULT_EFFORT_KEY,
  DEFAULT_MODEL_KEY,
  EFFORTS,
  claudeModels,
  knownModelKeys,
  modelOption,
  modelOptions,
  normalizeVisibleModelKeys,
} from "@/lib/domain/models";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  Dialog,
  DropdownSelect,
  ErrorNote,
  Spinner,
  TextInput,
  Toggle,
  SettingRow,
} from "@/components/ui";
import { ChevronDownIcon, ChevronRightIcon, PlusIcon, XIcon } from "@/components/icons";
import { BinaryCard } from "@/features/system/BinaryCard";
import { AuthCard, AuthDetailRows } from "./AuthCard";
import { ModelMultiSelectDialog } from "./ModelMultiSelectDialog";
import { SetupTerminal } from "@/features/onboarding/SetupTerminal";

const CLAUDE_SETTINGS_REL = ".claude/settings.json";
const CHROME_DOCS = "https://code.claude.com/docs/en/chrome";

export function ClaudeHarnessTab({
  settings,
  status,
}: {
  settings: AppSettings;
  status?: BinaryStatus;
}) {
  const providers = settings.providers ?? {};
  const claude: ClaudeSettings = providers.claude ?? {};
  const save = useSetAppSettings();
  // Same catalog the composer offers, so the defaults can't name a model the
  // picker doesn't list.
  const catalog = claudeModels(useClaudeProbe().data?.models);

  function patchClaude(next: Partial<ClaudeSettings>) {
    save.mutate({
      ...settings,
      providers: { ...providers, claude: { ...claude, ...next } },
    });
  }

  const authMode = claude.authMode === "apiKey" ? "apiKey" : "cli";

  return (
    <>
      {save.isError && <ErrorNote error={save.error} onDismiss={() => save.reset()} />}

      <Card>
        <CardTitle>Authentication</CardTitle>
        <div className="mt-2 grid grid-cols-2 gap-3">
          <AuthCard
            label="CLI"
            sublabel="Sign in with your Claude subscription"
            selected={authMode === "cli"}
            onSelect={() => patchClaude({ authMode: "cli" })}
          />
          <AuthCard
            label="API key"
            sublabel="Use an Anthropic API key (enterprise)"
            selected={authMode === "apiKey"}
            onSelect={() => patchClaude({ authMode: "apiKey" })}
          />
        </div>

        <div className="mt-4">
          {authMode === "cli" ? (
            <CliAuthPanel />
          ) : (
            <ApiKeyAuthPanel claude={claude} patchClaude={patchClaude} />
          )}
        </div>
      </Card>

      <Card>
        <div className="divide-y divide-edge/50">
          <SettingRow
            title="Use Claude Code with Chrome"
            control={
              <Toggle
                aria-label="Use Claude Code with Chrome"
                checked={claude.chromeEnabled ?? false}
                onChange={(v) => patchClaude({ chromeEnabled: v })}
              />
            }
          >
            Let Claude Code control your Chrome browser. Requires the{" "}
            <a className="text-accent hover:underline" href="https://code.claude.com/docs/en/chrome" target="_blank" rel="noreferrer">
              Claude in Chrome
            </a>{" "}
            extension. <DocLink href={CHROME_DOCS}>Docs</DocLink>
          </SettingRow>
        </div>
      </Card>

      <Card>
        <CardTitle>Defaults</CardTitle>
        <p className="mt-1 text-xs text-ink-dim">
          Defaults for new chats. Overridable per tab in the composer.
        </p>
        <div className="mt-3 divide-y divide-edge/50">
          <SettingRow
            title="Default model"
            control={
              <div className="flex gap-2">
                <DropdownSelect
                  aria-label="Default model"
                  value={claude.defaultModel ?? DEFAULT_MODEL_KEY}
                  onChange={(v) => patchClaude({ defaultModel: v })}
                  className="w-36"
                  options={modelOptions(catalog)}
                />
                <DropdownSelect
                  aria-label="Default effort"
                  value={claude.defaultEffort ?? DEFAULT_EFFORT_KEY}
                  onChange={(v) => patchClaude({ defaultEffort: v })}
                  className="w-32"
                  options={EFFORTS.map((eo) => ({ value: eo.key, label: eo.label }))}
                />
              </div>
            }
          />

          <SettingRow
            title="Commit & Ship model"
            control={
              <div className="flex gap-2">
                <DropdownSelect
                  aria-label="Commit and Ship model"
                  value={claude.commitModel ?? "sonnet"}
                  onChange={(v) => patchClaude({ commitModel: v })}
                  className="w-36"
                  options={modelOptions(catalog)}
                />
                <DropdownSelect
                  aria-label="Commit and Ship reasoning"
                  value={claude.commitEffort ?? "medium"}
                  onChange={(v) => patchClaude({ commitEffort: v })}
                  className="w-32"
                  options={EFFORTS.map((eo) => ({ value: eo.key, label: eo.label }))}
                />
              </div>
            }
          >
            A cheaper model + reasoning for the Commit and Ship agents.
          </SettingRow>

          <SettingRow
            title="Default to plan mode"
            control={
              <Toggle
                aria-label="Default to plan mode"
                checked={claude.defaultPlanMode ?? false}
                onChange={(v) => patchClaude({ defaultPlanMode: v })}
              />
            }
          />
        </div>
      </Card>

      <VisibleModelsCard claude={claude} patchClaude={patchClaude} />

      <ClaudeAdvancedCard status={status} />
    </>
  );
}

/** Collapsible Advanced section: executable-path override + link to the user's Claude settings file. */
function ClaudeAdvancedCard({ status }: { status?: BinaryStatus }) {
  const [open, setOpen] = useState(false);
  return (
    <Card>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1.5 text-left"
      >
        {open ? <ChevronDownIcon /> : <ChevronRightIcon size={14} />}
        <span className="text-sm font-semibold text-ink">Advanced</span>
      </button>

      {open && (
        <div className="mt-3 space-y-4">
          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0">
              <p className="text-sm text-ink">Claude settings</p>
              <p className="mt-0.5 truncate font-mono text-xs text-ink-dim">
                ~/{CLAUDE_SETTINGS_REL}
              </p>
            </div>
            <OpenInEditor relPath={CLAUDE_SETTINGS_REL} />
          </div>

          <div className="border-t border-edge/50 pt-4">
            <p className="mb-2 text-xs text-ink-dim">
              Override the auto-detected Claude Code executable.
            </p>
            {status ? (
              <BinaryCard status={status} />
            ) : (
              <p className="text-xs text-ink-dim">Claude Code status unavailable.</p>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}

function CliAuthPanel() {
  const status = useClaudeAuthStatus();
  const [loginOpen, setLoginOpen] = useState(false);
  const data = status.data;

  const rows: Array<[string, string | null | undefined]> = [
    ["Provider", data?.provider],
    ["Plan", data?.plan],
    ["Org", data?.org],
    ["Account", data?.account],
  ];

  return (
    <div>
      <div className="mb-2 flex items-center justify-between">
        {status.isLoading ? (
          <span className="flex items-center gap-2 text-xs text-ink-dim">
            <Spinner size={12} /> Checking…
          </span>
        ) : data?.connected ? (
          <Badge tone="ok">Connected</Badge>
        ) : (
          <Badge tone="warn">Not signed in</Badge>
        )}
        <button
          type="button"
          onClick={() => void status.refetch()}
          className="text-xs text-ink-dim hover:text-ink"
        >
          ↻ Refresh
        </button>
      </div>

      {data?.connected && <AuthDetailRows rows={rows} />}

      <div className="mt-3">
        <Button onClick={() => setLoginOpen(true)}>▶ Run claude login</Button>
      </div>

      <Dialog
        open={loginOpen}
        onClose={() => setLoginOpen(false)}
        title="Sign in to Claude Code"
        size="2xl"
      >
        <p className="mb-3 text-xs text-ink-dim">
          Runs <code className="font-mono">claude auth login</code> in a terminal.
        </p>
        <div className="relative h-80 overflow-hidden rounded-lg border border-edge bg-surface">
          <SetupTerminal
            shellCommand="claude auth login"
            onExit={(code) => {
              if (code === 0) {
                void status.refetch();
                setLoginOpen(false);
              }
            }}
            className="h-full"
          />
        </div>
      </Dialog>
    </div>
  );
}

function ApiKeyAuthPanel({
  claude,
  patchClaude,
}: {
  claude: ClaudeSettings;
  patchClaude: (next: Partial<ClaudeSettings>) => void;
}) {
  const [key, setKey] = useState(claude.apiKey ?? "");
  const [baseUrl, setBaseUrl] = useState(claude.baseUrl ?? "");

  return (
    <div className="space-y-3">
      <div>
        <label className="mb-1 block text-xs font-medium text-ink-dim">Anthropic API key</label>
        <div className="flex gap-2">
          <TextInput
            type="password"
            placeholder="sk-ant-…"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            onBlur={() => key !== (claude.apiKey ?? "") && patchClaude({ apiKey: key })}
          />
          <Button
            variant="primary"
            disabled={key === (claude.apiKey ?? "")}
            onClick={() => patchClaude({ apiKey: key })}
          >
            Save
          </Button>
        </div>
        <p className="mt-1 text-xs text-ink-dim">
          Injected as <code className="font-mono">ANTHROPIC_API_KEY</code>. Stored locally.
        </p>
      </div>

      <div>
        <label className="mb-1 block text-xs font-medium text-ink-dim">
          Base URL <span className="text-ink-dim/70">(optional, for a proxy/gateway)</span>
        </label>
        <TextInput
          placeholder="https://api.anthropic.com"
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
          onBlur={() =>
            baseUrl !== (claude.baseUrl ?? "") && patchClaude({ baseUrl: baseUrl || null })
          }
        />
      </div>

      <div className="divide-y divide-edge/50">
        <SettingRow
          title="Route through Amazon Bedrock"
          control={
            <Toggle
              aria-label="Use Bedrock"
              checked={claude.useBedrock ?? false}
              onChange={(v) => patchClaude({ useBedrock: v })}
            />
          }
        >
          Sets <code className="font-mono">CLAUDE_CODE_USE_BEDROCK</code>; uses your AWS
          credentials.
        </SettingRow>
        <SettingRow
          title="Route through Google Vertex"
          control={
            <Toggle
              aria-label="Use Vertex"
              checked={claude.useVertex ?? false}
              onChange={(v) => patchClaude({ useVertex: v })}
            />
          }
        >
          Sets <code className="font-mono">CLAUDE_CODE_USE_VERTEX</code>; uses your GCP
          credentials.
        </SettingRow>
      </div>
    </div>
  );
}

/** "Open in <editor>" control for a path under the user's home directory. */
function OpenInEditor({ relPath }: { relPath: string }) {
  const editors = useEditorTargets();
  const targets = editors.data ?? [];

  async function open(openWith: string | null) {
    const home = await homeDir();
    const path = `${home}/${relPath}`;
    await (openWith ? openPath(path, openWith) : openPath(path)).catch(() => {});
  }

  // The w-40 wrapper sets the trigger width (passing `className="w-40"` would
  // work too, DropdownSelect drops its own `w-full` when the caller sets one).
  // `value` stays "". This is an action menu, not a stored choice, so the
  // trigger always reads "Open in…" and picking never sticks.
  return (
    <div className="w-40 shrink-0">
      <DropdownSelect
        aria-label="Open Claude settings in"
        value=""
        onChange={(v) => void open(v === "__default__" ? null : v)}
        options={[
          { value: "", label: "Open in…", disabled: true },
          { value: "__default__", label: "Default app" },
          ...targets
            .filter((t) => t.openWith)
            .map((t) => ({ value: t.openWith as string, label: t.name })),
        ]}
      />
    </div>
  );
}

function DocLink({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <a className="text-accent hover:underline" href={href} target="_blank" rel="noreferrer">
      {children} ↗
    </a>
  );
}
/**
 * "Models visible in composer" for Claude, the same card + row-list + "Add
 * models" dialog every other harness tab uses (see [`ModelMultiSelectDialog`]).
 *
 * Claude's setting is a TRI-state, unlike OpenCode/Codex's plain list:
 * `visibleModels: null` means "not customised → show every preset", which is the
 * default and why a fresh install shows all models without the user picking any.
 * Removing a model materializes the explicit list; "Reset to default" restores
 * `null`. Claude also accepts arbitrary `--model` ids, so the dialog carries an
 * add-custom-id field the other harnesses have no use for.
 */
function VisibleModelsCard({
  claude,
  patchClaude,
}: {
  claude: ClaudeSettings;
  patchClaude: (next: Partial<ClaudeSettings>) => void;
}) {
  const [pickerOpen, setPickerOpen] = useState(false);

  // The CLI's live catalog, plus the legacy pins it no longer lists; the static
  // table stands in until the probe resolves (or if it can't run). Org-disabled
  // rows resolve (a curated list can already name one) but aren't offered.
  const catalog = claudeModels(useClaudeProbe().data?.models);
  const catalogKeys = catalog.filter((m) => !m.disabled).map((m) => m.key);
  // "Custom" means a user-typed id, a preset the catalog happens not to carry
  // is still one of ours, so `knownModelKeys` (not the catalog) draws that line.
  const known = new Set(knownModelKeys(catalog));

  const visible = claude.visibleModels ?? null;
  const explicit = visible ?? catalogKeys;
  const customKeys = (visible ?? []).filter((k) => !known.has(k));
  const shown = explicit.map((k) => modelOption(k, catalog));

  function setVisible(next: string[]) {
    patchClaude({ visibleModels: normalizeVisibleModelKeys(next, catalog) });
  }

  return (
    <Card>
      <div className="flex items-center justify-between">
        <CardTitle className="mb-0">
          Models visible in composer{" "}
          <span className="ml-1 font-normal text-ink-dim">{explicit.length} selected</span>
        </CardTitle>
        <div className="flex items-center gap-2">
          {visible !== null && (
            <button
              type="button"
              className="rounded px-2 py-1 text-xs text-ink-dim hover:bg-edge/60 hover:text-ink"
              onClick={() => patchClaude({ visibleModels: null })}
            >
              Reset to default
            </button>
          )}
          <Button onClick={() => setPickerOpen(true)}>
            <PlusIcon size={14} /> Add models
          </Button>
        </div>
      </div>

      {shown.length === 0 ? (
        <p className="mt-3 text-xs text-ink-dim">
          No models selected, so the composer&apos;s Claude group is empty.
        </p>
      ) : (
        <div className="mt-3 space-y-1.5">
          {shown.map((m) => (
            <div
              key={m.key}
              className="flex items-center justify-between rounded-md border border-edge bg-panel-2 px-3 py-2"
            >
              <span className="min-w-0">
                <span className="text-sm text-ink">
                  {m.label}
                  {m.hint ? ` ${m.hint}` : ""}
                  {!known.has(m.key) && (
                    <span className="ml-1.5 align-middle">
                      <Badge tone="dim">custom</Badge>
                    </span>
                  )}
                  {m.unlisted && (
                    <span className="ml-1.5 align-middle">
                      <Badge tone="dim">unlisted</Badge>
                    </span>
                  )}
                  {m.disabled && (
                    <span className="ml-1.5 align-middle">
                      <Badge tone="warn">disabled by org policy</Badge>
                    </span>
                  )}
                </span>
                <span className="block font-mono text-xs text-ink-dim">{m.flag ?? m.key}</span>
              </span>
              <button
                type="button"
                aria-label={`Remove ${m.label}`}
                onClick={() => setVisible(explicit.filter((k) => k !== m.key))}
                className="text-ink-dim hover:text-bad"
              >
                <XIcon />
              </button>
            </div>
          ))}
        </div>
      )}

      {pickerOpen && (
      <ModelMultiSelectDialog
        open
        onClose={() => setPickerOpen(false)}
        onSave={setVisible}
        selected={explicit}
        searchPlaceholder="Filter Claude Code models"
        groups={[
          {
            key: "listed",
            models: catalog
              .filter((m) => !m.unlisted && !m.disabled)
              .map((m) => ({
                id: m.key,
                name: m.hint ? `${m.label} ${m.hint}` : m.label,
                detail: m.flag ?? m.key,
              })),
          },
          // Older ids the CLI's catalog doesn't list. `--model` still takes them,
          // but nothing can confirm they run until the first API call.
          ...(catalog.some((m) => m.unlisted)
            ? [
                {
                  key: "unlisted",
                  label: "Not listed by the CLI",
                  models: catalog
                    .filter((m) => m.unlisted)
                    .map((m) => ({
                      id: m.key,
                      name: m.hint ? `${m.label} ${m.hint}` : m.label,
                      detail: m.flag ?? m.key,
                    })),
                },
              ]
            : []),
          ...(customKeys.length > 0
            ? [
                {
                  key: "custom",
                  label: "Custom",
                  models: customKeys.map((k) => ({ id: k, name: k, detail: k })),
                },
              ]
            : []),
        ]}
        // Claude's `--model` accepts any alias or dated id, so the picker doubles
        // as the way to add one the built-in list doesn't carry.
        addCustom={{ placeholder: "Add a model ID (e.g. claude-opus-4-7-20260115)" }}
        // A dozen-odd rows make all/none worth a click; the other harnesses have
        // far longer catalogs where "select all" isn't a sensible action.
        bulkActions={(apply) => (
          <>
            <button
              type="button"
              className="rounded px-2 py-1 text-xs text-ink-dim hover:bg-edge/60 hover:text-ink"
              onClick={() => apply([...catalogKeys, ...customKeys])}
            >
              All
            </button>
            <button
              type="button"
              className="rounded px-2 py-1 text-xs text-ink-dim hover:bg-edge/60 hover:text-ink"
              onClick={() => apply([])}
            >
              None
            </button>
          </>
        )}
      />
      )}
    </Card>
  );
}
