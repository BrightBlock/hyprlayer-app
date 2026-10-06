import { useEffect, useMemo, useState } from "react";

import { Badge, Button, Dialog, EmptyState, ErrorNote, PageSpinner, StepList, StepRow, TextInput } from "@/components/ui";
import {
  ListCardRow,
  ListHeader,
  NavFilterRow,
  NavGroupLabel,
  WorkshopFrame,
  WorkshopNav,
  type WorkshopChrome,
} from "@/components/workshop";
import type { SkillFile, SkillInstallPreview, SkillRestoreEntry, SkillRestoreReport, SkillScope, SkillUpdate } from "@/lib/bindings";
import { plural } from "@/lib/format";
import {
  useAgentFiles,
  useInstallSkill,
  usePreviewSkillUpdate,
  useRestoreSkills,
  useSkillFiles,
  useSkillLocks,
  useSkillUpdates,
} from "@/lib/queries";

import { SkillSourceBrowse } from "./SkillSourceBrowse";
import { AddSkillDialog, type SkillAdded } from "./AddSkillDialog";
import { SkillDetail } from "./SkillDetail";
import { SkillReviewDialog } from "./SkillReviewDialog";
import {
  attachedAgents,
  filterSkills,
  SKILL_FILTERS,
  SKILL_TARGET_BADGE,
  skillFilter,
  skillFolder,
  skillMeta,
  skillOriginLine,
  skillScope,
  skillSourceLabel,
  type SkillFilter,
  type SkillTab,
} from "./skills";

const SKILLS_SH_VIEW = "skills.sh";

type Review = { preview: SkillInstallPreview; scope: SkillScope; rest: SkillUpdate[]; restoreKey?: string };

const updateKey = (scope: SkillScope, name: string) => `${scope}:${name}`;

const RESTORE_LABEL: Record<SkillRestoreEntry["outcome"], string> = {
  ok: "in place",
  relinked: "relinked",
  refetched: "refetched",
  needsApproval: "needs approval",
  skipped: "skipped",
  failed: "failed",
};

function RestoreRows({
  report,
  approved,
  onReview,
}: {
  report: SkillRestoreReport;
  approved: ReadonlySet<string>;
  onReview: (entry: SkillRestoreEntry, preview: SkillInstallPreview) => void;
}) {
  return (
    <div className="space-y-1.5">
      <div className="text-ui font-medium text-ink-dim">{report.scope === "project" ? "This project" : "Global"}</div>
      <StepList>
        {report.entries.map((e) => {
          const pending = e.outcome === "needsApproval" && e.preview && !approved.has(updateKey(report.scope, e.name));
          if (pending && e.preview) {
            const preview = e.preview;
            return (
              <div key={e.name} className="flex h-9 items-center gap-2.5 px-3">
                <span className="min-w-0 flex-1 truncate text-body text-ink">{e.name}</span>
                <Button size="sm" onClick={() => onReview(e, preview)}>
                  Review
                </Button>
              </div>
            );
          }
          const status = e.outcome === "failed" ? "failed" : e.outcome === "skipped" ? "skipped" : "done";
          return (
            <StepRow
              key={e.name}
              label={e.detail ? `${e.name}: ${e.detail}` : e.name}
              status={status}
              detail={e.outcome === "needsApproval" ? "approved" : RESTORE_LABEL[e.outcome]}
            />
          );
        })}
      </StepList>
    </div>
  );
}

