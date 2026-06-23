import { useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";

import {
  useAiStatus,
  useBootstrap,
  useGithubUsername,
  useRedetectBinaries,
  useSetAppSettings,
} from "../lib/queries";
import type {
  AppSettings,
  ClaudeSettings,
  GitSettings,
  NotificationSettings,
} from "../lib/bindings";
import { invalidateNotificationSettingsCache, sendTestNotification } from "../lib/notifications";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  Dialog,
  ErrorNote,
  PageError,
  PageSpinner,
  Select,
  TextInput,
  Toggle,
} from "../components/ui";
import { SetupTerminal } from "../features/onboarding/SetupTerminal";
import { BinaryCard } from "../features/system/BinaryCard";
import { UpdatesCard } from "../features/system/updates";
import { KeyboardSettings } from "../features/settings/KeyboardSettings";
import {
  ALL_MODEL_KEY_SET,
  ALL_MODEL_KEYS,
  DEFAULT_EFFORT_KEY,
  DEFAULT_MODEL_KEY,
  EFFORTS,
  MODELS,
  modelOption,
  normalizeVisibleModelKeys,
} from "../features/session/models";
import { THEMES, useThemeStore } from "../lib/theme";

/** [surface, panel-2, accent] preview colors per theme. Kept explicit (rather
 *  than relying on the cascade) since the theme overrides in index.css are
 *  scoped to `html.<name>` and wouldn't resolve on a swatch element. "System"
 *  previews as the default Dark+. */
const SWATCHES: Record<string, [string, string, string]> = {
  system: ["#0c0c0c", "#1c1c1c", "#d97757"],
  dark: ["#0c0c0c", "#1c1c1c", "#d97757"],
  light: ["#faf9f6", "#ebe8e1", "#c25e3d"],
  claude: ["#1f1e1d", "#2a2926", "#d97757"],
  cobalt: ["#193549", "#234a6a", "#66c4ff"],
  matrix: ["#050905", "#161e16", "#3fb950"],
  solarized: ["#002b36", "#0a4756", "#cb4b16"],
};

/** Three-swatch preview of a theme, so the picker reads at a glance. */
function ThemeSwatch({ id }: { id: string }) {
  const [surface, panel, accent] = SWATCHES[id] ?? SWATCHES.dark;
  return (
    <span className="flex shrink-0 overflow-hidden rounded border border-edge">
      <span className="h-5 w-5" style={{ background: surface }} />
      <span className="h-5 w-5" style={{ background: panel }} />
      <span className="h-5 w-5" style={{ background: accent }} />
    </span>
  );
}

function AppearanceSection() {
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);

  return (
    <Card>
      <CardTitle>Appearance</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">
        Color theme for the app. “System” follows your OS light/dark setting.
      </p>
      <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
        {THEMES.map((theme) => {
          const active = mode === theme.id;
          return (
            <button
              key={theme.id}
              type="button"
              onClick={() => setMode(theme.id)}
              aria-pressed={active}
              className={`flex items-center gap-3 rounded-md border px-3 py-2 text-left text-sm transition-colors ${
                active
                  ? "border-accent bg-panel-2 text-ink"
                  : "border-edge text-ink-dim hover:bg-edge/60 hover:text-ink"
              }`}
            >
              <ThemeSwatch id={theme.id} />
              <span className="truncate">{theme.label}</span>
            </button>
          );
        })}
      </div>
    </Card>
  );
}

/* --------------------------------- Git ----------------------------------- */

const DEFAULT_GIT: Required<GitSettings> = {
  branchPrefix: null,
  deleteBranchOnArchive: true,
  archiveOnMerge: false,
  automerge: false,
  pruneIgnoredOnArchive: true,
};

const DEFAULT_PREFIX = "hyprlayer/";
/** Stand-in slug used to preview how a branch name will read. */
const SAMPLE_SLUG = "add-login";

type PrefixMode = "default" | "github" | "custom";

/** One labelled row with a description and a control on the right. */
function SettingRow({
  title,
  children,
  control,
}: {
  title: React.ReactNode;
  children: React.ReactNode;
  control: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-4 py-3.5">
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-sm text-ink">{title}</div>
        <p className="mt-0.5 text-xs text-ink-dim">{children}</p>
      </div>
      <div className="shrink-0 pt-0.5">{control}</div>
    </div>
  );
}

