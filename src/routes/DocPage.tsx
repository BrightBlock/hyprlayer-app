import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";

import { Badge, Button, ErrorNote, PageError, PageSpinner } from "../components/ui";
import { FrontmatterCard } from "../features/library/FrontmatterCard";
import { FrontmatterForm } from "../features/library/FrontmatterForm";
import { MarkdownPreview } from "../features/library/MarkdownPreview";
import { splitFrontmatter } from "../features/library/text";
import { useDoc, usePatchDocFrontmatter, useRevealDoc, useSaveDoc } from "../lib/queries";

const markdownExtensions = [markdown()];

function shortPath(path: string): string {
  const parts = path.split("/");
  return parts.slice(Math.max(0, parts.length - 4)).join("/");
}

export function DocPage() {
  const search = useSearch({ from: "/library/doc" });
  const navigate = useNavigate({ from: "/library/doc" });
  const doc = useDoc(search.id);
  const saveDoc = useSaveDoc(search.id);
  const patchFrontmatter = usePatchDocFrontmatter(search.id);
  const revealDoc = useRevealDoc();
  const [content, setContent] = useState("");
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    if (!doc.data || dirty) return;
    setContent(doc.data.content);
  }, [doc.data, dirty]);

  const body = useMemo(() => splitFrontmatter(content).body, [content]);
  const mode = search.mode;
  const meta = doc.data?.meta;
  const mtimeMs = doc.data?.mtimeMs ?? 0;

  if (!search.id) {
    return (
      <PageError
        error={{ code: "doc_missing", message: "No document id was supplied.", remedy: "Open a document from the library." }}
      />
    );
  }

  if (doc.isLoading) return <PageSpinner />;
  if (doc.isError) return <PageError error={doc.error} />;

  if (!meta) {
    return null;
  }

  function save() {
    if (!dirty || !doc.data) return;
    saveDoc.mutate(
      { content, baseMtimeMs: mtimeMs },
      {
        onSuccess: () => {
          setDirty(false);
        },
      },
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="border-b border-edge bg-panel px-5 py-3">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <Link to="/library" className="text-xs text-ink-dim hover:text-ink">
              Library
            </Link>
            <div className="mt-1 flex min-w-0 items-center gap-2">
              <h1 className="truncate text-lg font-semibold">{meta.fm.title ?? shortPath(meta.relPath)}</h1>
              {dirty && <Badge tone="warn">unsaved</Badge>}
            </div>
            <p className="selectable truncate font-mono text-xs text-ink-dim">{meta.relPath}</p>
          </div>
          <div className="flex shrink-0 flex-wrap justify-end gap-2">
            <Button
              onClick={() =>
                void navigate({
                  search: (prev) => ({ ...prev, mode: mode === "edit" ? "preview" : "edit" }),
                  replace: true,
                })
              }
            >
              {mode === "edit" ? "Preview" : "Edit"}
            </Button>
            <Button onClick={() => revealDoc.mutate(meta.id)} busy={revealDoc.isPending}>
              Reveal
            </Button>
            <Button variant="primary" onClick={save} disabled={!dirty} busy={saveDoc.isPending}>
              Save
            </Button>
          </div>
        </div>
      </header>

      {(saveDoc.isError || patchFrontmatter.isError || revealDoc.isError) && (
        <div className="border-b border-edge p-4">
          {saveDoc.isError && <ErrorNote error={saveDoc.error} />}
          {patchFrontmatter.isError && <ErrorNote error={patchFrontmatter.error} />}
          {revealDoc.isError && <ErrorNote error={revealDoc.error} />}
        </div>
      )}

      <main className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_minmax(380px,440px)]">
        <section className="min-w-0 overflow-y-auto p-5">
          {mode === "edit" ? (
            <CodeMirror
              value={content}
              height="calc(100vh - 140px)"
              theme="dark"
              extensions={markdownExtensions}
              basicSetup={{
                foldGutter: true,
                lineNumbers: true,
                highlightActiveLine: true,
              }}
              onChange={(value) => {
                setContent(value);
                setDirty(value !== doc.data?.content);
              }}
            />
          ) : (
            <div className="mx-auto max-w-3xl">
              <MarkdownPreview body={body} />
            </div>
          )}
        </section>

        <aside className="min-w-0 space-y-4 overflow-y-auto border-l border-edge bg-panel p-4">
          <FrontmatterCard meta={meta} />
          <FrontmatterForm
            key={meta.id + String(mtimeMs)}
            meta={meta}
            disabled={dirty}
            busy={patchFrontmatter.isPending}
            onApply={(patch) =>
              patchFrontmatter.mutate(
                { patch, baseMtimeMs: mtimeMs },
                {
                  onSuccess: () => {
                    void doc.refetch();
                  },
                },
              )
            }
          />
          {dirty && (
            <p className="mt-2 text-xs text-warn">
              Save or discard editor changes before patching frontmatter fields.
            </p>
          )}
        </aside>
      </main>
    </div>
  );
}
