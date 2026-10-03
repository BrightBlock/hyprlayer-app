import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";
import type { ReviewFile } from "@/lib/bindings";
import { basename } from "@/lib/path";
import type { CheckGroup, PlanDoc, PlanDocCriterion, PlanDocPhase } from "@/features/plan/planDoc";

import { tasksForFile, type Attribution } from "./attribution";

const CHECK_LABEL: Record<CheckGroup | "none", string> = {
  automated: "Automated",
  manual: "Manual",
  none: "Checks",
};

type CriteriaGroup = { label: string; rows: PlanDocCriterion[] };

function criteriaGroups(criteria: PlanDocCriterion[]): CriteriaGroup[] {
  const groups: CriteriaGroup[] = [];
  for (const row of criteria) {
    const label = CHECK_LABEL[row.group ?? "none"];
    const last = groups[groups.length - 1];
    if (last?.label === label) last.rows.push(row);
    else groups.push({ label, rows: [row] });
  }
  return groups;
}

// Keys match the tree's task groups: a doc row by key, phase-level work by phase label.
function filesByTask(files: ReviewFile[], attribution: Attribution): Map<string, ReviewFile[]> {
  const out = new Map<string, ReviewFile[]>();
  for (const f of files) {
    for (const ref of tasksForFile(attribution, f)) {
      if (ref.phaseLabel === null) continue;
      const key = ref.taskKey ? `task:${ref.taskKey}` : `phase:${ref.phaseLabel}`;
      const list = out.get(key);
      if (!list) out.set(key, [f]);
      else if (!list.includes(f)) list.push(f);
    }
  }
  return out;
}

function FileChips({
  files,
  viewed,
  onSelect,
}: {
  files: ReviewFile[] | undefined;
  viewed: ReadonlySet<string>;
  onSelect: (path: string) => void;
}) {
  if (!files || files.length === 0) {
    return <p className="text-ui text-ink-4">No code changes for this task.</p>;
  }
  return (
    <div className="flex flex-wrap gap-1.5">
      {[...files]
        .sort((a, b) => a.path.localeCompare(b.path))
        .map((f) => {
          const seen = viewed.has(f.path);
          return (
            <button
              key={f.path}
              type="button"
              title={f.path}
              onClick={() => onSelect(f.path)}
              className={cx(
                "flex max-w-full items-center gap-1 h-6 rounded-control border border-edge px-2 text-meta hover:bg-edge/70",
                seen ? "text-ink-4" : "text-ink-2",
              )}
            >
              {seen && <Icon name="check" size="xs" className="shrink-0 text-ok" />}
              <span className="truncate">{basename(f.path)}</span>
            </button>
          );
        })}
    </div>
  );
}

function PhaseCard({
  phase,
  byTask,
  viewed,
  onSelect,
}: {
  phase: PlanDocPhase;
  byTask: Map<string, ReviewFile[]>;
  viewed: ReadonlySet<string>;
  onSelect: (path: string) => void;
}) {
  const phaseFiles = byTask.get(`phase:${phase.label}`);
  return (
    <section className="space-y-4 rounded-card border border-edge bg-panel p-4">
      <h3 className="text-ui font-semibold text-ink">
        Phase {phase.label}
        {phase.name && <span className="font-normal text-ink-dim"> · {phase.name}</span>}
      </h3>
      {(phaseFiles || phase.tasks.length === 0) && (
        <FileChips files={phaseFiles} viewed={viewed} onSelect={onSelect} />
      )}
      {phase.tasks.length > 0 && (
        <ul className="divide-y divide-edge/60">
          {phase.tasks.map((t) => (
            <li key={t.key} className="space-y-2 py-3 first:pt-0 last:pb-0">
              <div className="text-body text-ink">{t.title}</div>
              <FileChips files={byTask.get(`task:${t.key}`)} viewed={viewed} onSelect={onSelect} />
            </li>
          ))}
        </ul>
      )}
      {phase.criteria.length > 0 && (
        <div className="space-y-2 border-t border-edge pt-3">
          <div className="text-label text-ink-dim">Success criteria</div>
          {criteriaGroups(phase.criteria).map((g, i) => (
            <div key={i}>
              <div className="mb-2 text-meta text-ink-4">{g.label}</div>
              <ul className="space-y-1.5">
                {g.rows.map((row) => (
                  <li key={row.key} className="flex items-start gap-2 text-ui text-ink-2">
                    <input
                      type="checkbox"
                      checked={row.checked}
                      readOnly
                      disabled
                      aria-label={row.title}
                      className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-accent"
                    />
                    <span className={cx("min-w-0 break-words", row.checked && "text-ink-4")}>{row.title}</span>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

export function PlanCoverage({
  plan,
  files,
  attribution,
  viewed,
  onSelectFile,
}: {
  plan: PlanDoc;
  files: ReviewFile[];
  attribution: Attribution;
  viewed: ReadonlySet<string>;
  onSelectFile: (path: string) => void;
}) {
  const byTask = filesByTask(files, attribution);
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto max-w-[880px] space-y-4 px-8 pb-10 pt-6">
        <h2 className="text-h3 text-ink">Plan coverage</h2>
        {plan.phases.map((p, i) => (
          <PhaseCard key={`${p.label}:${i}`} phase={p} byTask={byTask} viewed={viewed} onSelect={onSelectFile} />
        ))}
      </div>
    </div>
  );
}