function GitSection({ settings }: { settings: AppSettings }) {
  // Fields are individually optional in the bindings (the struct has
  // `#[serde(default)]`), so coalesce each against its built-in default.
  const raw = settings.git ?? {};
  const git: Required<GitSettings> = {
    branchPrefix: raw.branchPrefix ?? DEFAULT_GIT.branchPrefix ?? null,
    deleteBranchOnArchive: raw.deleteBranchOnArchive ?? DEFAULT_GIT.deleteBranchOnArchive,
    archiveOnMerge: raw.archiveOnMerge ?? DEFAULT_GIT.archiveOnMerge,
    automerge: raw.automerge ?? DEFAULT_GIT.automerge,
    pruneIgnoredOnArchive:
      raw.pruneIgnoredOnArchive ?? DEFAULT_GIT.pruneIgnoredOnArchive,
  };
  const save = useSetAppSettings();
  const username = useGithubUsername();

  // Local mirror of the custom prefix text, so typing stays responsive and we
  // only persist on commit (blur/enter). Seeded from the saved value.
  const initialCustom =
    git.branchPrefix && git.branchPrefix !== `${username.data ?? ""}/`
      ? git.branchPrefix
      : "feature/";
  const [customPrefix, setCustomPrefix] = useState(initialCustom);

  const effectivePrefix = git.branchPrefix ?? DEFAULT_PREFIX;
  const ghPrefix = username.data ? `${username.data}/` : null;

  const mode: PrefixMode =
    git.branchPrefix == null
      ? "default"
      : ghPrefix && git.branchPrefix === ghPrefix
        ? "github"
        : "custom";

  function patch(next: Partial<GitSettings>) {
    save.mutate({ ...settings, git: { ...git, ...next } });
  }

  function pickMode(m: PrefixMode) {
    if (m === "default") patch({ branchPrefix: null });
    else if (m === "github" && ghPrefix) patch({ branchPrefix: ghPrefix });
    else patch({ branchPrefix: customPrefix });
  }

  const radio = (m: PrefixMode, label: React.ReactNode) => (
    <button
      type="button"
      role="radio"
      aria-checked={mode === m}
      onClick={() => pickMode(m)}
      className="flex items-center gap-2 text-left text-sm text-ink"
    >
      <span
        className={`flex h-4 w-4 items-center justify-center rounded-full border ${
          mode === m ? "border-accent" : "border-edge"
        }`}
      >
        {mode === m && <span className="h-2 w-2 rounded-full bg-accent" />}
      </span>
      {label}
    </button>
  );

  return (
    <Card>
      <CardTitle>Git</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">
        Branch naming and the worktree archive &amp; merge behaviour for new workspaces.
      </p>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} />
        </div>
      )}

      <div className="mt-3 divide-y divide-edge/50">
        {/* Branch name prefix */}
        <div className="py-3.5">
          <p className="text-sm text-ink">Branch name prefix</p>
          <p className="mt-0.5 text-xs text-ink-dim">Prefix for new workspace branch names.</p>
          <div className="mt-3 space-y-2.5">
            {radio("default", <span>Default ({DEFAULT_PREFIX})</span>)}
            {radio(
              "github",
              ghPrefix ? (
                <span>GitHub username ({username.data})</span>
              ) : (
                <span className="text-ink-dim">
                  GitHub username{" "}
                  {username.isLoading ? "(detecting…)" : "(unavailable — run gh auth login)"}
                </span>
              ),
            )}
            {radio("custom", <span>Custom</span>)}
            <div className="pl-6">
              <TextInput
                value={customPrefix}
                placeholder="feature/"
                onFocus={() => mode !== "custom" && pickMode("custom")}
                onChange={(e) => setCustomPrefix(e.target.value)}
                onBlur={() => mode === "custom" && patch({ branchPrefix: customPrefix })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") patch({ branchPrefix: customPrefix });
                }}
                className="max-w-xs"
              />
              <p className="mt-1.5 text-xs text-ink-dim">
                Preview:{" "}
                <span className="font-mono text-ink">
                  {effectivePrefix}
                  {SAMPLE_SLUG}
                </span>
              </p>
            </div>
          </div>
        </div>

        <SettingRow
          title="Delete branch on archive"
          control={
            <Toggle
              aria-label="Delete branch on archive"
              checked={git.deleteBranchOnArchive}
              onChange={(v) => patch({ deleteBranchOnArchive: v })}
            />
          }
        >
          Delete the local branch when archiving a workspace. The remote branch is left to
          GitHub’s “automatically delete head branches” setting.
        </SettingRow>

        <SettingRow
          title="Archive on merge"
          control={
            <Toggle
              aria-label="Archive on merge"
              checked={git.archiveOnMerge}
              onChange={(v) => patch({ archiveOnMerge: v })}
            />
          }
        >
          Automatically archive a workspace once its pull request is merged.
        </SettingRow>

        <SettingRow
          title="Automerge"
          control={
            <Toggle
              aria-label="Automerge"
              checked={git.automerge}
              onChange={(v) => patch({ automerge: v })}
            />
          }
        >
          Queue a GitHub auto-merge so a PR lands once required checks pass. Requires auto-merge
          enabled in your GitHub repository settings.
        </SettingRow>

        <SettingRow
          title="Prune ignored files on archive"
          control={
            <Toggle
              aria-label="Prune ignored files on archive"
              checked={git.pruneIgnoredOnArchive}
              onChange={(v) => patch({ pruneIgnoredOnArchive: v })}
            />
          }
        >
          Remove git-ignored files (build output, dependencies) when archiving so archives stay
          small. Files in your repo's <code className="font-mono text-xs">worktree.copy</code> list
          (e.g. <code className="font-mono text-xs">.env</code>) are kept.
        </SettingRow>
      </div>
    </Card>
  );
}

