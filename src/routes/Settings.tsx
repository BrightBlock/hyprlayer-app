import { useMemo, useState } from "react";
import { Link, useSearch } from "@tanstack/react-router";

import {
  useAiStatus,
  useBootstrap,
  useGithubUsername,
  useOpencodeCatalog,
  useRedetectBinaries,
  useSetAppSettings,
} from "@/lib/queries";
import type {
  AppSettings,
  GitSettings,
  MultiplexerSettings,
  NotificationSettings,
  TabTitleSettings,
} from "@/lib/bindings";
import {
  DEFAULT_ENGINE_ID,
  enabledEngines,
  resolveMultiplexer,
  type ResolvedEngine,
} from "@/features/multiplexer/engines";
import { EngineGlyph } from "@/features/multiplexer/agentStateUi";
import { MODELS } from "@/lib/domain/models";
import { DEFAULT_HARNESS_OPTIONS } from "@/lib/domain/harnessing";
import { invalidateNotificationSettingsCache, sendTestNotification } from "@/lib/notifications";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  Dialog,
  ErrorNote,
  Field,
  PageError,
  PageSpinner,
  Select,
  TextInput,
  Toggle,
} from "@/components/ui";
import { SetupTerminal } from "@/features/onboarding/SetupTerminal";
import { BinaryCard } from "@/features/system/BinaryCard";
import { UpdatesCard } from "@/features/system/updates";
import { KeyboardSettings } from "@/features/settings/KeyboardSettings";
import { HarnessesSection } from "@/features/settings/harnesses/HarnessesSection";
import { SettingRow } from "@/features/settings/parts";
import { THEMES, useThemeStore } from "@/lib/theme";
import { IS_MACOS, IS_WINDOWS } from "@/lib/platform";
import { AnytypeIcon, CheckIcon, ClipboardIcon, GitBranchIcon, harnessIcon, MultiplexerIcon, NotionIcon, ObsidianIcon, PersonIcon, PlusIcon, XIcon } from "@/components/icons";

function BackendIcon({ kind, size = 15 }: { kind: string; size?: number }) {
  switch (kind) {
    case "git": return <GitBranchIcon size={size} />;
    case "obsidian": return <ObsidianIcon size={size} />;
    case "notion": return <NotionIcon size={size} />;
    case "anytype": return <AnytypeIcon size={size} />;
    default: return null;
  }
}

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
          Removes the branch when the workspace is archived. Only branches the app created are affected.
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
          Enable GitHub auto-merge when required checks pass. Requires auto-merge enabled in the repo settings.
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
          Remove git-ignored files when archiving. Files in{" "}
          <code className="font-mono text-xs">worktree.copy</code> (e.g.{" "}
          <code className="font-mono text-xs">.env</code>) are kept.
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
        Native OS toasts when an agent needs your input.
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
          Confirm your OS notification permissions are working.
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

/* ------------------------------- General --------------------------------- */

/** Built-in default title models (mirror the backend's DEFAULT_TAB_TITLE_MODEL /
 *  DEFAULT_CODEX_TAB_TITLE_MODEL — keep in sync with `settings.rs`). */
const DEFAULT_TAB_TITLE_MODEL = "claude:haiku";
const DEFAULT_CODEX_TAB_TITLE_MODEL = "codex:gpt-5.4-mini";

/** The title model used when the user hasn't picked one — Codex-by-default users
 *  get Codex's small model rather than Claude Haiku, which would need a `claude`
 *  binary they may not have. Mirrors `TabTitleSettings::effective_model`. */
function defaultTitleModel(defaultHarness: string | null | undefined): string {
  return defaultHarness === "codex" ? DEFAULT_CODEX_TAB_TITLE_MODEL : DEFAULT_TAB_TITLE_MODEL;
}

/** Toggle auto-generated agent-tab titles and pick the model that writes them —
 *  any Claude model, or one of the user's configured OpenCode models so
 *  opencode-only users are covered. The choice is stored as a tagged id
 *  (`claude:<flag>` / `opencode:<provider/model>`) the backend routes on. */
