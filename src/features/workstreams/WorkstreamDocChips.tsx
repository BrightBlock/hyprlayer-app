import { DocIcon, ModeIcon, XIcon } from "@/components/icons";
import type { AttachedDoc } from "@/features/session/workstreamTabs";

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
  docs: AttachedDoc[];
  onOpenDoc: (id: string) => void;
  onDetach: (id: string) => void;
  className?: string;
};

export function WorkstreamDocChips({ docs, onOpenDoc, onDetach, className }: Props) {
  if (docs.length === 0) return null;
  return (
    <div className={`flex flex-wrap items-center gap-1.5 ${className ?? ""}`}>
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
    </div>
  );
}