/* ---------------------------- Notifications ------------------------------ */

const DEFAULT_NOTIFICATIONS: Required<NotificationSettings> = {
  enabled: true,
  sound: true,
};

function NotificationsSection({ settings }: { settings: AppSettings }) {
  const raw = settings.notifications ?? {};
  const notif: Required<NotificationSettings> = {
    enabled: raw.enabled ?? DEFAULT_NOTIFICATIONS.enabled,
    sound: raw.sound ?? DEFAULT_NOTIFICATIONS.sound,
  };
  const save = useSetAppSettings();
  // Inline result of the most recent "Send test" click — surfaces a clear
  // message when the OS rejected the toast (e.g. permission not granted
  // because the app bundle isn't properly registered with macOS).
  const [testStatus, setTestStatus] = useState<
    null | { ok: true } | { ok: false; reason: "permission_denied" | "error" }
  >(null);

  function patch(next: Partial<NotificationSettings>) {
    save.mutate(
      { ...settings, notifications: { ...notif, ...next } },
      // Invalidate the local in-memory snapshot so the next toast picks up the
      // new value without an app restart.
      { onSuccess: () => invalidateNotificationSettingsCache() },
    );
  }

  async function runTest() {
    setTestStatus(null);
    const result = await sendTestNotification();
    setTestStatus(result.sent ? { ok: true } : { ok: false, reason: result.reason ?? "error" });
  }

  return (
    <Card>
      <CardTitle>Notifications</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">
        Native OS toasts when an agent needs your input. Quiet when you’re
        already looking at the waiting session; fires otherwise — including
        when you’re on a different tab or workstream inside Hyprlayer.
      </p>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} />
        </div>
      )}

      <div className="mt-3 divide-y divide-edge/50">
        <SettingRow
          title="Notify when an agent needs you"
          control={
            <Toggle
              aria-label="Notify when an agent needs you"
              checked={notif.enabled}
              onChange={(v) => patch({ enabled: v })}
            />
          }
        >
          Show a toast when a session pauses for permission or a question.
        </SettingRow>

        <SettingRow
          title="Play sound"
          control={
            <Toggle
              aria-label="Play sound"
              checked={notif.sound}
              onChange={(v) => patch({ sound: v })}
            />
          }
        >
          Play the default OS notification sound with the toast.
        </SettingRow>

        <SettingRow
          title="Test notification"
          control={
            <Button onClick={runTest}>Send test</Button>
          }
        >
          Send a sample toast right now to confirm your OS notification
          permissions are set up. Ignores the toggles above.
          {testStatus?.ok === true && (
            <span className="ml-2 text-ok">Sent — check the top-right.</span>
          )}
          {testStatus?.ok === false && testStatus.reason === "permission_denied" && (
            <span className="ml-2 text-warn">
              macOS denied permission — enable it in System Settings → Notifications.
            </span>
          )}
          {testStatus?.ok === false && testStatus.reason === "error" && (
            <span className="ml-2 text-bad">Dispatch failed — see the console.</span>
          )}
        </SettingRow>
      </div>
    </Card>
  );
}

