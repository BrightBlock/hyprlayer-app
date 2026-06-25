import { useState } from "react";
import { homeDir } from "@tauri-apps/api/path";
import { openPath } from "@tauri-apps/plugin-opener";

import type { AppSettings, BinaryStatus, ClaudeSettings } from "../../../lib/bindings";
import {
  useClaudeAuthStatus,
  useEditorTargets,
  useSetAppSettings,
} from "../../../lib/queries";
import {
  ALL_MODEL_KEYS,
  ALL_MODEL_KEY_SET,
  DEFAULT_EFFORT_KEY,
  DEFAULT_MODEL_KEY,
  EFFORTS,
  MODELS,
  modelOption,
  normalizeVisibleModelKeys,
} from "../../session/models";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  Dialog,
  ErrorNote,
  Select,
  Spinner,
  TextInput,
  Toggle,
} from "../../../components/ui";
import { ChevronDownIcon, ChevronRightIcon } from "../../../components/icons";
import { BinaryCard } from "../../system/BinaryCard";
import { SetupTerminal } from "../../onboarding/SetupTerminal";
import { SettingRow } from "../parts";

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

  function patchClaude(next: Partial<ClaudeSettings>) {
    save.mutate({
      ...settings,
      providers: { ...providers, claude: { ...claude, ...next } },
    });
  }

  const authMode = claude.authMode === "apiKey" ? "apiKey" : "cli";

  return (
    <>
      {save.isError && <ErrorNote error={save.error} />}

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
          What new Claude chats start with. You can still change any of these per tab in the
          composer.
        </p>
        <div className="mt-3 divide-y divide-edge/50">
          <SettingRow
            title="Default model"
            control={
              <div className="flex gap-2">
                <Select
                  aria-label="Default model"
                  value={claude.defaultModel ?? DEFAULT_MODEL_KEY}
                  onChange={(e) => patchClaude({ defaultModel: e.target.value })}
                  className="w-36"
                >
                  {MODELS.map((m) => (
                    <option key={m.key} value={m.key}>
                      {m.label}
                      {m.hint ? ` (${m.hint})` : ""}
                    </option>
                  ))}
                </Select>
                <Select
                  aria-label="Default effort"
                  value={claude.defaultEffort ?? DEFAULT_EFFORT_KEY}
                  onChange={(e) => patchClaude({ defaultEffort: e.target.value })}
                  className="w-32"
                >
                  {EFFORTS.map((eo) => (
                    <option key={eo.key} value={eo.key}>
                      {eo.label}
                    </option>
                  ))}
                </Select>
              </div>
            }
          >
            Model and reasoning effort for new chats.
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
          >
            Start new chats in plan mode — the agent researches and proposes a plan before making
            any edits.
          </SettingRow>
        </div>
      </Card>

      <Card>
        <CardTitle>Models visible in composer</CardTitle>
        <p className="mt-1 text-xs text-ink-dim">
          Choose which Claude models are shown when starting or switching chats.
        </p>
        <div className="mt-3">
          <VisibleModelsPanel claude={claude} patchClaude={patchClaude} />
        </div>
      </Card>

      <ClaudeAdvancedCard status={status} />
    </>
  );
}

/** Collapsible Advanced section — the executable-path override and a link to
 *  the user's Claude settings file, tucked away like the OpenCode tab's. */
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
              Override the Claude Code executable with a custom one. Leave unset to use the
              auto-detected version.
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

