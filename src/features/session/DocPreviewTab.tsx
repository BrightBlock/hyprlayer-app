import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { commands, type DocContent } from "@/lib/bindings";
import { isAppError, unwrap } from "@/lib/ipc";
import { useProjectMemory, useSaveProjectMemoryFile } from "@/lib/queries";
import { Button, ErrorNote, IconButton, Spinner } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { FrontmatterCard } from "@/features/library/FrontmatterCard";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import { splitFrontmatter } from "@/features/library/text";
import { codeBase } from "@/lib/codeView";

const editorExtensions = [...codeBase, EditorView.lineWrapping, markdown()];

type Draft = { seed: string; content: string; baseMtimeMs: number };
const drafts = new Map<string, Draft>();

function draftOf(doc: DocContent): Draft {
  return { seed: doc.content, content: doc.content, baseMtimeMs: doc.mtimeMs ?? 0 };
}

export function DocPreviewTab({
  workstreamId,
  path,
  title,
}: {
  workstreamId: string;
  path: string;
  title: string;
}) {
  const doc = useQuery({
    queryKey: ["doc", path],
    queryFn: () => unwrap(commands.getDoc(path)),
    enabled: path.length > 0,
    retry: 4,
    retryDelay: (attempt) => Math.min(1500, 300 * 2 ** attempt),
  });
  const memory = useProjectMemory(workstreamId);
  const memoryFile = memory.data?.files.find((f) => f.path === path) ?? null;
  const save = useSaveProjectMemoryFile(workstreamId, path);

  const draftKey = `${workstreamId}:${path}`;
  const [draft, setDraft] = useState<Draft | null>(() => drafts.get(draftKey) ?? null);
  useEffect(() => {
    if (draft) drafts.set(draftKey, draft);
    else drafts.delete(draftKey);
  }, [draftKey, draft]);

  function startEdit() {
    if (doc.data) setDraft(draftOf(doc.data));
  }
  function cancel() {
    setDraft(null);
    save.reset();
  }
  function commit() {
    if (!draft || !memoryFile) return;
    save.mutate(
      { name: memoryFile.name, content: draft.content, baseMtimeMs: draft.baseMtimeMs },
      { onSuccess: () => setDraft(null) },
    );
  }
  function reload() {
    void doc.refetch().then(({ data }) => {
      save.reset();
      setDraft(data ? draftOf(data) : null);
    });
  }

  const conflict = isAppError(save.error) && save.error.code === "doc_conflict";

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-5">
      <div className="mx-auto w-[90%]">
        <div className="mb-4 flex items-center gap-2 border-b border-edge pb-3">
          <Icon name="doc" />
          <span className="truncate text-body font-medium text-ink" title={path}>
            {doc.data?.meta.relPath ?? title}
          </span>
          <div className="ml-auto flex shrink-0 items-center gap-2">
            {draft ? (
              <>
                <Button onClick={cancel}>Cancel</Button>
                <Button
                  variant="primary"
                  busy={save.isPending}
                  disabled={!memoryFile || draft.content === draft.seed}
                  onClick={commit}
                >
                  Save
                </Button>
              </>
            ) : (
              memoryFile &&
              doc.data && (
                <IconButton aria-label="Edit memory file" title="Edit memory file" onClick={startEdit}>
                  <Icon name="edit" size="sm" />
                </IconButton>
              )
            )}
          </div>
        </div>
        {draft ? (
          <div className="space-y-3">
            <ErrorNote error={save.error} />
            {conflict && <Button onClick={reload}>Reload from disk</Button>}
            <CodeMirror
              value={draft.content}
              minHeight="60vh"
              theme="none"
              extensions={editorExtensions}
              basicSetup={{
                foldGutter: true,
                lineNumbers: false,
                syntaxHighlighting: false,
                highlightActiveLine: true,
              }}
              onChange={(content) => setDraft((d) => (d ? { ...d, content } : d))}
            />
          </div>
        ) : doc.isLoading ? (
          <div className="flex items-center gap-2 text-body text-ink-dim">
            <Spinner size="sm" />
            Loading document…
          </div>
        ) : doc.isError ? (
          <ErrorNote error={doc.error} />
        ) : (
          <div className="space-y-4">
            <FrontmatterCard meta={doc.data!.meta} />
            <MarkdownPreview body={splitFrontmatter(doc.data!.content).body} />
          </div>
        )}
      </div>
    </div>
  );
}
