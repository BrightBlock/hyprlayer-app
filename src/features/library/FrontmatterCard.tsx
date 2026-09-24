import { Fragment } from "react";

import type { DocMeta } from "@/lib/bindings";
import { Badge, Card } from "@/components/ui";
import { STATUS_TONE } from "./constants";

export function FrontmatterCard({ meta }: { meta: DocMeta }) {
  const fm = meta.fm;

  const rows: Array<{ label: string; value: string }> = [];
  if (fm.date) rows.push({ label: "Date", value: fm.date });
  if (fm.ticket) rows.push({ label: "Ticket", value: fm.ticket });
  const project = fm.project ?? meta.project;
  if (project) rows.push({ label: "Project", value: project });
  if (fm.author) rows.push({ label: "Author", value: fm.author });
  for (const [key, value] of Object.entries(fm.extra ?? {})) {
    rows.push({ label: key, value });
  }

  const tags = fm.tags ?? [];
  const related = fm.related ?? [];

  return (
    <Card>
      <div className="flex flex-wrap items-center gap-2">
        {fm.type && <Badge tone="accent">{fm.type}</Badge>}
        {fm.status && <Badge tone={STATUS_TONE[fm.status] ?? "dim"}>{fm.status}</Badge>}
        <Badge tone="dim">{meta.scope}</Badge>
        {meta.fmWarnings.length > 0 && (
          <span className="text-xs text-warn-ink" title={meta.fmWarnings.join("\n")}>
            ⚠ {meta.fmWarnings.length} frontmatter warning{meta.fmWarnings.length === 1 ? "" : "s"}
          </span>
        )}
      </div>
      {rows.length > 0 && (
        <dl className="selectable mt-3 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-xs">
          {rows.map((row, i) => (
            <Fragment key={`${row.label}:${i}`}>
              <dt className="text-ink-dim">{row.label}</dt>
              <dd className="min-w-0 break-all text-ink">{row.value}</dd>
            </Fragment>
          ))}
        </dl>
      )}
      {tags.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {tags.map((tag) => (
            <span key={tag} className="rounded bg-panel-2 px-1.5 py-0.5 text-[11px] text-ink-dim">
              #{tag}
            </span>
          ))}
        </div>
      )}
      {related.length > 0 && (
        <p className="selectable mt-2 text-xs text-ink-dim">Related: {related.join(", ")}</p>
      )}
    </Card>
  );
}