function AuthCard({
  label,
  sublabel,
  selected,
  onSelect,
}: {
  label: string;
  sublabel: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={`relative rounded-lg border p-4 text-left transition-colors ${
        selected ? "border-accent bg-panel-2" : "border-edge hover:bg-edge/40"
      }`}
    >
      {selected && (
        <span className="absolute right-3 top-3 text-accent" aria-hidden>
          ✓
        </span>
      )}
      <p className="text-sm font-medium text-ink">{label}</p>
      <p className="mt-0.5 text-xs text-ink-dim">{sublabel}</p>
    </button>
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

      {data?.connected && (
        <div className="overflow-hidden rounded-md border border-edge">
          {rows.map(([k, v], i) => (
            <div
              key={k}
              className={`flex items-center justify-between px-3 py-2 text-sm ${
                i > 0 ? "border-t border-edge/60" : ""
              }`}
            >
              <span className="text-ink-dim">{k}</span>
              <span className="selectable text-ink">{v ?? "—"}</span>
            </div>
          ))}
        </div>
      )}

      <div className="mt-3">
        <Button onClick={() => setLoginOpen(true)}>▶ Run claude login</Button>
      </div>

      <Dialog
        open={loginOpen}
        onClose={() => setLoginOpen(false)}
        title="Sign in to Claude Code"
        className="max-w-2xl"
      >
        <p className="mb-3 text-sm text-ink-dim">
          Follow the prompts below to sign in. This runs <code className="font-mono">claude auth login</code> in
          a terminal.
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
          Injected as <code className="font-mono">ANTHROPIC_API_KEY</code> when running Claude
          Code. Stored locally in the app settings.
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

  // Select's base style is `w-full`, so constrain it with a fixed-width wrapper
  // rather than a competing width utility (which wouldn't reliably win).
  return (
    <div className="w-40 shrink-0">
      <Select
        aria-label="Open Claude settings in"
        value=""
        onChange={(e) => {
          const v = e.target.value;
          void open(v === "__default__" ? null : v);
          e.currentTarget.value = "";
        }}
      >
        <option value="" disabled>
          Open in…
        </option>
        <option value="__default__">Default app</option>
        {targets
          .filter((t) => t.openWith)
          .map((t) => (
            <option key={t.id} value={t.openWith as string}>
              {t.name}
            </option>
          ))}
      </Select>
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

/** The "Models visible in composer" picker for Claude. `visibleModels: null`
 *  means "not customised → show every preset". (Moved here from the old
 *  Settings → Models section.) */
function VisibleModelsPanel({
  claude,
  patchClaude,
}: {
  claude: ClaudeSettings;
  patchClaude: (next: Partial<ClaudeSettings>) => void;
}) {
  const [query, setQuery] = useState("");
  const [newId, setNewId] = useState("");

  const visible = claude.visibleModels ?? null;
  const explicit = visible ?? ALL_MODEL_KEYS;
  const visibleSet = new Set(explicit);
  const customKeys = (visible ?? []).filter((k) => !ALL_MODEL_KEY_SET.has(k));

  const rows = [...MODELS, ...customKeys.map(modelOption)];
  const q = query.trim().toLowerCase();
  const filtered = q
    ? rows.filter((m) => m.label.toLowerCase().includes(q) || m.key.toLowerCase().includes(q))
    : rows;

  function toggle(key: string, on: boolean) {
    const next = on
      ? normalizeVisibleModelKeys([...explicit, key])
      : explicit.filter((k) => k !== key);
    patchClaude({ visibleModels: next });
  }

  function addCustom() {
    const id = newId.trim();
    if (!id) return;
    patchClaude({ visibleModels: normalizeVisibleModelKeys([...explicit, id]) });
    setNewId("");
  }

  const actionBtn =
    "rounded px-2 py-1 text-xs text-ink-dim hover:bg-edge/60 hover:text-ink disabled:opacity-40";

  return (
    <div className="rounded-md border border-edge bg-panel-2/40 p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <p className="text-sm text-ink">Claude Code</p>
          <p className="text-xs text-ink-dim">
            {visibleSet.size} {visibleSet.size === 1 ? "model" : "models"} visible · Default:{" "}
            {ALL_MODEL_KEYS.length} models
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            className={actionBtn}
            disabled={visible === null}
            onClick={() => patchClaude({ visibleModels: null })}
          >
            Reset
          </button>
          <button
            type="button"
            className={actionBtn}
            onClick={() =>
              patchClaude({
                visibleModels: normalizeVisibleModelKeys([...ALL_MODEL_KEYS, ...customKeys]),
              })
            }
          >
            Show all
          </button>
          <button
            type="button"
            className={actionBtn}
            disabled={visibleSet.size === 0}
            onClick={() => patchClaude({ visibleModels: [] })}
          >
            Clear
          </button>
        </div>
      </div>

      <div className="mt-2.5 flex gap-2">
        <TextInput
          value={query}
          placeholder="Search Claude Code models"
          onChange={(e) => setQuery(e.target.value)}
        />
        <TextInput
          value={newId}
          placeholder="Add model ID"
          onChange={(e) => setNewId(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") addCustom();
          }}
          className="max-w-[10rem]"
        />
        <Button onClick={addCustom} disabled={!newId.trim()}>
          Add
        </Button>
      </div>

      <ul className="mt-2 max-h-64 overflow-y-auto">
        {filtered.length === 0 && (
          <li className="px-1 py-3 text-xs text-ink-dim">No models match “{query}”.</li>
        )}
        {filtered.map((m) => {
          const custom = !ALL_MODEL_KEY_SET.has(m.key);
          return (
            <li key={m.key}>
              <label className="flex cursor-pointer items-center gap-2.5 rounded px-1 py-1.5 hover:bg-edge/40">
                <input
                  type="checkbox"
                  className="h-4 w-4 shrink-0 cursor-pointer accent-accent"
                  checked={visibleSet.has(m.key)}
                  onChange={(e) => toggle(m.key, e.target.checked)}
                />
                <span className="min-w-0">
                  <span className="text-sm text-ink">
                    {m.label}
                    {m.hint ? ` ${m.hint}` : ""}
                    {custom && (
                      <span className="ml-1.5 align-middle">
                        <Badge tone="dim">custom</Badge>
                      </span>
                    )}
                  </span>
                  <span className="block font-mono text-xs text-ink-dim">{m.flag ?? m.key}</span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
