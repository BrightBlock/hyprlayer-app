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
  Badge,
  Button,
  Card,
  CardTitle,
  Dialog,
  ErrorNote,
  Select,
  Spinner,
  TextInput,
} from "@/components/ui";
import { ChevronDownIcon, ChevronRightIcon, PlusIcon, XIcon } from "@/components/icons";
import { BinaryCard } from "@/features/system/BinaryCard";
import { AuthCard, AuthDetailRows } from "./AuthCard";
import { SetupTerminal } from "@/features/onboarding/SetupTerminal";
import { codexEfforts } from "@/lib/domain/models";
import { ModelMultiSelectDialog } from "./ModelMultiSelectDialog";
import { SettingRow } from "@/features/settings/parts";

const DOCS_URL = "https://developers.openai.com/codex";

/** Module-scope so an unsaved-settings render doesn't mint fresh identities and
 *  invalidate every memo keyed on them. */
const NO_MODELS: string[] = [];
const EMPTY_CODEX: CodexSettings = { selectedModels: NO_MODELS };

/** Settings → Harnesses → Codex. Auth lives in `~/.codex` (owned by
 *  `codex login`), so this tab only reads it; what it *writes* is which models
 *  the composer offers and which defaults new chats and the action agents use. */
export function CodexHarnessTab({
  settings,
  status,
}: {
  settings: AppSettings;
  status?: BinaryStatus;
}) {
  const installed = status?.state.state === "ok";
  // One probe for account + models: it spawns an app-server, so don't even try
  // without a binary — and never as two concurrent queries (see useCodexProbe).
  const probe = useCodexProbe(installed);
  const save = useSetAppSettings();
  const [pickerOpen, setPickerOpen] = useState(false);

  const codex: CodexSettings = settings.providers?.codex ?? EMPTY_CODEX;
  // Stable `[]` (not a fresh literal) so the `effortChoices` memo below actually
  // memoizes for the un-saved-settings case.
  const selectedModels = codex.selectedModels ?? NO_MODELS;
  const available = useMemo(() => probe.data?.models ?? [], [probe.data]);
  const displayName = useMemo(() => {
    const map = new Map<string, string>();
    for (const m of available) map.set(m.id, m.displayName);
    return map;
  }, [available]);
  // An empty selection means "not customised" → offer every model Codex reports,
  // the dynamic-list twin of how Claude shows all its presets until you narrow
  // them. Removing a model from that default set materialises an explicit list.
  const effectiveIds = useMemo(
    () => (selectedModels.length > 0 ? selectedModels : available.map((m) => m.id)),
    [selectedModels, available],
  );
  const usingAllDefault = selectedModels.length === 0 && available.length > 0;
  // Effort tiers differ per model (GPT-5.6-Sol has `ultra`, Luna stops at `max`),
  // so offer the union across the models actually on offer — these settings apply
  // to whichever of them a chat runs on. Falls back to the common core before the
  // list loads.
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
    // Drop any default the new selection can no longer honor, in the SAME write —
    // a separate clear could land before this one and be overwritten by the stale
    // value. Efforts go too: tiers are per-model, and Commit/Ship sends its stored
    // effort through unvalidated, so a tier no model offers would fail the turn.
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
      {save.isError && <ErrorNote error={save.error} />}

      <Card>
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-md border border-edge bg-panel-2 text-sm font-bold text-ink">
              cx
            </div>
            <div>
              <p className="text-sm font-semibold text-ink">Codex</p>
              <p className="text-xs text-ink-dim">OpenAI&apos;s coding agent</p>
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

      {/* Models */}
      <Card>
        <div className="flex items-center justify-between">
          <CardTitle className="mb-0">
            Models{" "}
            <span className="ml-1 font-normal text-ink-dim">
              {effectiveIds.length} selected
            </span>
          </CardTitle>
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
          // A probe that RAN and failed is an error, not "no models" — showing the
          // signed-out hint here would send the user to fix the wrong thing.
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
              >
                Model new chats start on.
              </SettingRow>
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
              >
                Reasoning effort new chats start on.
              </SettingRow>
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
              >
                Model the Commit and Ship agents run on.
              </SettingRow>
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
                These stages are mechanical git plumbing — a lower effort is usually enough.
              </SettingRow>
            </div>
          </>
        )}
      </Card>

      {installed && <AdvancedCard status={status} />}

      {/* Same picker OpenCode uses. Codex returns one flat model list rather
          than a provider→model catalog, so it's a single unlabelled group.
          Mounted only while open — that's what re-seeds its draft selection. */}
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

