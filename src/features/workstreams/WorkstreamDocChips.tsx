import type { AttachedDoc } from "../../routes/ProjectPage";

/** "2026-06-12-AUTH-1-auth-research.md" -> "AUTH-1-auth-research" */
export function chipLabel(docPath: string): string {
  const base = docPath.split("/").pop() ?? docPath;
  const stem = base.replace(/\.md$/i, "");
  return stem.replace(/^\d{4}-\d{2}-\d{2}-?/, "") || stem;
}

function RoleIcon({ role }: { role: string }) {
  if (role === "research") {
    return (
      <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none">
        <circle cx="10.5" cy="10.5" r="5.5" stroke="currentColor" strokeWidth="2" />
        <path d="m15 15 4 4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      </svg>
    );
  }
  if (role === "plan") {
    return (
      <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none">
        <path d="M7 7h11M7 12h11M7 17h11" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        <path d="m3.5 7 .8.8L6 6M3.5 12l.8.8L6 11" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }
  return (
    <svg aria-hidden="true" width="12" height="12" viewBox="0 0 24 24" fill="none">
      <path d="M6 3h8l4 4v14H6V3Z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
      <path d="M14 3v4h4" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
    </svg>
  );
}

type Props = {
  /** Context docs of the ACTIVE agent tab — each tab carries its own. */
  docs: AttachedDoc[];
  /** Open a doc (absolute-path id) in the doc viewer. */
  onOpenDoc: (id: string) => void;
  /** Detach a doc from this tab's context. */
  onDetach: (id: string) => void;
  /** Jump to the Docs view to attach more context. */
  onBrowse: () => void;
};

/**
 * Context documents attached to one agent tab — research/plan docs the
 * agent should carry through its spec phases. Rendered above the composer.
 */
export function WorkstreamDocChips({ docs, onOpenDoc, onDetach, onBrowse }: Props) {
  return (
    <div className="mb-2 flex flex-wrap items-center gap-1.5">
      {docs.map((doc) => (
        <span
          key={doc.path}
          className="inline-flex max-w-56 items-center gap-1 rounded-md border border-edge bg-panel-2 pl-1.5 text-xs text-ink-dim"
        >
          <span className="shrink-0 text-accent">
            <RoleIcon role={doc.role} />
          </span>
          <button
            type="button"
            onClick={() => onOpenDoc(doc.path)}
            className="min-w-0 truncate py-1 hover:text-ink"
            title={doc.path}
          >
            {chipLabel(doc.path)}
          </button>
          <button
            type="button"
            onClick={() => onDetach(doc.path)}
            className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded hover:bg-edge hover:text-ink"
            title="Detach from this tab"
            aria-label={`Detach ${chipLabel(doc.path)}`}
          >
            <svg aria-hidden="true" width="10" height="10" viewBox="0 0 24 24" fill="none">
              <path d="m7 7 10 10M17 7 7 17" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
            </svg>
          </button>
        </span>
      ))}
      <button
        type="button"
        onClick={onBrowse}
        className="inline-flex items-center gap-1 rounded-md border border-dashed border-edge px-1.5 py-1 text-xs text-ink-dim hover:border-accent/50 hover:text-ink"
        title="Attach docs as context for this tab"
      >
        <svg aria-hidden="true" width="11" height="11" viewBox="0 0 24 24" fill="none">
          <path
            d="m20 11-8.4 8.4a5 5 0 0 1-7-7L13 4a3.3 3.3 0 0 1 4.7 4.7L9.4 17a1.7 1.7 0 0 1-2.4-2.4L14.5 7"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
          />
        </svg>
        {docs.length === 0 ? "Attach context" : "Attach"}
      </button>
    </div>
  );
}
