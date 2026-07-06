import { DocIcon, ModeIcon, XIcon } from "@/components/icons";
import type { AttachedDoc } from "@/features/session/workstreamTabs";

/** "2026-06-12-AUTH-1-auth-research.md" -> "AUTH-1-auth-research" */
export function chipLabel(docPath: string): string {
  const base = docPath.split("/").pop() ?? docPath;
  const stem = base.replace(/\.md$/i, "");
  return stem.replace(/^\d{4}-\d{2}-\d{2}-?/, "") || stem;
}

function RoleIcon({ role }: { role: string }) {
  if (role === "research" || role === "plan") {
    return <ModeIcon mode={role} size={12} strokeWidth={2} />;
  }
  return <DocIcon />;
}

type Props = {
  /** Context docs of the ACTIVE agent tab — each tab carries its own. */
  docs: AttachedDoc[];
  /** Open a doc (absolute-path id) in the doc viewer. */
  onOpenDoc: (id: string) => void;
  /** Remove a doc from this tab — detaches a manual doc, dismisses a suggested one. */
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
      {docs.map((doc) => {
        const verb = doc.suggested ? "Dismiss" : "Detach";
        return (
          <span
            key={doc.path}
            className={`inline-flex max-w-56 items-center gap-1 rounded-md border bg-panel-2 pl-1.5 text-xs text-ink-dim ${
              doc.suggested ? "border-dashed border-edge/70" : "border-edge"
            }`}
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
            {doc.suggested && (
              <span className="shrink-0 rounded px-1 py-0.5 text-[10px] leading-none text-ink-dim/60">
                Suggested
              </span>
            )}
            <button
              type="button"
              onClick={() => onDetach(doc.path)}
              className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded hover:bg-edge hover:text-ink"
              title={doc.suggested ? "Dismiss suggestion" : "Detach from this tab"}
              aria-label={`${verb} ${chipLabel(doc.path)}`}
            >
              <XIcon size={10} strokeWidth={2.4} />
            </button>
          </span>
        );
      })}
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
