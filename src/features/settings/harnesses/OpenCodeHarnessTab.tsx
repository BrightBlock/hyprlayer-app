import { useMemo, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import type { AppSettings, BinaryStatus, OpenCodeSettings } from "@/lib/bindings";
import {
  useInstallOpencode,
  useOpencodeAuth,
  useOpencodeCatalog,
  useRedetectBinaries,
  useRefreshOpencodeCatalog,
  useRemoveOpencodeProvider,
  useSetAppSettings,
  useSetOpencodeDefaultModel,
} from "@/lib/queries";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  DropdownSelect,
  ErrorNote,
  Spinner,
  SettingRow,
} from "@/components/ui";
import { isSupportedOpencodeProvider } from "@/lib/domain/harnessing";
import { ChevronDownIcon, ChevronRightIcon, PlusIcon, XIcon } from "@/components/icons";
import { BinaryCard } from "@/features/system/BinaryCard";
import { ProviderPickerModal } from "./ProviderPickerModal";
import { ModelPickerModal } from "./ModelPickerModal";

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

  const providers = useMemo(() => catalog.data?.providers ?? [], [catalog.data]);
  const opencode: OpenCodeSettings = settings.providers?.opencode ?? { selectedModels: [] };
  const selectedModels = opencode.selectedModels ?? [];
  const installed = status?.state.state === "ok";

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
            <div className="flex h-9 w-9 items-center justify-center rounded-md border border-edge bg-panel-2 text-sm font-bold text-ink">
              oc
            </div>
            <div>
              <p className="text-sm font-semibold text-ink">OpenCode</p>
              <p className="text-xs text-ink-dim">The open source AI coding agent</p>
            </div>
          </div>
          <a className="text-xs text-accent hover:underline" href={DOCS_URL} target="_blank" rel="noreferrer">
            Docs ↗
          </a>
        </div>
      </Card>

      {!installed && <AdvancedCard status={status} installed={installed} defaultOpen />}

      <Card>
        <div className="flex items-center justify-between">
          <CardTitle className="mb-0">
            Providers{" "}
            <span className="ml-1 font-normal text-ink-dim">
              {configured.size} configured
            </span>
          </CardTitle>
          <Button onClick={() => setProviderOpen(true)}>
            <PlusIcon size={14} /> Add provider
          </Button>
        </div>

        <div className="mt-3 space-y-1.5">
          {configured.size === 0 && (
            <p className="text-xs text-ink-dim">
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
                <span className="min-w-0 truncate text-sm text-ink">
                  {providerName.get(e.provider.toLowerCase()) ?? e.provider}
                </span>
                <div className="flex shrink-0 items-center gap-3">
                  <Badge tone={supported ? "dim" : "warn"}>
                    {supported ? "key set" : "unsupported"}
                  </Badge>
                  <button
                    type="button"
                    aria-label={`Remove ${e.provider}`}
                    onClick={() => removeProvider.mutate(e.provider)}
                    className="text-ink-dim hover:text-bad"
                  >
                    <XIcon />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      <Card>
        <div className="flex items-center justify-between">
          <CardTitle className="mb-0">
            Models <span className="ml-1 font-normal text-ink-dim">{selectedModels.length} selected</span>
          </CardTitle>
          <Button
            onClick={() => {
              refreshCatalog.mutate();
              setModelOpen(true);
            }}
            disabled={catalog.isLoading}
          >
            <PlusIcon size={14} /> Add models
          </Button>
        </div>

        {selectedModels.length === 0 ? (
          <p className="mt-3 text-xs text-ink-dim">
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
                    <span className="text-sm text-ink">{modelName.get(id) ?? id}</span>
                    <span className="block font-mono text-xs text-ink-dim">{id}</span>
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove ${id}`}
                    onClick={() => setSelectedModels(selectedModels.filter((m) => m !== id))}
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
      />
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
            <a className="text-xs text-ink-dim hover:text-ink" href={DOCS_URL} target="_blank" rel="noreferrer">
              Docs ↗
            </a>
            <button
              type="button"
              className="ml-auto flex items-center gap-1 text-xs text-ink-dim hover:text-ink"
              onClick={() => {
                redetect.mutate();
                refreshCatalog.mutate();
              }}
            >
              {redetect.isPending ? <Spinner size={12} /> : "↻"} Refresh
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
                    <Spinner size={12} /> Installing OpenCode…
                  </span>
                ) : (
                  "Install OpenCode"
                )}
              </Button>
              {install.isPending && (
                <p className="text-xs text-ink-dim">
                  Downloading and installing. This can take a minute.
                </p>
              )}
              {install.isError && (
                <ErrorNote error={install.error} onDismiss={() => install.reset()} />
              )}
            </div>
          )}

          <div>
            <p className="mb-2 text-xs text-ink-dim">
              Override the auto-detected OpenCode executable.
            </p>
            {status ? (
              <BinaryCard status={status} />
            ) : (
              <p className="text-xs text-ink-dim">OpenCode status unavailable.</p>
            )}
          </div>
        </div>
      )}

    </Card>
  );
}
