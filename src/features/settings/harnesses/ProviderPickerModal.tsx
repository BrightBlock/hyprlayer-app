import { useMemo, useState } from "react";

import type { CatalogProvider } from "@/lib/bindings";
import { isFreeOpencodeModel } from "@/lib/domain/harnessing";
import { useSetOpencodeProviderKey } from "@/lib/queries";
import { Button, Dialog, ErrorNote, TextInput } from "@/components/ui";
import { reauthSucceeded } from "@/lib/reauth";
import { Icon } from "@/components/Icon";

const FEATURED = 7;

export function ProviderPickerModal({
  open,
  onClose,
  providers,
  configured,
}: {
  open: boolean;
  onClose: () => void;
  providers: CatalogProvider[];
  configured: Set<string>;
}) {
  const [query, setQuery] = useState("");
  const [showAll, setShowAll] = useState(false);
  const [picked, setPicked] = useState<CatalogProvider | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = q
      ? providers.filter(
          (p) => p.name.toLowerCase().includes(q) || p.id.toLowerCase().includes(q),
        )
      : providers;
    return q || showAll ? list : list.slice(0, FEATURED);
  }, [providers, query, showAll]);

  function reset() {
    setQuery("");
    setShowAll(false);
    setPicked(null);
  }

  function close() {
    reset();
    onClose();
  }

  return (
    <Dialog open={open} onClose={close} title="Providers" size="xl">
      {picked ? (
        <KeyEntry provider={picked} onBack={() => setPicked(null)} onDone={close} />
      ) : (
        <>
          <div className="group/search relative mb-3">
            <span className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-dim group-focus-within/search:text-ink">
              <Icon name="search" size="sm" />
            </span>
            <TextInput
              autoFocus
              className="pl-8"
              placeholder="Search providers"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>

          <ul className="max-h-[22rem] space-y-1.5 overflow-y-auto">
            {filtered.map((p) => (
              <li key={p.id}>
                <button
                  type="button"
                  onClick={() => setPicked(p)}
                  className="flex w-full items-center justify-between rounded-md border border-edge px-3 py-2.5 text-left hover:bg-edge/40"
                >
                  <span className="flex items-center gap-2 text-body text-ink">
                    {p.name}
                  </span>
                  {configured.has(p.id) ? (
                    <span className="text-ui text-ok">configured</span>
                  ) : (
                    <Icon name="chevronRight" size="xs" className="text-ink-dim" />
                  )}
                </button>
              </li>
            ))}
            {filtered.length === 0 && (
              <li className="px-1 py-6 text-center text-body text-ink-dim">
                No providers match “{query}”.
              </li>
            )}
          </ul>

          {!query && !showAll && providers.length > FEATURED && (
            <button
              type="button"
              onClick={() => setShowAll(true)}
              className="mt-3 w-full text-center text-body text-ink-dim hover:text-ink"
            >
              View all providers ({providers.length})
            </button>
          )}
        </>
      )}
    </Dialog>
  );
}

function KeyEntry({
  provider,
  onBack,
  onDone,
}: {
  provider: CatalogProvider;
  onBack: () => void;
  onDone: () => void;
}) {
  const [key, setKey] = useState("");
  const save = useSetOpencodeProviderKey();
  const envHint = provider.env[0];
  const hasFreeModels = provider.models.some((m) => isFreeOpencodeModel(m.id));

  async function submit() {
    if (!key.trim()) return;
    await save.mutateAsync({ provider: provider.id, key: key.trim() });
    onDone();
    reauthSucceeded("opencode");
  }

  return (
    <div>
      <button
        type="button"
        onClick={onBack}
        className="mb-3 inline-flex items-center gap-1 text-ui text-ink-dim hover:text-ink"
      >
        <Icon name="chevronLeft" size="xs" /> All providers
      </button>
      <p className="text-body font-medium text-ink">{provider.name}</p>
      <p className="mt-0.5 text-ui text-ink-dim">
        {provider.models.length} model{provider.models.length === 1 ? "" : "s"} available
        {provider.doc && (
          <>
            {" · "}
            <a
              className="inline-flex items-center gap-1 text-accent hover:underline"
              href={provider.doc}
              target="_blank"
              rel="noreferrer"
            >
              Docs <Icon name="external" size="xs" />
            </a>
          </>
        )}
      </p>

      {hasFreeModels && (
        <p className="mt-3 rounded-md border border-edge bg-panel-2 px-3 py-2 text-ui text-ink-dim">
          {provider.name}’s <span className="text-ink">free</span> models need no key, so pick them
          straight from <span className="text-ink">Add models</span>. A key is only required for its
          paid models.
        </p>
      )}

      <label className="mb-1 mt-4 block text-ui font-medium text-ink-dim">
        API key{envHint ? ` (${envHint})` : ""}
      </label>
      <TextInput
        autoFocus
        type="password"
        placeholder="Paste your API key or token"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void submit();
        }}
      />
      <p className="mt-1 text-ui text-ink-dim">
        Saved to OpenCode’s <code className="font-mono">auth.json</code> (0600), and handed to
        Hyprlayer’s OpenCode server.
      </p>

      {save.isError && (
        <div className="mt-3">
          <ErrorNote error={save.error} onDismiss={() => save.reset()} />
        </div>
      )}

      <div className="mt-4 flex justify-end gap-2">
        <Button onClick={onBack}>Cancel</Button>
        <Button variant="primary" busy={save.isPending} disabled={!key.trim()} onClick={() => void submit()}>
          Save key
        </Button>
      </div>
    </div>
  );
}
