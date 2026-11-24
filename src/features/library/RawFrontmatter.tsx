import { Button } from "@/components/ui";
import type { DocContent } from "@/lib/bindings";

type Props = {
  doc: DocContent;
  raw: string | null;
  setRaw: (text: string | null) => void;
  draftNonEmpty: boolean;
  disabled: boolean;
};

// It is the only in-app path for unknown keys, comments, and repair.
export function RawFrontmatter({ doc, raw, setRaw, draftNonEmpty, disabled }: Props) {
  if (raw === null) {
    if (doc.fmError) {
      return (
        <button
          type="button"
          onClick={() => setRaw(doc.frontmatter ?? "")}
          disabled={disabled}
          title={doc.fmError}
          className="mt-2 flex items-center gap-1.5 text-ui text-warn-ink hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
        >
          <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" aria-hidden="true" />
          Frontmatter couldn't be parsed · Edit raw
        </button>
      );
    }
    return (
      <button
        type="button"
        onClick={() => setRaw(doc.frontmatter ?? "")}
        disabled={disabled || draftNonEmpty}
        title={draftNonEmpty ? "Save or clear property edits first" : undefined}
        className="mt-2 text-ui text-ink-dim hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
      >
        Edit raw
      </button>
    );
  }

  return (
    <div className="mt-2 space-y-1.5">
      <label htmlFor="fm-raw" className="sr-only">
        Raw frontmatter
      </label>
      <textarea
        id="fm-raw"
        rows={12}
        value={raw}
        disabled={disabled}
        onChange={(e) => setRaw(e.target.value)}
        className="w-full rounded-control border border-edge bg-panel-2 p-2 font-mono text-meta text-ink disabled:opacity-60"
      />
      <Button size="sm" disabled={disabled} onClick={() => setRaw(null)}>
        Cancel raw edit
      </Button>
    </div>
  );
}