export function SkillLibrary({
  chrome,
  projectPath,
  projectName,
  selectedPath,
  onSelect,
  filter: rawFilter,
  onFilter,
  view,
  onView,
  onOpenAgent,
  tab,
  onTab,
}: {
  chrome: WorkshopChrome;
  projectPath: string | null;
  projectName: string | null;
  selectedPath: string | null;
  onSelect: (path: string | null) => void;
  filter: string | undefined;
  onFilter: (filter: SkillFilter) => void;
  view: string | undefined;
  onView: (view: string | null) => void;
  onOpenAgent: (path: string, target: "agents" | "system") => void;
  tab: SkillTab;
  onTab: (tab: SkillTab) => void;
}) {
  const skills = useSkillFiles(projectPath);
  const agents = useAgentFiles(projectPath);
  const [search, setSearch] = useState("");
  const [adding, setAdding] = useState(false);
  const [added, setAdded] = useState<SkillAdded | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  const [reports, setReports] = useState<SkillRestoreReport[] | null>(null);
  const [approved, setApproved] = useState<ReadonlySet<string>>(new Set());
  const [updating, setUpdating] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
  const locks = useSkillLocks(projectPath);
  const updates = useSkillUpdates(projectPath);
  const previewUpdate = usePreviewSkillUpdate();
  const installSkill = useInstallSkill();
  const restore = useRestoreSkills();
  const filter = skillFilter(rawFilter);

  const all = useMemo(() => skills.data ?? [], [skills.data]);
  const inFilter = useMemo(() => filterSkills(all, filter), [all, filter]);
  const visible = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return inFilter;
    return inFilter.filter(
      (s) => s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
    );
  }, [inFilter, search]);

  const selected = selectedPath ? (all.find((s) => s.path === selectedPath) ?? null) : null;
  const filterLabel = SKILL_FILTERS.find((f) => f.key === filter)?.label ?? "All";

  const outdated = useMemo(() => (updates.data ?? []).filter((u) => u.updateAvailable), [updates.data]);
  const outdatedKeys = useMemo(() => new Set(outdated.map((u) => updateKey(u.scope, u.name))), [outdated]);
  const hasInstalled = all.some((s) => s.installed);
  const needsRestore = (locks.data ?? []).some((l) => l.state !== "ok");
  const hasUpdate = (s: SkillFile) => s.installed && outdatedKeys.has(updateKey(skillScope(s), skillFolder(s)));

  // An update whose content didn't change applies silently; any change stops for approval.
  const runUpdates = async (queue: SkillUpdate[]) => {
    setUpdating(true);
    setActionError(null);
    try {
      for (const [i, u] of queue.entries()) {
        const preview = await previewUpdate.mutateAsync({ projectPath, scope: u.scope, name: u.name });
        const c = preview.changes;
        if (c && !c.added.length && !c.removed.length && !c.changed.length) {
          await installSkill.mutateAsync({ projectPath, token: preview.token, scope: u.scope, targets: [] });
          continue;
        }
        setReview({ preview, scope: u.scope, rest: queue.slice(i + 1) });
        return;
      }
    } catch (e) {
      setActionError(e);
    } finally {
      setUpdating(false);
    }
  };

  const runRestore = async () => {
    setActionError(null);
    try {
      const scopes = [...new Set((locks.data ?? []).filter((l) => l.state !== "ok").map((l) => l.scope))];
      const out: SkillRestoreReport[] = [];
      for (const scope of scopes) out.push(await restore.mutateAsync({ projectPath, scope }));
      setApproved(new Set());
      setReports(out);
    } catch (e) {
      setActionError(e);
    }
  };

  const finishReview = () => {
    if (!review) return;
    const { restoreKey, rest } = review;
    setReview(null);
    if (restoreKey) setApproved((prev) => new Set(prev).add(restoreKey));
    if (rest.length) void runUpdates(rest);
  };

  // The list refetches after an install, so select once the new row arrives.
  useEffect(() => {
    if (!added) return;
    const row = all.find((s) => skillFolder(s) === added.folder && skillScope(s) === added.scope);
    if (!row) return;
    onSelect(row.path);
    setAdded(null);
  }, [added, all, onSelect]);

  if (skills.isLoading) return <PageSpinner />;

  const browsing = view === SKILLS_SH_VIEW;
  const nav = (
    <WorkshopNav
      chrome={chrome}
      action={
        <Button variant="primary" className="w-full justify-center" onClick={() => setAdding(true)}>
          Add skill
        </Button>
      }
      filters={
        <div className="space-y-4">
          <div>
            <NavGroupLabel>Source</NavGroupLabel>
            <div className="space-y-0.5">
              {SKILL_FILTERS.map((f) => (
                <NavFilterRow
                  key={f.key}
                  label={f.label}
                  count={filterSkills(all, f.key).length}
                  selected={!browsing && filter === f.key}
                  onClick={() => onFilter(f.key)}
                />
              ))}
            </div>
          </div>
          <div>
            <NavGroupLabel>Browse</NavGroupLabel>
            <div className="space-y-0.5">
              <NavFilterRow label="skills.sh" selected={browsing} onClick={() => onView(SKILLS_SH_VIEW)} />
            </div>
          </div>
        </div>
      }
    />
  );
  const dialog = (
    <AddSkillDialog
      open={adding}
      onClose={() => setAdding(false)}
      onDone={setAdded}
      projectPath={projectPath}
      projectName={projectName}
    />
  );

  if (browsing) {
    return (
      <>
        <SkillSourceBrowse nav={nav} skills={all} projectPath={projectPath} projectName={projectName} />
        {dialog}
      </>
    );
  }

  return (
    <>
      <WorkshopFrame
        navKey={`skills:${filter}`}
        nav={nav}
        list={
          <>
            <ListHeader
              actions={
                <>
                  {hasInstalled && (
                    <Button size="sm" busy={updates.isFetching} onClick={() => void updates.refetch()}>
                      Check for updates
                    </Button>
                  )}
                  {hasInstalled && outdated.length > 0 && (
                    <Button size="sm" busy={updating} onClick={() => void runUpdates(outdated)}>
                      Update all
                    </Button>
                  )}
                  {needsRestore && (
                    <Button size="sm" busy={restore.isPending} onClick={() => void runRestore()}>
                      Restore
                    </Button>
                  )}
                </>
              }
            >
              {`${filterLabel} · ${plural(visible.length, "skill")}`}
            </ListHeader>
            <div className="shrink-0 border-b border-edge p-3">
              <TextInput
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Filter…"
                aria-label="Filter skills"
              />
            </div>
            <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-3 pt-2">
              {skills.isError && <ErrorNote error={skills.error} />}
              {updates.isError && <ErrorNote error={updates.error} />}
              {actionError != null && <ErrorNote error={actionError} onDismiss={() => setActionError(null)} />}
              {visible.map((s) => (
                <ListCardRow
                  key={s.path}
                  title={s.name}
                  badges={
                    <>
                      <Badge tone="dim">{skillSourceLabel(s)}</Badge>
                      {s.userOnly && <Badge tone="dim">User-only</Badge>}
                      {s.installed && s.targets.map((t) => <Badge key={t} tone="dim">{SKILL_TARGET_BADGE[t]}</Badge>)}
                      {hasUpdate(s) && <Badge tone="accent">Update</Badge>}
                    </>
                  }
                  description={s.description}
                  meta={skillOriginLine(s) ?? skillMeta(s)}
                  selected={s.path === selectedPath}
                  onClick={() => onSelect(s.path)}
                />
              ))}
              {visible.length === 0 && !skills.isError && !skills.isFetching && (
                <div className="mx-0.5 rounded-md border border-dashed border-edge px-2.5 py-2.5 text-ui leading-prose text-ink-dim">
                  {search.trim() ? `No skills match “${search.trim()}”` : "No skills"}
                </div>
              )}
            </div>
          </>
        }
        detail={
          selected ? (
            <SkillDetail
              skill={selected}
              projectPath={projectPath}
              projectName={projectName}
              updateAvailable={hasUpdate(selected)}
              attached={attachedAgents(selected, agents.data ?? [])}
              onOpenAgent={onOpenAgent}
              tab={tab}
              onTab={onTab}
              onSelect={onSelect}
            />
          ) : (
            <EmptyState title="Select a skill" />
          )
        }
      />
      {dialog}
      <Dialog open={reports != null} onClose={() => setReports(null)} size="lg" title="Restore skills" ariaLabel="Restore skills">
        <div className="space-y-3">
          {reports?.map((r) => (
            <RestoreRows
              key={r.scope}
              report={r}
              approved={approved}
              onReview={(e, preview) =>
                setReview({ preview, scope: r.scope, rest: [], restoreKey: updateKey(r.scope, e.name) })
              }
            />
          ))}
          <div className="flex justify-end">
            <Button type="button" variant="primary" onClick={() => setReports(null)}>
              Done
            </Button>
          </div>
        </div>
      </Dialog>
      <SkillReviewDialog
        preview={review?.preview ?? null}
        scope={review?.scope ?? "global"}
        restoring={review?.restoreKey != null}
        projectPath={projectPath}
        projectName={projectName}
        onDone={finishReview}
        onCancel={() => setReview(null)}
      />
    </>
  );
}