/* -------------------------------- Models --------------------------------- */

/** Model `<option>`s for a default-model picker, built from MODELS so the
 *  settings page and composer stay in lock-step. */
function ModelOptions() {
  return (
    <>
      {MODELS.map((m) => (
        <option key={m.key} value={m.key}>
          {m.label}
          {m.hint ? ` (${m.hint})` : ""}
        </option>
      ))}
    </>
  );
}

/** Effort `<option>`s for an effort picker. */
function EffortOptions() {
  return (
    <>
      {EFFORTS.map((e) => (
        <option key={e.key} value={e.key}>
          {e.label}
        </option>
      ))}
    </>
  );
}

/** The "Models visible in composer" picker for the Claude provider. Toggling,
 *  adding custom ids, and the bulk actions all write `visibleModels`: `null`
 *  means "not customised → show every preset". */
function VisibleModelsPanel({
  claude,
  patchClaude,
}: {
  claude: ClaudeSettings;
  patchClaude: (next: Partial<ClaudeSettings>) => void;
}) {
  const [query, setQuery] = useState("");
  const [newId, setNewId] = useState("");

  const visible = claude.visibleModels ?? null; // null = default (all presets)
  // The current selection as an explicit list (materialising the "all" default).
  const explicit = visible ?? ALL_MODEL_KEYS;
  const visibleSet = new Set(explicit);
  const customKeys = (visible ?? []).filter((k) => !ALL_MODEL_KEY_SET.has(k));

  // Rows = every preset plus any custom ids the user added.
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

function ModelsSection({ settings }: { settings: AppSettings }) {
  // `providers` and each sub-section are individually optional in the bindings
  // (every struct is `#[serde(default)]`), so coalesce as we read.
  const providers = settings.providers ?? {};
  const claude: ClaudeSettings = providers.claude ?? {};
  const save = useSetAppSettings();

  function patchClaude(next: Partial<ClaudeSettings>) {
    save.mutate({
      ...settings,
      providers: { ...providers, claude: { ...claude, ...next } },
    });
  }

  const defaultModel = claude.defaultModel ?? DEFAULT_MODEL_KEY;
  const defaultEffort = claude.defaultEffort ?? DEFAULT_EFFORT_KEY;

  const comingSoon = <Badge tone="dim">Coming soon</Badge>;

  return (
    <>
      {save.isError && <ErrorNote error={save.error} />}

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
                  value={defaultModel}
                  onChange={(e) => patchClaude({ defaultModel: e.target.value })}
                  className="w-36"
                >
                  <ModelOptions />
                </Select>
                <Select
                  aria-label="Default effort"
                  value={defaultEffort}
                  onChange={(e) => patchClaude({ defaultEffort: e.target.value })}
                  className="w-32"
                >
                  <EffortOptions />
                </Select>
              </div>
            }
          >
            Model and reasoning effort for new chats.
          </SettingRow>

          {/* Stub — there's no in-app code-review session yet. */}
          <SettingRow
            title={<>Review model {comingSoon}</>}
            control={
              <div className="flex gap-2">
                <Select aria-label="Review model" value={defaultModel} disabled className="w-36">
                  <ModelOptions />
                </Select>
                <Select aria-label="Review effort" value={defaultEffort} disabled className="w-32">
                  <EffortOptions />
                </Select>
              </div>
            }
          >
            Model for code reviews.
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

          {/* Stub — "fast mode" isn't a claude spawn flag the app passes yet. */}
          <SettingRow
            title={<>Default to fast mode {comingSoon}</>}
            control={<Toggle aria-label="Default to fast mode" checked={false} disabled onChange={() => {}} />}
          >
            Start new chats in fast mode.
          </SettingRow>
        </div>
      </Card>

      <Card>
        <div className="flex items-center justify-between">
          <CardTitle className="mb-0">Codex</CardTitle>
          {comingSoon}
        </div>
        <p className="mt-1 text-xs text-ink-dim">
          OpenAI Codex support is on the way. Its defaults will appear here once it lands.
        </p>
        <div className="mt-3 divide-y divide-edge/50">
          <SettingRow
            title="Personality for new chats"
            control={
              <Select aria-label="Codex personality" value="" disabled className="w-44">
                <option value="">Pragmatic (default)</option>
              </Select>
            }
          >
            Style to use when a new chat starts with a Codex model.
          </SettingRow>
        </div>
      </Card>

      <Card>
        <CardTitle>Models visible in composer</CardTitle>
        <p className="mt-1 text-xs text-ink-dim">
          Choose which provider models are shown when starting or switching chats.
        </p>
        <div className="mt-3">
          <VisibleModelsPanel claude={claude} patchClaude={patchClaude} />
        </div>
      </Card>
    </>
  );
}