function DefaultHarnessCard({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const current = settings.providers?.defaultHarness ?? "claude";
  return (
    <Card>
      <CardTitle>Default harness</CardTitle>
      <p className="mt-1 mb-3 text-xs text-ink-dim">
        The harness new chats and the Commit &amp; Ship actions start on. Set each
        harness's model on its own tab under Harnesses.
      </p>
      {save.isError && (
        <div className="mb-3">
          <ErrorNote error={save.error} />
        </div>
      )}
      <div className="flex gap-2">
        {DEFAULT_HARNESS_OPTIONS.map((h) => {
          const active = current === h.key;
          const Icon = harnessIcon(h.key);
          return (
            <button
              key={h.key}
              type="button"
              aria-pressed={active}
              onClick={() =>
                save.mutate({
                  ...settings,
                  providers: { ...(settings.providers ?? {}), defaultHarness: h.key },
                })
              }
              className={`flex flex-1 items-center justify-center gap-1.5 rounded-md border bg-panel-2 px-3 py-2 text-sm transition-colors ${
                active ? "border-accent text-ink" : "border-edge text-ink-dim hover:text-ink"
              }`}
            >
              <Icon size={14} />
              {h.label}
            </button>
          );
        })}
      </div>
    </Card>
  );
}

/** Confirm-on-quit. Off is also reachable from the quit dialog's "Don't ask
 *  again" checkbox, which is why this row exists: it's the only way back on. */
function QuitCard({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const confirm = settings.quit?.confirm ?? true;
  return (
    <Card>
      <CardTitle>Quitting</CardTitle>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} />
        </div>
      )}

      <div className="mt-1 divide-y divide-edge/50">
        <SettingRow
          title="Confirm before quitting"
          control={
            <Toggle
              aria-label="Confirm before quitting"
              checked={confirm}
              onChange={(v) => save.mutate({ ...settings, quit: { confirm: v } })}
            />
          }
        >
          Ask before {IS_MACOS ? "⌘Q or closing the window" : "closing the window"} stops running
          agents, terminals, and scripts.
        </SettingRow>
      </div>
    </Card>
  );
}