/** Keep a stored model id only while it's still in the selected list. */
function keepIfSelected(id: string | null | undefined, selected: string[]): string | null {
  return id && selected.includes(id) ? id : null;
}

/** Keep a stored effort only while some selected model still offers it. An empty
 *  set means the catalog hasn't loaded — keep the value rather than clearing it
 *  on a probe that simply hasn't answered yet. */
function keepIfOffered(effort: string | null | undefined, offered: Set<string>): string | null {
  if (!effort) return null;
  return offered.size === 0 || offered.has(effort) ? effort : null;
}

/** The one control shape all four Codex defaults use: a fixed-width select whose
 *  empty option means "let Codex decide". */
function DefaultSelect({
  label,
  value,
  onChange,
  children,
}: {
  label: string;
  value: string | null | undefined;
  onChange: (value: string | null) => void;
  children: React.ReactNode;
}) {
  return (
    <div className="w-56">
      <Select
        aria-label={label}
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
      >
        <option value="">Codex default</option>
        {children}
      </Select>
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
    <DefaultSelect label={label} value={value} onChange={onChange}>
      {ids.map((id) => (
        <option key={id} value={id}>
          {names.get(id) ?? id}
        </option>
      ))}
    </DefaultSelect>
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
    <DefaultSelect label={label} value={value} onChange={onChange}>
      {choices.map((e) => (
        <option key={e.key} value={e.key}>
          {e.label}
        </option>
      ))}
    </DefaultSelect>
  );
}

/**
 * Codex authentication — the same two-tile shape as the Claude tab: pick a
 * method, configure it below.
 *
 * The tiles are LOCAL UI state, not a stored setting (Claude persists
 * `authMode` because it injects credentials at spawn). Codex owns its own
 * credential store — `$CODEX_HOME/auth.json`, written by `codex login` — so
 * whichever method you last used is simply what `account/read` reports, and the
 * tiles just choose how you sign in next. Nothing is stored by Hyprlayer: no
 * plaintext key in `settings.json`, and no `OPENAI_API_KEY` at spawn (the
 * app-server ignores the env var; only `auth.json` counts).
 */
function AuthenticationCard({ probe }: { probe: ReturnType<typeof useCodexProbe> }) {
  const signedInMethod = probe.data?.auth.method;
  // Default the tile to whatever's already in use, else the subscription flow.
  const [mode, setMode] = useState<"chatgpt" | "apiKey">(
    signedInMethod === "apiKey" ? "apiKey" : "chatgpt",
  );
  // Re-sync once the probe lands (it resolves after first render).
  const [seenMethod, setSeenMethod] = useState(signedInMethod);
  if (signedInMethod !== seenMethod) {
    setSeenMethod(signedInMethod);
    if (signedInMethod) setMode(signedInMethod === "apiKey" ? "apiKey" : "chatgpt");
  }

  return (
    <Card>
      <CardTitle>Authentication</CardTitle>
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

/** Status + `codex login` (the browser OAuth flow). Mirrors Claude's CliAuthPanel. */
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
        className="max-w-2xl"
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

/** API-key sign-in. Mirrors Claude's ApiKeyAuthPanel, minus the base-URL and
 *  Bedrock rows — those configure env vars Claude's spawn injects, whereas Codex
 *  reads neither (its own `config.toml` covers custom providers). */
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

      {login.isError && <ErrorNote error={login.error} />}
    </div>
  );
}

/** Connected badge + refresh/sign-out, shared by both Codex auth panels. */
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
      {logout.isError && <ErrorNote error={logout.error} />}
      {probe.isError && <ErrorNote error={probe.error} />}
    </>
  );
}

/** Friendly name for the `Account.type` Codex reports. */
function authMethodLabel(method: string | null | undefined): string | null {
  if (!method) return null;
  if (method === "chatgpt") return "ChatGPT";
  if (method === "apiKey") return "API key";
  if (method === "amazonBedrock") return "Amazon Bedrock";
  return method;
}

/** Before install this card is the primary setup CTA (so it starts open and is
 *  named for the job); once installed it's the collapsed "Advanced" drawer. Both
 *  follow from `status`, so that's the only prop. */
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
        <span className="text-sm font-semibold text-ink">{title}</span>
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
            // No in-app installer: Codex ships via npm/Homebrew and its sign-in is
            // an interactive browser flow, so pointing at the docs beats a
            // half-working one-click path.
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
