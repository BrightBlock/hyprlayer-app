import { useEffect, useState } from "react";

import type { AppSettings, BinaryStatus, OllamaSettings } from "@/lib/bindings";
import {
  useOllamaStatus,
  useSetAppSettings,
  useSetOllamaDefaultModel,
  useSetOllamaEndpoint,
  useSetOllamaModels,
} from "@/lib/queries";
import {
  Badge,
  Button,
  Card,
  CardTitle,
  DropdownSelect,
  ErrorNote,
  Field,
  Spinner,
  TextInput,
  Toggle,
  SettingRow,
} from "@/components/ui";
import { BinaryCard } from "@/features/settings/BinaryCard";

const DOCS_URL = "https://ollama.com";
const DEFAULT_ENDPOINT = "http://localhost:11434";

function sameSet(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  const set = new Set(a);
  return b.every((id) => set.has(id));
}

export function OllamaHarnessTab({
  settings,
  status,
  opencodeInstalled,
}: {
  settings: AppSettings;
  status?: BinaryStatus;
  opencodeInstalled: boolean;
}) {
  const ollama: OllamaSettings = settings.providers?.ollama ?? {
    endpoint: null,
    selectedModels: [],
    defaultModel: null,
  };
  const probe = useOllamaStatus(ollama.endpoint);
  const setEndpoint = useSetOllamaEndpoint();
  const setModels = useSetOllamaModels();
  const setDefault = useSetOllamaDefaultModel();
  const save = useSetAppSettings();

  const settingsSelected = ollama.selectedModels ?? [];
  const [pendingSelected, setPendingSelected] = useState<string[] | null>(null);
  const selected = pendingSelected ?? settingsSelected;
  useEffect(() => {
    if (pendingSelected && sameSet(pendingSelected, settingsSelected)) {
      setPendingSelected(null);
    }
  }, [pendingSelected, settingsSelected]);

  const installedModels = probe.data?.models ?? [];
  const reachable = probe.data?.reachable ?? false;
  const localBinary = status?.state.state === "ok";

  const storedEndpoint = ollama.endpoint ?? "";
  const [url, setUrl] = useState(storedEndpoint);
  useEffect(() => {
    setUrl(storedEndpoint);
  }, [storedEndpoint]);

  function toggleModel(id: string, on: boolean) {
    const set = new Set(selected);
    if (on) set.add(id);
    else set.delete(id);
    const next = [...set];
    setPendingSelected(next);
    setModels.mutate(next);
  }

  return (
    <>
      {setEndpoint.isError && (
        <ErrorNote error={setEndpoint.error} onDismiss={() => setEndpoint.reset()} />
      )}
      {setModels.isError && (
        <ErrorNote error={setModels.error} onDismiss={() => setModels.reset()} />
      )}
      {setDefault.isError && (
        <ErrorNote error={setDefault.error} onDismiss={() => setDefault.reset()} />
      )}

      <Card>
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-md border border-edge bg-panel-2 text-sm font-bold text-ink">
              ol
            </div>
            <div>
              <p className="text-sm font-semibold text-ink">Ollama</p>
              <p className="text-xs text-ink-dim">
                Run local or self-hosted models through the OpenCode harness
              </p>
            </div>
          </div>
          <a className="text-xs text-accent hover:underline" href={DOCS_URL} target="_blank" rel="noreferrer">
            Docs ↗
          </a>
        </div>
      </Card>

      {!opencodeInstalled && (
        <Card>
          <Badge tone="warn">OpenCode required</Badge>
          <p className="mt-2 text-xs text-ink-dim">
            Ollama models require OpenCode. Install it from the OpenCode tab.
          </p>
        </Card>
      )}

      <Card>
        <CardTitle>Server</CardTitle>
        <Field label="Server URL">
          <div className="flex items-center gap-2">
            <TextInput
              className="flex-1"
              value={url}
              placeholder={DEFAULT_ENDPOINT}
              spellCheck={false}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") setEndpoint.mutate(url.trim() || null);
              }}
            />
            <Button
              variant="primary"
              busy={setEndpoint.isPending}
              onClick={() => setEndpoint.mutate(url.trim() || null)}
            >
              Save
            </Button>
          </div>
        </Field>
        <p className="mt-1 text-xs text-ink-dim">
          Default <span className="font-mono">{DEFAULT_ENDPOINT}</span>. Point at any reachable host (e.g. <span className="font-mono">http://nexus:11434</span>).
        </p>

        <div className="mt-3 flex flex-wrap items-center gap-3">
          {probe.isFetching ? (
            <Badge tone="dim">Checking…</Badge>
          ) : reachable ? (
            <Badge tone="ok">Connected · {installedModels.length} models</Badge>
          ) : (
            <Badge tone="warn">Not reachable</Badge>
          )}
          {localBinary && <Badge tone="dim">Ollama CLI detected locally</Badge>}
          <button
            type="button"
            className="ml-auto flex items-center gap-1 text-xs text-ink-dim hover:text-ink"
            onClick={() => void probe.refetch()}
          >
            {probe.isFetching ? <Spinner size={12} /> : "↻"} Refresh
          </button>
        </div>
        {!reachable && !probe.isFetching && (
          <p className="mt-2 text-xs text-ink-dim">
            Can't reach Ollama. Make sure it's running (<span className="font-mono">ollama serve</span>).
          </p>
        )}
      </Card>

      {reachable && (
        <Card>
          <CardTitle>
            Models <span className="ml-1 font-normal text-ink-dim">{selected.length} selected</span>
          </CardTitle>
          {installedModels.length === 0 ? (
            <p className="mt-3 text-xs text-ink-dim">
              No models installed on this server. Pull one first, e.g.{" "}
              <span className="font-mono">ollama pull qwen2.5-coder</span>, then Refresh.
            </p>
          ) : (
            <>
              <p className="mt-1 mb-3 text-xs text-ink-dim">
                Choose which installed models appear in the composer.
              </p>
              <div className="space-y-1.5">
                {installedModels.map((m) => (
                  <div
                    key={m.id}
                    className="flex items-center justify-between gap-3 rounded-md border border-edge bg-panel-2 px-3 py-2"
                  >
                    <span className="min-w-0">
                      <span className="font-mono text-sm text-ink">{m.id}</span>
                      {(m.parameterSize || m.family) && (
                        <span className="block text-xs text-ink-dim">
                          {[m.parameterSize, m.family].filter(Boolean).join(" · ")}
                        </span>
                      )}
                    </span>
                    <Toggle
                      aria-label={m.id}
                      checked={selected.includes(m.id)}
                      onChange={(on) => toggleModel(m.id, on)}
                    />
                  </div>
                ))}
              </div>

              {selected.length > 0 && (
                <div className="mt-4 divide-y divide-edge/50 border-t border-edge/50">
                  <SettingRow
                    title="Default model"
                    control={
                      <div className="w-56">
                        <DropdownSelect
                          aria-label="Ollama default model"
                          value={ollama.defaultModel ?? ""}
                          onChange={(v) => setDefault.mutate(v || null)}
                          options={[
                            { value: "", label: "No default" },
                            ...selected.map((id) => ({ value: `ollama/${id}`, label: id })),
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
                          aria-label="Ollama Commit and Ship model"
                          value={ollama.commitModel ?? ""}
                          onChange={(v) =>
                            save.mutate({
                              ...settings,
                              providers: {
                                ...(settings.providers ?? {}),
                                ollama: { ...ollama, commitModel: v || null },
                              },
                            })
                          }
                          options={[
                            { value: "", label: "No default" },
                            ...selected.map((id) => ({ value: `ollama/${id}`, label: id })),
                          ]}
                        />
                      </div>
                    }
                  />
                </div>
              )}
            </>
          )}
        </Card>
      )}

      {status && (
        <Card>
          <CardTitle>Local install (optional)</CardTitle>
          <p className="mb-2 text-xs text-ink-dim">
            Only needed for local Ollama installs. Remote setups can ignore this.
          </p>
          <BinaryCard status={status} />
        </Card>
      )}
    </>
  );
}