/* ------------------------------- General --------------------------------- */

/** AI tool card — shows current agent-tool status and lets users reconfigure
 *  via the embedded terminal without leaving Settings. */
function AiToolCard() {
  const aiStatus = useAiStatus();
  const [dialogOpen, setDialogOpen] = useState(false);

  const handleExit = (code: number | null) => {
    if (code === 0) {
      void aiStatus.refetch();
      setDialogOpen(false);
    }
  };

  return (
    <Card>
      <div className="flex items-center justify-between">
        <CardTitle className="mb-0">AI tool</CardTitle>
        <Button onClick={() => setDialogOpen(true)}>
          {aiStatus.data?.installed ? "Reconfigure" : "Configure"}
        </Button>
      </div>
      <div className="mt-2 text-sm">
        {aiStatus.isLoading && (
          <p className="text-ink-dim">Checking status…</p>
        )}
        {!aiStatus.isLoading && aiStatus.data?.installed && (
          <p>
            <Badge tone="ok">Installed</Badge>
            <span className="ml-2 text-ink-dim">
              {aiStatus.data.agentTool ?? "AI tool"}{" "}
              {aiStatus.data.location ? (
                <span className="font-mono text-xs">{aiStatus.data.location}</span>
              ) : null}
            </span>
          </p>
        )}
        {!aiStatus.isLoading && !aiStatus.data?.installed && (
          <p>
            <Badge tone="warn">Not configured</Badge>
            <span className="ml-2 text-ink-dim">
              Click Configure to install your AI coding assistant.
            </span>
          </p>
        )}
      </div>

      <Dialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        title="Configure AI tool"
        className="max-w-2xl"
      >
        <p className="mb-3 text-sm text-ink-dim">
          Use the terminal below to select and install your AI coding assistant.
          The dialog closes automatically on success.
        </p>
        <div className="relative h-80 rounded-lg border border-edge bg-surface overflow-hidden">
          <SetupTerminal
            args={["ai", "configure", "--force"]}
            onExit={handleExit}
            className="h-full"
          />
        </div>
      </Dialog>
    </Card>
  );
}

