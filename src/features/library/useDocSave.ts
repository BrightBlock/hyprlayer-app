import { useEffect, useRef, useState } from "react";
import { useBlocker } from "@tanstack/react-router";

import type { DocContent, FrontmatterEditDto } from "@/lib/bindings";
import { isAppError } from "@/lib/ipc";
import { fetchDocFresh, useSaveDocParts } from "@/lib/queries";
import { setField as setDraftField, toPatch, type FmDraft, type SchemaKey } from "./fmDraft";

const SAVED_FLASH_MS = 1800;

// CodeMirror hands back LF; compare against that, not the file's own EOLs.
function lf(text: string): string {
  return text.replace(/\r\n/g, "\n");
}

type Base = { body: string; rawBody: string; mtimeMs: number };

function baseOf(doc: DocContent): Base {
  return { body: lf(doc.body), rawBody: doc.body, mtimeMs: doc.mtimeMs ?? 0 };
}

function docOf(search: unknown): string | undefined {
  return (search as { doc?: string }).doc;
}

export function useDocSave(id: string, doc: DocContent) {
  const saveParts = useSaveDocParts(id);
  const [base, setBase] = useState(() => baseOf(doc));
  const [body, setBody] = useState(base.body);
  const [draft, setDraft] = useState<FmDraft>({});
  const [raw, setRaw] = useState<string | null>(null);
  const [justSaved, setJustSaved] = useState(false);

  const dirty =
    body !== base.body ||
    Object.keys(draft).length > 0 ||
    (raw !== null && raw !== lf(doc.frontmatter ?? ""));

  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  function reset(fresh: DocContent) {
    const nextBase = baseOf(fresh);
    setBase(nextBase);
    setBody(nextBase.body);
    setDraft({});
    setRaw(null);
    saveParts.reset();
  }

  // Adopt outside edits only while clean; a dirty doc meets them as doc_conflict on save.
  useEffect(() => {
    if (!dirty && doc.mtimeMs !== base.mtimeMs) reset(doc);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doc]);

  const savedTimer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (savedTimer.current) window.clearTimeout(savedTimer.current);
    },
    [],
  );

  function flashSaved() {
    setJustSaved(true);
    if (savedTimer.current) window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setJustSaved(false), SAVED_FLASH_MS);
  }

  async function save(): Promise<boolean> {
    if (!dirty) return true;
    if (saveParts.isPending) return false;
    const sent = body === base.body ? base.rawBody : body; // untouched body goes back byte-for-byte
    const frontmatter: FrontmatterEditDto =
      raw !== null ? { kind: "raw", text: raw } : { kind: "patch", patch: toPatch(draft) };
    try {
      await saveParts.mutateAsync({ body: sent, frontmatter, baseMtimeMs: base.mtimeMs });
    } catch {
      return false;
    }
    const fresh = await fetchDocFresh(id);
    setBase(baseOf(fresh));
    setDraft({});
    setRaw(null);
    flashSaved();
    return true;
  }

  function setField(key: SchemaKey, next: string | string[]) {
    setDraft((d) => setDraftField(d, doc.meta.fm, doc.fmKeys, key, next));
  }

  function reloadFromDisk() {
    return fetchDocFresh(id).then(reset);
  }

  const blocker = useBlocker({
    shouldBlockFn: ({ current, next }) =>
      dirtyRef.current && (current.pathname !== next.pathname || docOf(current.search) !== docOf(next.search)),
    withResolver: true,
    // The default (true) arms beforeunload regardless of shouldBlockFn, so a clean doc would still prompt on reload.
    enableBeforeUnload: false,
  });

  return {
    body,
    setBody,
    draft,
    setField,
    raw,
    setRaw,
    dirty,
    saving: saveParts.isPending,
    justSaved,
    save,
    conflict: isAppError(saveParts.error) && saveParts.error.code === "doc_conflict",
    reloadFromDisk,
    error: saveParts.error,
    blocker,
  };
}