function TabTitlesCard({ settings }: { settings: AppSettings }) {
  const save = useSetAppSettings();
  const catalog = useOpencodeCatalog();
  const tab: Required<TabTitleSettings> = {
    enabled: settings.tabTitles?.enabled ?? true,
    model: settings.tabTitles?.model ?? null,
  };
  const selectedModel =
    tab.model && tab.model.length > 0
      ? tab.model
      : defaultTitleModel(settings.providers?.defaultHarness);
  const opencodeModels = settings.providers?.opencode?.selectedModels ?? [];
  // Codex's own small model is always offered (it's the Codex default and needs
  // no configuration); anything the user selected in the Codex tab joins it.
  const codexModels = [
    ...new Set([
      DEFAULT_CODEX_TAB_TITLE_MODEL.replace(/^codex:/, ""),
      ...(settings.providers?.codex?.selectedModels ?? []),
    ]),
  ];
  // Friendly name for a "provider/model" id, from the cached models.dev catalog.
  const modelName = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of catalog.data?.providers ?? [])
      for (const m of p.models) map.set(`${p.id}/${m.id}`, m.name);
    return map;
  }, [catalog.data]);

  function patch(next: Partial<TabTitleSettings>) {
    save.mutate({ ...settings, tabTitles: { ...tab, ...next } });
  }

  return (
    <Card>
      <CardTitle>Tab titles</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">
        Automatically name each agent tab from its first prompt. Manual renames are preserved.
      </p>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} />
        </div>
      )}

      <div className="mt-3 divide-y divide-edge/50">
        <SettingRow
          title="Auto-generate tab titles"
          control={
            <Toggle
              aria-label="Auto-generate tab titles"
              checked={tab.enabled}
              onChange={(v) => patch({ enabled: v })}
            />
          }
        >
          Off keeps the default “Agent N” name.
        </SettingRow>

        <SettingRow
          title="Title model"
          control={
            <Select
              aria-label="Title model"
              className="w-56"
              disabled={!tab.enabled}
              value={selectedModel}
              onChange={(e) => patch({ model: e.target.value })}
            >
              <optgroup label="Claude">
                {MODELS.map((m) => (
                  <option key={m.key} value={`claude:${m.flag ?? ""}`}>
                    {m.label}
                    {m.hint ? ` (${m.hint})` : ""}
                  </option>
                ))}
              </optgroup>
              <optgroup label="Codex">
                {codexModels.map((id) => (
                  <option key={id} value={`codex:${id}`}>
                    {id}
                  </option>
                ))}
              </optgroup>
              {opencodeModels.length > 0 && (
                <optgroup label="OpenCode">
                  {opencodeModels.map((id) => (
                    <option key={id} value={`opencode:${id}`}>
                      {modelName.get(id) ?? id}
                    </option>
                  ))}
                </optgroup>
              )}
            </Select>
          }
        >
          {settings.providers?.defaultHarness === "codex"
            ? "Defaults to GPT-5.4-Mini, matching your default harness."
            : "Defaults to Claude Haiku, and follows your default harness. Pick a model to pin it."}
        </SettingRow>
      </div>
    </Card>
  );
}

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
        <p className="mb-3 text-xs text-ink-dim">
          Select and install your AI coding assistant below.
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
  const [pathCopied, setPathCopied] = useState(false);

  function copyPath(path: string) {
    void navigator.clipboard.writeText(path).then(() => {
      setPathCopied(true);
      window.setTimeout(() => setPathCopied(false), 1200);
    });
  }
  return (
    <Card>
      <div className="flex items-center justify-between gap-4">
        <CardTitle className="mb-0">Hyprlayer</CardTitle>
        {data.config.state === "ready" && (
          <span className="flex items-center gap-1.5 text-sm text-ink">
            <BackendIcon kind={data.config.backendKind} size={15} />
            <span className="font-mono font-semibold">{data.config.backendKind}</span>
            {!data.config.configured && <Badge tone="warn">incomplete</Badge>}
          </span>
        )}
      </div>
      {data.config.state === "ready" && (() => {
        const hlBinary = data.binaries.find((b) => b.kind === "hyprlayer");
        const binaryOk = hlBinary?.state.state === "ok";
        return (
          <div className="mt-2 flex items-center justify-between gap-4">
            <div className="flex min-w-0 items-center gap-2">
              {hlBinary && (
                <Badge tone={binaryOk ? "ok" : "warn"}>{binaryOk ? "Found" : "Missing"}</Badge>
              )}
              <button
                type="button"
                onClick={() => copyPath(data.config.path)}
                title={pathCopied ? "Copied" : "Copy path"}
                className="group flex min-w-0 items-center gap-1.5 text-left"
              >
                <span className="truncate font-mono text-xs text-ink-dim group-hover:text-ink">
                  {data.config.path}
                </span>
                <span className="shrink-0 text-ink-dim opacity-0 transition-opacity group-hover:opacity-100">
                  {pathCopied ? <span className="text-xs text-ok">✓</span> : <ClipboardIcon size={11} />}
                </span>
              </button>
            </div>
            <p className="flex shrink-0 items-center gap-1 font-mono text-sm text-ink-dim">
              <PersonIcon size={13} />
              <span>{data.config.user || "—"}</span>
            </p>
          </div>
        );
      })()}
      {data.config.state === "missing" && (
        <div className="space-y-2 text-sm">
          <p>
            <Badge tone="warn">Not set up yet</Badge>
          </p>
          <p className="text-ink-dim">
            No config found at{" "}
            <span className="selectable font-mono text-xs">{data.config.path}</span>. Add your
            first project from the dashboard.
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

/* ------------------------------- Advanced -------------------------------- */

/** An engine toggle chip in "Available engines". Click toggles it on/off in the
 *  `+` menu; custom (non-builtin) engines get a remove "×". */
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
      className={`inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs ${
        engine.enabled ? "border-edge bg-panel-2 text-ink" : "border-edge/60 text-ink-dim"
      }`}
    >
      <EngineGlyph glyph={engine.glyph} accentToken={engine.accentToken} />
      <button type="button" onClick={onToggle} className="hover:text-ink" aria-pressed={engine.enabled}>
        {engine.label}
      </button>
      {engine.enabled && <CheckIcon size={11} className="text-accent" />}
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          title={`Remove ${engine.label}`}
          className="text-ink-dim hover:text-bad"
        >
          <XIcon size={11} />
        </button>
      )}
    </span>
  );
}

