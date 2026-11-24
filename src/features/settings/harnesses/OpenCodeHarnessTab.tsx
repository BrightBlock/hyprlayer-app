import { useMemo, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import type { AppSettings, BinaryStatus, OpenCodeSettings } from "@/lib/bindings";
import {
  useInstallOpencode,
  useOpencodeAuth,
  useOpencodeSignInMethods,
  useOpencodeCatalog,
  useRedetectBinaries,
  useRefreshOpencodeCatalog,
  useRemoveOpencodeProvider,
  useSetAppSettings,
  useSetOpencodeDefaultModel,
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
  DropdownSelect,
  ErrorNote,
  Spinner,
  SettingRow,
} from "@/components/ui";
import { isSupportedOpencodeProvider } from "@/lib/domain/harnessing";
import { Icon } from "@/components/Icon";
import { BinaryCard } from "@/features/settings/BinaryCard";
import { ProviderPickerModal } from "./ProviderPickerModal";
import { ModelPickerModal } from "./ModelPickerModal";
import { OpenCodeSignInDialog } from "./OpenCodeSignInDialog";

const DOCS_URL = "https://opencode.ai/docs";

export function OpenCodeHarnessTab({
  settings,
  status,
}: {
  settings: AppSettings;
  status?: BinaryStatus;
}) {
  const catalog = useOpencodeCatalog();
  const auth = useOpencodeAuth();
  const save = useSetAppSettings();
  const setDefault = useSetOpencodeDefaultModel();
  const removeProvider = useRemoveOpencodeProvider();
  const refreshCatalog = useRefreshOpencodeCatalog();

  const [providerOpen, setProviderOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [signingIn, setSigningIn] = useState<string | null>(null);

  const providers = useMemo(() => catalog.data?.providers ?? [], [catalog.data]);
  const opencode: OpenCodeSettings = settings.providers?.opencode ?? { selectedModels: [] };
  const selectedModels = opencode.selectedModels ?? [];
  const installed = status?.state.state === "ok";
  const signInMethods = useOpencodeSignInMethods(installed);
  const methodsFor = (provider: string) =>
    (signInMethods.data ?? []).filter((m) => m.provider.toLowerCase() === provider.toLowerCase());

  const providerById = useMemo(() => {
    const map = new Map<string, (typeof providers)[number]>();
    for (const provider of providers) map.set(provider.id.toLowerCase(), provider);
    return map;
  }, [providers]);
  const configured = useMemo(
    () =>
      new Set(
        (auth.data ?? []).flatMap((entry) => {
          if (!isSupportedOpencodeProvider(entry.provider)) return [];
          const provider = providerById.get(entry.provider.toLowerCase());
          return [provider ? provider.id : entry.provider.toLowerCase()];
        }),
      ),
    [auth.data, providerById],
  );
  const providerName = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of providers) map.set(p.id.toLowerCase(), p.name);
    return map;
  }, [providers]);
  const modelName = useMemo(() => {
    const map = new Map<string, string>();
    for (const p of providers) for (const m of p.models) map.set(`${p.id}/${m.id}`, m.name);
    return map;
  }, [providers]);

  function setSelectedModels(next: string[]) {
    const clearDefault = !!opencode.defaultModel && !next.includes(opencode.defaultModel);
    save.mutate({
      ...settings,
      providers: {
        ...(settings.providers ?? {}),
        opencode: {
          ...opencode,
          selectedModels: next,
          defaultModel: clearDefault ? undefined : opencode.defaultModel,
        },
      },
    });
    if (clearDefault) {
      setDefault.mutate(null);
    }
  }

  return (
    <>
      {save.isError && <ErrorNote error={save.error} onDismiss={() => save.reset()} />}
      {setDefault.isError && (
        <ErrorNote error={setDefault.error} onDismiss={() => setDefault.reset()} />
      )}
      {auth.isError && <ErrorNote error={auth.error} />}

      <Card>
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-md border border-edge bg-panel-2 text-body font-bold text-ink">
              oc
            </div>
            <div>
              <p className={SETTINGS_CARD_TITLE}>OpenCode</p>
              <p className={SETTINGS_CARD_DESCRIPTION}>The open source AI coding agent</p>
            </div>
          </div>
          <a
            className="inline-flex items-center gap-1 text-ui text-accent hover:underline"
            href={DOCS_URL}
            target="_blank"
            rel="noreferrer"
          >
            Docs <Icon name="external" size="xs" />
          </a>
        </div>
      </Card>

      {!installed && <AdvancedCard status={status} installed={installed} defaultOpen />}

      <Card>
        <div className="flex items-center justify-between">
          <SettingsCardTitle className="mb-0">
            Providers{" "}
            <span className="ml-1 font-normal text-ink-dim">
              {configured.size} configured
            </span>
          </SettingsCardTitle>
          <Button onClick={() => setProviderOpen(true)}>
            <Icon name="add" size="sm" /> Add provider
          </Button>
        </div>

        <div className="mt-3 space-y-1.5">
          {configured.size === 0 && (
            <p className="text-ui text-ink-dim">
              No supported providers configured yet. Add one to use its models with OpenCode.
            </p>
          )}
          {(auth.data ?? []).map((e) => {
            const supported = isSupportedOpencodeProvider(e.provider);
            return (
              <div
                key={e.provider}
                className="flex items-center justify-between gap-3 rounded-md border border-edge bg-panel-2 px-3 py-2"
              >
                <div className="min-w-0">
                  <span className="block truncate text-body text-ink">
                    {providerName.get(e.provider.toLowerCase()) ?? e.provider}
                  </span>
                  {supported && e.remedy && <p className="text-ui text-warn-ink">{e.remedy}</p>}
                </div>
                <div className="flex shrink-0 items-center gap-3">
                  {supported && e.remedy && methodsFor(e.provider).length > 0 && (
                    <Button size="sm" onClick={() => setSigningIn(e.provider)}>
                      Sign in
                    </Button>
                  )}
                  <Badge tone={supported && !e.remedy ? "dim" : "warn"}>
                    {!supported ? "unsupported" : e.remedy ? "not usable" : e.kind === "signIn" ? "signed in" : "key set"}
                  </Badge>
                  <button
                    type="button"
                    aria-label={`Remove ${e.provider}`}
                    onClick={() => removeProvider.mutate(e.provider)}
                    className="text-ink-dim hover:text-bad"
                  >
                    <Icon name="close" size="sm" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      <Card>
        <div className="flex items-center justify-between">
          <SettingsCardTitle className="mb-0">
            Models <span className="ml-1 font-normal text-ink-dim">{selectedModels.length} selected</span>
          </SettingsCardTitle>
          <Button
            onClick={() => {
              refreshCatalog.mutate();
              setModelOpen(true);
            }}
            disabled={catalog.isLoading}
          >
            <Icon name="add" size="sm" /> Add models
          </Button>
        </div>

        {selectedModels.length === 0 ? (
          <p className="mt-3 text-ui text-ink-dim">
            No models selected yet. Add the models you want available in the composer.
          </p>
        ) : (
          <>
            <div className="mt-3 space-y-1.5">
              {selectedModels.map((id) => (
                <div
                  key={id}
                  className="flex items-center justify-between rounded-md border border-edge bg-panel-2 px-3 py-2"
                >
                  <span className="min-w-0">
                    <span className="text-body text-ink">{modelName.get(id) ?? id}</span>
                    <span className="block text-meta text-ink-dim">{id}</span>
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove ${id}`}
                    onClick={() => setSelectedModels(selectedModels.filter((m) => m !== id))}
                    className="text-ink-dim hover:text-bad"
                  >
                    <Icon name="close" size="sm" />
                  </button>
                </div>
              ))}
            </div>

            <div className="mt-4 divide-y divide-edge/50 border-t border-edge/50">
              <SettingRow
                title="Default model"
                control={
                  <div className="w-56">
                    <DropdownSelect
                      aria-label="OpenCode default model"
                      value={opencode.defaultModel ?? ""}
                      onChange={(v) => setDefault.mutate(v || null)}
                      options={[
                        { value: "", label: "OpenCode default" },
                        ...selectedModels.map((id) => ({
                          value: id,
                          label: modelName.get(id) ?? id,
                        })),
                      ]}
                    />
                  </div>
                }
              />
              <SettingRow
                title="Commit & Ship model"
                control={
                  <div className="w-56">
                    <DropdownSelect
                      aria-label="OpenCode Commit and Ship model"
                      value={opencode.commitModel ?? ""}
                      onChange={(v) =>
                        save.mutate({
                          ...settings,
                          providers: {
                            ...(settings.providers ?? {}),
                            opencode: { ...opencode, commitModel: v || null },
                          },
                        })
                      }
                      options={[
                        { value: "", label: "OpenCode default" },
                        ...selectedModels.map((id) => ({
                          value: id,
                          label: modelName.get(id) ?? id,
                        })),
                      ]}
                    />
                  </div>
                }
              />
            </div>
          </>
        )}
      </Card>

      {installed && <AdvancedCard status={status} installed={installed} />}

      <ProviderPickerModal
        open={providerOpen}
        onClose={() => setProviderOpen(false)}
        providers={providers}
        configured={configured}
        signInMethods={signInMethods.data ?? []}
      />
      {signingIn && (
        <OpenCodeSignInDialog
          providerName={providerName.get(signingIn.toLowerCase()) ?? signingIn}
          methods={methodsFor(signingIn)}
          onSignedIn={() => setSigningIn(null)}
          onClose={() => setSigningIn(null)}
        />
      )}
      <ModelPickerModal
        open={modelOpen}
        onClose={() => setModelOpen(false)}
        providers={providers}
        selected={selectedModels}
        onSave={setSelectedModels}
        onlyConfigured={configured.size > 0 ? configured : undefined}
      />
    </>
  );
}

function AdvancedCard({
  status,
  installed,
  defaultOpen = false,
}: {
  status?: BinaryStatus;
  installed: boolean;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const install = useInstallOpencode();
  const redetect = useRedetectBinaries();
  const refreshCatalog = useRefreshOpencodeCatalog();

  const title = installed ? "Advanced" : "Install OpenCode";

  return (
    <Card>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1.5 text-left"
      >
        <Icon name={open ? "chevronDown" : "chevronRight"} size="sm" />
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
                className="inline-flex items-center gap-1 text-ui text-ink-dim hover:text-ink"
                onClick={() => void revealItemInDir(status.path as string).catch(() => {})}
              >
                Open in Finder <Icon name="external" size="xs" />
              </button>
            )}
            <a
              className="inline-flex items-center gap-1 text-ui text-ink-dim hover:text-ink"
              href={DOCS_URL}
              target="_blank"
              rel="noreferrer"
            >
              Docs <Icon name="external" size="xs" />
            </a>
            <button
              type="button"
              className="ml-auto flex items-center gap-1 text-ui text-ink-dim hover:text-ink"
              onClick={() => {
                redetect.mutate();
                refreshCatalog.mutate();
              }}
            >
              {redetect.isPending ? <Spinner size="xs" /> : <Icon name="sync" size="xs" />} Refresh
            </button>
          </div>

          {!installed && (
            <div className="space-y-1.5">
              <Button
                variant="primary"
                disabled={install.isPending}
                onClick={() => install.mutate()}
              >
                {install.isPending ? (
                  <span className="flex items-center gap-1.5">
                    <Spinner size="xs" /> Installing OpenCode…
                  </span>
                ) : (
                  "Install OpenCode"
                )}
              </Button>
              {install.isPending && (
                <p className="text-ui text-ink-dim">
                  Downloading and installing. This can take a minute.
                </p>
              )}
              {install.isError && (
                <ErrorNote error={install.error} onDismiss={() => install.reset()} />
              )}
            </div>
          )}

          <div>
            <p className="mb-2 text-ui text-ink-dim">
              Override the auto-detected OpenCode executable.
            </p>
            {status ? (
              <BinaryCard status={status} />
            ) : (
              <p className="text-ui text-ink-dim">OpenCode status unavailable.</p>
            )}
          </div>
        </div>
      )}

    </Card>
  );
}
