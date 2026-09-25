import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";

import { cx, ErrorNote, Popover } from "@/components/ui";
import { Icon } from "@/components/Icon";
import type { DocMeta } from "@/lib/bindings";
import { fuzzyScore } from "@/lib/fuzzy";
import { resolveRelatedOnce, useDocs } from "@/lib/queries";
import { docTitle } from "./text";

const MAX_RESULTS = 20;

type Props = {
  id: string;
  excludeIds: readonly string[];
  onPick: (value: string) => void;
};

// The picker never links the wrong doc: it resolves the candidate forms in
// order and writes the first one that actually resolves back to the pick.
async function resolveWrittenValue(id: string, meta: DocMeta): Promise<string | null> {
  const stem = meta.relPath.split("/").pop()!.replace(/\.md$/i, "");
  const candidates = [stem, meta.relPath, meta.id];
  const resolved = await resolveRelatedOnce(id, candidates);
  const winner = resolved.find((l) => l.target.kind === "doc" && l.target.id === meta.id);
  return winner?.raw ?? null;
}

export function RelatedPicker({ id, excludeIds, onPick }: Props) {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      setError(null);
    }
  }, [open]);

  const docsQuery = useDocs({});
  const excluded = useMemo(() => new Set([id, ...excludeIds]), [id, excludeIds]);
  const results = useMemo(() => {
    const candidates = (docsQuery.data?.items ?? []).filter((meta) => !excluded.has(meta.id));
    return candidates
      .map((meta) => ({ meta, score: fuzzyScore(`${docTitle(meta)} ${meta.relPath}`, query) }))
      .filter((r): r is { meta: DocMeta; score: number } => r.score !== null)
      .sort((a, b) => b.score - a.score)
      .slice(0, MAX_RESULTS)
      .map((r) => r.meta);
  }, [docsQuery.data, excluded, query]);

  async function pick(meta: DocMeta) {
    setPending(true);
    setError(null);
    try {
      const value = await resolveWrittenValue(id, meta);
      if (value) {
        onPick(value);
        setOpen(false);
      }
    } catch (e) {
      setError(e);
    } finally {
      setPending(false);
    }
  }

  function onSearchKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (results.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => (a + 1) % results.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => (a - 1 + results.length) % results.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const meta = results[Math.min(active, results.length - 1)];
      if (meta) void pick(meta);
    }
  }

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="dialog"
        aria-expanded={open}
        className="mt-1.5 inline-flex items-center gap-1.5 text-ui text-ink-dim hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
      >
        <Icon name="linkDoc" size="xs" />
        Link a thought
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchorRef}
        trapFocus
        initialFocusRef={inputRef}
        aria-label="Link a thought"
        className="w-[320px] p-0"
      >
        <div className="border-b border-edge p-2">
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={onSearchKeyDown}
            disabled={pending}
            placeholder="Search titles and paths…"
            className="h-7 w-full rounded-control border border-edge bg-surface px-2 text-ui text-ink placeholder:text-ink-dim focus:outline-none disabled:opacity-60"
          />
        </div>
        {error != null && (
          <div className="p-2">
            <ErrorNote error={error} onDismiss={() => setError(null)} />
          </div>
        )}
        <ul role="listbox" aria-label="Matching documents" className="max-h-64 overflow-y-auto py-1">
          {results.length === 0 ? (
            <li className="px-3 py-2 text-ui text-ink-dim">No matches</li>
          ) : (
            results.map((meta, i) => (
              <li key={meta.id}>
                <button
                  type="button"
                  role="option"
                  aria-selected={i === active}
                  disabled={pending}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => void pick(meta)}
                  className={cx(
                    "flex w-full items-center gap-2 truncate px-3 py-1.5 text-left text-body disabled:cursor-not-allowed disabled:opacity-60",
                    i === active ? "bg-edge/60 text-ink" : "text-ink hover:bg-edge/50",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate">{docTitle(meta)}</span>
                  <span className="shrink-0 truncate text-ui text-ink-dim/70">{meta.relPath}</span>
                </button>
              </li>
            ))
          )}
        </ul>
      </Popover>
    </>
  );
}