function AdvancedSection({ settings }: { settings: AppSettings }) {
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
        <h1 className="text-base font-semibold text-ink">Advanced</h1>
        <p className="mt-0.5 text-xs text-ink-dim">
          Power-user features. Some change how Hyprlayer runs — enable with intent.
        </p>
      </div>

      {save.isError && <ErrorNote error={save.error} />}

      {/* Hero: Multiplexer mode */}
      <div className="overflow-hidden rounded-lg border border-accent/40 bg-accent/5">
        <div className="flex items-center gap-3 p-4">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-accent/15 text-accent">
            <MultiplexerIcon size={18} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-ink">Multiplexer mode</span>
              <span className="rounded border border-accent/30 px-1.5 py-0.5 text-[9px] font-semibold tracking-wide text-accent">
                ADVANCED
              </span>
            </div>
            <p className="mt-1 max-w-xl text-xs leading-relaxed text-ink-dim">
              Run agents as native CLIs in their own persistent PTYs. With this on, a new agent tab
              launches a CLI instead of the native chat — and the <span className="text-ink">+</span>{" "}
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
            {/* Available engines */}
            <div className="flex items-start justify-between gap-4 py-3.5">
              <div className="min-w-0">
                <div className="text-sm text-ink">Available engines</div>
                <p className="mt-0.5 text-xs text-ink-dim">Which CLIs appear in the + menu.</p>
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
                  className="inline-flex items-center gap-1 rounded-md border border-dashed border-edge px-2 py-1 text-xs text-ink-dim hover:text-ink"
                >
                  <PlusIcon size={11} /> Add CLI…
                </button>
              </div>
            </div>

            {/* Default engine */}
            <SettingRow
              title="Default engine"
              control={
                <Select
                  className="min-w-44"
                  value={cfg.defaultEngineId}
                  onChange={(e) => patchMux({ defaultEngineId: e.target.value })}
                >
                  {enabledEngines(cfg).map((engine) => (
                    <option key={engine.id} value={engine.id}>
                      {engine.label}
                    </option>
                  ))}
                </Select>
              }
            >
              What a new agent tab launches with (⌘T).
            </SettingRow>

            {/* Notify when blocked */}
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

      {/* Sibling advanced rows */}
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

/* ------------------------------ Page shell ------------------------------- */

type SectionId =
  | "general"
  | "harnesses"
  | "git"
  | "notifications"
  | "keyboard"
  | "appearance"
  | "advanced"
  | "binaries";

const NAV: Array<{ id: SectionId; label: string }> = [
  { id: "general", label: "General" },
  { id: "harnesses", label: "Harnesses" },
  { id: "git", label: "Git" },
  { id: "notifications", label: "Notifications" },
  { id: "keyboard", label: "Keyboard" },
  { id: "appearance", label: "Appearance" },
  { id: "advanced", label: "Advanced" },
  { id: "binaries", label: "Binaries" },
];

export function Settings() {
  const bootstrap = useBootstrap();
  // A `?section=` deep-link (e.g. the composer's "Harnesses →" footer) sets the
  // initial section; otherwise default to General.
  const { section: requestedSection } = useSearch({ from: "/settings" });
  const [active, setActive] = useState<SectionId>(() =>
    NAV.some((n) => n.id === requestedSection) ? (requestedSection as SectionId) : "general",
  );

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
            {/* macOS checks for updates from the app menu; Windows has no such
                menu, so keep the manual "Check now" card there. */}
            {IS_WINDOWS && <UpdatesCard />}
            <AiToolCard />
            <DefaultHarnessCard settings={data.settings} />
            <HyprlayerConfigCard config={data} />
            <TabTitlesCard settings={data.settings} />
            <QuitCard settings={data.settings} />
            <Card>
              <CardTitle>Setup wizard</CardTitle>
              <p className="mb-3 text-xs text-ink-dim">
                Re-run the guided setup to reconfigure your AI tool or change your thoughts
                repository.
              </p>
              <Link to="/onboarding">
                <Button>Re-run setup</Button>
              </Link>
            </Card>
          </>
        )}
        {section === "harnesses" && (
          <HarnessesSection settings={data.settings} binaries={data.binaries} />
        )}
        {section === "git" && <GitSection settings={data.settings} />}
        {section === "notifications" && <NotificationsSection settings={data.settings} />}
        {section === "keyboard" && <KeyboardSettings settings={data.settings} />}
        {section === "appearance" && <AppearanceSection />}
        {section === "advanced" && <AdvancedSection settings={data.settings} />}
        {section === "binaries" && <BinariesCard data={data} />}
      </div>
    </div>
  );
}