function HyprlayerConfigCard({ config }: { config: ReturnType<typeof useBootstrap>["data"] }) {
  const data = config!;
  return (
    <Card>
      <CardTitle>Hyprlayer config</CardTitle>
      {data.config.state === "ready" && (
        <div className="space-y-1 text-sm">
          <p>
            <Badge tone="ok">Found</Badge>
            <span className="selectable ml-2 font-mono text-xs text-ink-dim">
              {data.config.path}
            </span>
          </p>
          <p className="text-ink-dim">
            User <span className="font-mono text-ink">{data.config.user || "—"}</span> · default
            backend <span className="font-mono text-ink">{data.config.backendKind}</span>
            {!data.config.configured && (
              <span className="ml-2">
                <Badge tone="warn">incomplete</Badge>
              </span>
            )}
          </p>
          <p className="mt-2 text-xs text-ink-dim">
            The hyprlayer CLI owns this file; the app reads it and follows changes live.
          </p>
        </div>
      )}
      {data.config.state === "missing" && (
        <div className="space-y-2 text-sm">
          <p>
            <Badge tone="warn">Not set up yet</Badge>
          </p>
          <p className="text-ink-dim">
            No hyprlayer config was found at{" "}
            <span className="selectable font-mono text-xs">{data.config.path}</span>. Add your
            first project from the dashboard — the app sets up your thoughts storage automatically.
          </p>
        </div>
      )}
      {data.config.state === "error" && (
        <ErrorNote error={{ message: data.config.message, remedy: data.config.remedy }} />
      )}
    </Card>
  );
}

function BinariesCard({ data }: { data: NonNullable<ReturnType<typeof useBootstrap>["data"]> }) {
  const redetect = useRedetectBinaries();
  return (
    <Card>
      <div className="flex items-center justify-between">
        <CardTitle className="mb-0">Binaries</CardTitle>
        <Button onClick={() => redetect.mutate()} busy={redetect.isPending}>
          Re-detect
        </Button>
      </div>
      <p className="mt-1 text-xs text-ink-dim">
        The app searches your shell PATH plus common install locations. Set an explicit path if a
        tool isn't found.
      </p>
      <div className="mt-3 space-y-2">
        {data.binaries.map((b) => (
          <BinaryCard key={b.kind} status={b} />
        ))}
      </div>
    </Card>
  );
}

/* ------------------------------ Page shell ------------------------------- */

type SectionId =
  | "general"
  | "models"
  | "git"
  | "notifications"
  | "keyboard"
  | "appearance"
  | "binaries";

const NAV: Array<{ id: SectionId; label: string }> = [
  { id: "general", label: "General" },
  { id: "models", label: "Models" },
  { id: "git", label: "Git" },
  { id: "notifications", label: "Notifications" },
  { id: "keyboard", label: "Keyboard" },
  { id: "appearance", label: "Appearance" },
  { id: "binaries", label: "Binaries" },
];

export function Settings() {
  const bootstrap = useBootstrap();
  const [active, setActive] = useState<SectionId>("general");

  const section = useMemo(() => active, [active]);

  if (bootstrap.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;

  const data = bootstrap.data!;

  return (
    <div className="mx-auto flex max-w-4xl gap-8 p-8">
      <nav className="w-40 shrink-0">
        <h1 className="mb-3 text-xs font-semibold uppercase tracking-wider text-ink-dim">
          Settings
        </h1>
        <ul className="space-y-0.5">
          {NAV.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                onClick={() => setActive(item.id)}
                aria-current={section === item.id}
                className={`w-full rounded-md px-2.5 py-1.5 text-left text-sm transition-colors ${
                  section === item.id
                    ? "bg-edge text-ink"
                    : "text-ink-dim hover:bg-edge/60 hover:text-ink"
                }`}
              >
                {item.label}
              </button>
            </li>
          ))}
        </ul>
      </nav>

      <div className="min-w-0 flex-1 space-y-4">
        {section === "general" && (
          <>
            <UpdatesCard />
            <AiToolCard />
            <HyprlayerConfigCard config={data} />
            <Card>
              <CardTitle>Setup wizard</CardTitle>
              <p className="mb-3 text-sm text-ink-dim">
                Re-run the guided setup to reconfigure your AI tool or change your thoughts
                repository.
              </p>
              <Link to="/onboarding">
                <Button>Re-run setup</Button>
              </Link>
            </Card>
          </>
        )}
        {section === "models" && <ModelsSection settings={data.settings} />}
        {section === "git" && <GitSection settings={data.settings} />}
        {section === "notifications" && <NotificationsSection settings={data.settings} />}
        {section === "keyboard" && <KeyboardSettings settings={data.settings} />}
        {section === "appearance" && <AppearanceSection />}
        {section === "binaries" && <BinariesCard data={data} />}
      </div>
    </div>
  );
}
