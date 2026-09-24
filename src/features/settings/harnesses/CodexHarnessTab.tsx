import { useMemo, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import type { AppSettings, BinaryStatus, CodexSettings } from "@/lib/bindings";
import {
  useCodexLoginWithApiKey,
  useCodexLogout,
  useCodexProbe,
  useRedetectBinaries,
  useSetAppSettings,
} from "@/lib/queries";
import {
  SETTINGS_CARD_DESCRIPTION,
  SETTINGS_CARD_TITLE,
  SettingsCardTitle,
} from "@/components/SettingsCard";
import {
  Badge,
  Button,
  Card,
  Dialog,
  DropdownSelect,
  ErrorNote,
  Spinner,
  type DropdownOption,
  TextInput,
  SettingRow,
} from "@/components/ui";
import { ChevronDownIcon, ChevronRightIcon, PlusIcon, XIcon } from "@/components/icons";
import { BinaryCard } from "@/features/settings/BinaryCard";
import { AuthCard, AuthDetailRows } from "./AuthCard";
import { SetupTerminal } from "@/features/onboarding/SetupTerminal";
import { codexEfforts } from "@/lib/domain/models";
import { ModelMultiSelectDialog } from "./ModelMultiSelectDialog";

const DOCS_URL = "https://developers.openai.com/codex";

const NO_MODELS: string[] = [];
const EMPTY_CODEX: CodexSettings = { selectedModels: NO_MODELS };

export function CodexHarnessTab({
  settings,
  status,
}: {
  settings: AppSettings;
  status?: BinaryStatus;
}) {
  const installed = status?.state.state === "ok";
  const probe = useCodexProbe(installed);
  const save = useSetAppSettings();
  const [pickerOpen, setPickerOpen] = useState(false);

  const codex: CodexSettings = settings.providers?.codex ?? EMPTY_CODEX;
  const selectedModels = codex.selectedModels ?? NO_MODELS;
  const available = useMemo(() => probe.data?.models ?? [], [probe.data]);
  const displayName = useMemo(() => {
    const map = new Map<string, string>();
    for (const m of available) map.set(m.id, m.displayName);
    return map;
  }, [available]);
  const effectiveIds = useMemo(
    () => (selectedModels.length > 0 ? selectedModels : available.map((m) => m.id)),
    [selectedModels, available],
  );
  const usingAllDefault = selectedModels.length === 0 && available.length > 0;
  const effortChoices = useMemo(
    () =>
      codexEfforts(
        available
          .filter((m) => effectiveIds.includes(m.id))
          .flatMap((m) => m.supportedEfforts)
          .filter((e, i, all) => all.indexOf(e) === i),
      ),
    [available, effectiveIds],
  );

  function patchCodex(next: Partial<CodexSettings>) {
    save.mutate({
      ...settings,
      providers: { ...(settings.providers ?? {}), codex: { ...codex, ...next } },
    });
  }

  function setSelectedModels(next: string[]) {
    const efforts = new Set(
      available.filter((m) => next.includes(m.id)).flatMap((m) => m.supportedEfforts),
    );
    patchCodex({
      selectedModels: next,
      defaultModel: keepIfSelected(codex.defaultModel, next),
      commitModel: keepIfSelected(codex.commitModel, next),
      defaultEffort: keepIfOffered(codex.defaultEffort, efforts),
      commitEffort: keepIfOffered(codex.commitEffort, efforts),
    });
  }

  return (
    <>
      {save.isError && <ErrorNote error={save.error} onDismiss={() => save.reset()} />}

      <Card>
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-md border border-edge bg-panel-2 text-sm font-bold text-ink">
              cx
            </div>
            <div>
              <p className={SETTINGS_CARD_TITLE}>Codex</p>
              <p className={SETTINGS_CARD_DESCRIPTION}>OpenAI&apos;s coding agent</p>
            </div>
          </div>
          <a
            className="text-xs text-accent hover:underline"
            href={DOCS_URL}
            target="_blank"
            rel="noreferrer"
          >
            Docs ↗
          </a>
        </div>
      </Card>

      {!installed && <AdvancedCard status={status} />}

      {installed && <AuthenticationCard probe={probe} />}

      <Card>
        <div className="flex items-center justify-between">
          <SettingsCardTitle className="mb-0">
            Models{" "}
            <span className="ml-1 font-normal text-ink-dim">
              {effectiveIds.length} selected
            </span>
          </SettingsCardTitle>
          <div className="flex items-center gap-2">
            {!usingAllDefault && selectedModels.length > 0 && (
              <button
                type="button"
                className="rounded px-2 py-1 text-xs text-ink-dim hover:bg-edge/60 hover:text-ink"
                onClick={() => patchCodex({ selectedModels: NO_MODELS })}
              >
                Reset to default
              </button>
            )}
            <Button onClick={() => setPickerOpen(true)} disabled={!installed || probe.isLoading}>
              <PlusIcon size={14} /> Add models
            </Button>
          </div>
        </div>

        {!installed ? (
          <p className="mt-3 text-xs text-ink-dim">Install Codex to choose models.</p>
        ) : probe.isLoading ? (
          <p className="mt-3 flex items-center gap-1.5 text-xs text-ink-dim">
            <Spinner size={12} /> Loading models from Codex…
          </p>
        ) : probe.isError ? (
          <div className="mt-3">
            <ErrorNote error={probe.error} />
          </div>
        ) : available.length === 0 ? (
          <p className="mt-3 text-xs text-ink-dim">
            Codex didn&apos;t return any models. Sign in with{" "}
            <code className="font-mono">codex login</code>, then refresh.
          </p>
        ) : (
          <>
            {usingAllDefault && (
              <p className="mt-3 text-xs text-ink-dim">
                Every model Codex offers is available in the composer by default. Remove any you
                don&apos;t want, or reset to restore the full list.
              </p>
            )}
            <div className="mt-3 space-y-1.5">
              {effectiveIds.map((id) => (
                <div
                  key={id}
                  className="flex items-center justify-between rounded-md border border-edge bg-panel-2 px-3 py-2"
                >
                  <span className="min-w-0">
                    <span className="text-sm text-ink">{displayName.get(id) ?? id}</span>
                    <span className="block font-mono text-xs text-ink-dim">{id}</span>
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove ${id}`}
                    onClick={() => setSelectedModels(effectiveIds.filter((m) => m !== id))}
                    className="text-ink-dim hover:text-bad"
                  >
                    <XIcon />
                  </button>
                </div>
              ))}
            </div>

            <div className="mt-4 divide-y divide-edge/50 border-t border-edge/50">
              <SettingRow
                title="Default model"
                control={
                  <ModelSelect
                    label="Codex default model"
                    value={codex.defaultModel}
                    ids={effectiveIds}
                    names={displayName}
                    onChange={(v) => patchCodex({ defaultModel: v })}
                  />
                }
              />
              <SettingRow
                title="Default reasoning effort"
                control={
                  <EffortSelect
                    label="Codex default reasoning effort"
                    value={codex.defaultEffort}
                    choices={effortChoices}
                    onChange={(v) => patchCodex({ defaultEffort: v })}
                  />
                }
              />
              <SettingRow
                title="Commit &amp; Ship model"
                control={
                  <ModelSelect
                    label="Codex Commit and Ship model"
                    value={codex.commitModel}
                    ids={effectiveIds}
                    names={displayName}
                    onChange={(v) => patchCodex({ commitModel: v })}
                  />
                }
              />
              <SettingRow
                title="Commit &amp; Ship effort"
                control={
                  <EffortSelect
                    label="Codex Commit and Ship reasoning effort"
                    value={codex.commitEffort}
                    choices={effortChoices}
                    onChange={(v) => patchCodex({ commitEffort: v })}
                  />
                }
              >
                Git plumbing, so a low effort is enough.
              </SettingRow>
            </div>
          </>
        )}
      </Card>

      {installed && <AdvancedCard status={status} />}

      {pickerOpen && (
        <ModelMultiSelectDialog
          open
          onClose={() => setPickerOpen(false)}
          onSave={setSelectedModels}
          selected={effectiveIds}
          searchPlaceholder="Filter Codex models"
          emptyLabel="Codex returned no models. Sign in with `codex login`, then refresh."
          groups={[
            { key: "codex", models: available.map((m) => ({ id: m.id, name: m.displayName })) },
          ]}
        />
      )}
    </>
  );
}

function keepIfSelected(id: string | null | undefined, selected: string[]): string | null {
  return id && selected.includes(id) ? id : null;
}

function keepIfOffered(effort: string | null | undefined, offered: Set<string>): string | null {
  if (!effort) return null;
  return offered.size === 0 || offered.has(effort) ? effort : null;
}

function DefaultSelect({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string | null | undefined;
  onChange: (value: string | null) => void;
  options: DropdownOption[];
}) {
  return (
    <div className="w-56">
      <DropdownSelect
        aria-label={label}
        value={value ?? ""}
        onChange={(v) => onChange(v || null)}
        options={[{ value: "", label: "Codex default" }, ...options]}
      />
    </div>
  );
}

function ModelSelect({
  label,
  value,
  ids,
  names,
  onChange,
}: {
  label: string;
  value: string | null | undefined;
  ids: string[];
  names: Map<string, string>;
  onChange: (value: string | null) => void;
}) {
  return (
    <DefaultSelect
      label={label}
      value={value}
      onChange={onChange}
      options={ids.map((id) => ({ value: id, label: names.get(id) ?? id }))}
    />
  );
}

function EffortSelect({
  label,
  value,
  choices,
  onChange,
}: {
  label: string;
  value: string | null | undefined;
  choices: { key: string; label: string }[];
  onChange: (value: string | null) => void;
}) {
  return (
    <DefaultSelect
      label={label}
      value={value}
      onChange={onChange}
      options={choices.map((e) => ({ value: e.key, label: e.label }))}
    />
  );
}

function AuthenticationCard({ probe }: { probe: ReturnType<typeof useCodexProbe> }) {
  const signedInMethod = probe.data?.auth.method;
  const [mode, setMode] = useState<"chatgpt" | "apiKey">(
    signedInMethod === "apiKey" ? "apiKey" : "chatgpt",
  );
  const [seenMethod, setSeenMethod] = useState(signedInMethod);
  if (signedInMethod !== seenMethod) {
    setSeenMethod(signedInMethod);
    if (signedInMethod) setMode(signedInMethod === "apiKey" ? "apiKey" : "chatgpt");
  }

  return (
    <Card>
      <SettingsCardTitle>Authentication</SettingsCardTitle>
      <div className="mt-2 grid grid-cols-2 gap-3">
        <AuthCard
          label="ChatGPT"
          sublabel="Sign in with your ChatGPT plan"
          selected={mode === "chatgpt"}
          onSelect={() => setMode("chatgpt")}
        />
        <AuthCard
          label="API key"
          sublabel="Use an OpenAI API key (enterprise)"
          selected={mode === "apiKey"}
          onSelect={() => setMode("apiKey")}
        />
      </div>

      <div className="mt-4">
        {mode === "chatgpt" ? <ChatgptAuthPanel probe={probe} /> : <CodexApiKeyPanel probe={probe} />}
      </div>
    </Card>
  );
}

function ChatgptAuthPanel({ probe }: { probe: ReturnType<typeof useCodexProbe> }) {
  const [loginOpen, setLoginOpen] = useState(false);
  const data = probe.data?.auth;
  const signedIn = Boolean(data?.method);

  const rows: Array<[string, string | null | undefined]> = [
    ["Method", authMethodLabel(data?.method)],
    ["Account", data?.email],
    ["Plan", data?.plan],
  ];

  return (
    <div>
      <AuthStatusRow probe={probe} />
      {signedIn && <AuthDetailRows rows={rows} />}

      <div className="mt-3">
        <Button onClick={() => setLoginOpen(true)}>▶ Run codex login</Button>
      </div>

      <Dialog
        open={loginOpen}
        onClose={() => setLoginOpen(false)}
        title="Sign in to Codex"
        size="2xl"
      >
        <p className="mb-3 text-xs text-ink-dim">
          Runs <code className="font-mono">codex login</code> in a terminal; it opens a
          browser to finish signing in.
        </p>
        <div className="relative h-80 overflow-hidden rounded-lg border border-edge bg-surface">
          <SetupTerminal
            shellCommand="codex login"
            onExit={(code) => {
              if (code === 0) {
                void probe.refetch();
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

function CodexApiKeyPanel({ probe }: { probe: ReturnType<typeof useCodexProbe> }) {
  const [key, setKey] = useState("");
  const login = useCodexLoginWithApiKey();
  const submit = () => key.trim() && login.mutate(key, { onSuccess: () => setKey("") });

  return (
    <div className="space-y-3">
      <AuthStatusRow probe={probe} />

      <div>
        <label className="mb-1 block text-xs font-medium text-ink-dim">OpenAI API key</label>
        <div className="flex gap-2">
          <TextInput
            type="password"
            autoComplete="off"
            placeholder="sk-…"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
          <Button
            variant="primary"
            className="shrink-0"
            busy={login.isPending}
            disabled={!key.trim()}
            onClick={submit}
          >
            Sign in
          </Button>
        </div>
        <p className="mt-1 text-xs text-ink-dim">
          Handed to <code className="font-mono">codex login --with-api-key</code> and stored by
          Codex in <code className="font-mono">~/.codex/auth.json</code>. Hyprlayer keeps no copy.
        </p>
      </div>

      {login.isError && <ErrorNote error={login.error} onDismiss={() => login.reset()} />}
    </div>
  );
}

function AuthStatusRow({ probe }: { probe: ReturnType<typeof useCodexProbe> }) {
  const logout = useCodexLogout();
  const signedIn = Boolean(probe.data?.auth.method);

  return (
    <>
      <div className="mb-2 flex items-center justify-between">
        {probe.isLoading ? (
          <span className="flex items-center gap-2 text-xs text-ink-dim">
            <Spinner size={12} /> Checking…
          </span>
        ) : signedIn ? (
          <Badge tone="ok">Connected</Badge>
        ) : (
          <Badge tone="warn">Not signed in</Badge>
        )}
        <div className="flex items-center gap-3">
          {signedIn && (
            <button
              type="button"
              disabled={logout.isPending}
              onClick={() => logout.mutate()}
              className="text-xs text-ink-dim hover:text-ink disabled:opacity-50"
            >
              {logout.isPending ? "Signing out…" : "Sign out"}
            </button>
          )}
          <button
            type="button"
            onClick={() => void probe.refetch()}
            className="text-xs text-ink-dim hover:text-ink"
          >
            ↻ Refresh
          </button>
        </div>
      </div>
      {logout.isError && <ErrorNote error={logout.error} onDismiss={() => logout.reset()} />}
      {probe.isError && <ErrorNote error={probe.error} />}
    </>
  );
}

function authMethodLabel(method: string | null | undefined): string | null {
  if (!method) return null;
  if (method === "chatgpt") return "ChatGPT";
  if (method === "apiKey") return "API key";
  if (method === "amazonBedrock") return "Amazon Bedrock";
  return method;
}

function AdvancedCard({ status }: { status?: BinaryStatus }) {
  const installed = status?.state.state === "ok";
  const [open, setOpen] = useState(!installed);
  const redetect = useRedetectBinaries();

  const title = installed ? "Advanced" : "Install Codex";

  return (
    <Card>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1.5 text-left"
      >
        {open ? <ChevronDownIcon /> : <ChevronRightIcon size={14} />}
        <span className={SETTINGS_CARD_TITLE}>{title}</span>
      </button>

      {open && (
        <div className="mt-3 space-y-3">
          <div className="flex flex-wrap items-center gap-3">
            {installed ? (
              <Badge tone="ok">Installed {status?.version ? `v${status.version}` : ""}</Badge>
            ) : (
              <Badge tone="warn">Not installed</Badge>
            )}
            {installed && status?.path && (
              <button
                type="button"
                className="text-xs text-ink-dim hover:text-ink"
                onClick={() => void revealItemInDir(status.path as string).catch(() => {})}
              >
                Open in Finder ↗
              </button>
            )}
            <a
              className="text-xs text-ink-dim hover:text-ink"
              href={DOCS_URL}
              target="_blank"
              rel="noreferrer"
            >
              Docs ↗
            </a>
            <button
              type="button"
              className="ml-auto flex items-center gap-1 text-xs text-ink-dim hover:text-ink"
              onClick={() => redetect.mutate()}
            >
              {redetect.isPending ? <Spinner size={12} /> : "↻"} Refresh
            </button>
          </div>

          {!installed && (
            <p className="text-xs text-ink-dim">
              Install with <code className="font-mono">npm i -g @openai/codex</code> or{" "}
              <code className="font-mono">brew install codex</code>, then run{" "}
              <code className="font-mono">codex login</code> and hit Refresh.
            </p>
          )}

          <div>
            <p className="mb-2 text-xs text-ink-dim">
              Override the auto-detected Codex executable.
            </p>
            {status ? (
              <BinaryCard status={status} />
            ) : (
              <p className="text-xs text-ink-dim">Codex status unavailable.</p>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}
