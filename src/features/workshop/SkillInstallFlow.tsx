import { useEffect, useState } from "react";

import { Icon } from "@/components/Icon";
import { Button, Callout, Card, CardTitle, ErrorNote, SegmentedControl, StepList, StepRow, Toggle, cx } from "@/components/ui";
import type {
  SkillChanges,
  SkillConflict,
  SkillInstallPreview,
  SkillInstallReport,
  SkillInstallStepKind,
  SkillScope,
  SkillTarget,
} from "@/lib/bindings";
import { formatBytes, formatElapsed, plural } from "@/lib/format";
import { useInstallSkill } from "@/lib/queries";
import { useToast } from "@/lib/toast";

import { SKILL_TARGET_LABEL, SKILL_TARGETS, sourceLine } from "./skills";

const STEP_LABEL: Record<Exclude<SkillInstallStepKind, "link">, string> = {
  verify: "Verified the files",
  store: "Copied into the store",
  lock: "Recorded in skills.lock",
};

const SCOPES: readonly SkillScope[] = ["project", "global"];

function stepTime(ms: number | null): string | null {
  if (ms == null) return null;
  return ms < 1000 ? `${Math.round(ms)} ms` : formatElapsed(ms);
}

function conflictLine(name: string, c: SkillConflict): string {
  if (c.reason === "reserved") return `/${name} is a stage command.`;
  if (c.reason === "installed") {
    return `A skill named ${name} is already installed ${c.scope === "global" ? "globally" : "in this project"}.`;
  }
  return `A skill named ${name} already exists there.`;
}

function trustLine(preview: SkillInstallPreview): string | null {
  const { flags, files } = preview;
  const scripts = files.filter((f) => f.script).length;
  const executables = files.filter((f) => f.executable).length;
  const has: string[] = [];
  if (flags.scripts) has.push(plural(scripts, "script"));
  if (flags.executables) has.push(executables === 1 ? "an executable" : plural(executables, "executable"));
  const parts: string[] = [];
  if (has.length) parts.push(`Includes ${has.join(" and ")}.`);
  if (flags.bashAllowed) parts.push(has.length ? "Agents can run them with Bash." : "Pre-approves Bash.");
  if (has.length) parts.push("Install at your own risk: Hyprlayer doesn't vet skill code.");
  return parts.length ? parts.join(" ") : null;
}

function ChangeSummary({ changes }: { changes: SkillChanges }) {
  const groups = [
    { label: "changed", files: changes.changed },
    { label: "added", files: changes.added },
    { label: "removed", files: changes.removed },
  ];
  return (
    <div className="space-y-1">
      <div className="text-ui font-medium text-ink-dim">
        {groups.map((g) => `${g.files.length} ${g.label}`).join(" · ")}
      </div>
      {groups.map((g) =>
        g.files.map((f) => (
          <div key={`${g.label}:${f}`} className="truncate font-mono text-meta text-ink">
            {f}
          </div>
        )),
      )}
    </div>
  );
}

// `lockedScope` is an update or restore approval: the scope and links come from the lock.
export function SkillInstallFlow({
  preview,
  projectPath,
  projectName,
  lockedScope,
  restoring,
  report,
  onInstalled,
  onBusy,
  onPreviewAgain,
}: {
  preview: SkillInstallPreview;
  projectPath: string | null;
  projectName: string | null;
  lockedScope?: SkillScope;
  restoring?: boolean;
  report: SkillInstallReport | null;
  onInstalled: (report: SkillInstallReport) => void;
  onBusy: (busy: boolean) => void;
  onPreviewAgain: () => void;
}) {
  const install = useInstallSkill();
  const [picked, setPicked] = useState<SkillScope | null>(null);
  const [targets, setTargets] = useState<ReadonlySet<SkillTarget>>(new Set(SKILL_TARGETS));

  useEffect(() => {
    onBusy(install.isPending);
  }, [install.isPending, onBusy]);

  const conflictOf = (scope: SkillScope) => preview.conflicts.find((c) => c.scope === scope);
  const enabled = (scope: SkillScope) => !(scope === "project" && projectPath == null) && !conflictOf(scope);
  const scope = lockedScope ?? (picked && enabled(picked) ? picked : (SCOPES.find(enabled) ?? null));
  const note = (scope && conflictOf(scope)) ?? preview.conflicts[0];
  const trust = trustLine(preview);
  const replaceVerb = restoring ? "Restore" : "Update";

  const header = (
    <div className="min-w-0">
      <h3 className="truncate text-h3 text-ink">{preview.name}</h3>
      <div className="truncate text-meta text-ink-dim">{sourceLine(preview)}</div>
    </div>
  );

  if (report) {
    return (
      <div className="space-y-3">
        {header}
        <StepList>
          {report.steps.map((s, i) => (
            <StepRow
              key={`${s.kind}:${s.target ?? ""}:${i}`}
              label={
                s.kind === "link" && s.target
                  ? `Linked for ${SKILL_TARGET_LABEL[s.target]}`
                  : s.kind === "link"
                    ? "Linked"
                    : STEP_LABEL[s.kind]
              }
              status={s.status}
              detail={[s.detail, stepTime(s.elapsedMs)].filter(Boolean).join(" · ") || null}
            />
          ))}
        </StepList>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {header}
      <Card className="space-y-3">
        <CardTitle className="mb-0">{lockedScope ? `${replaceVerb} skill` : "Add to your skills"}</CardTitle>
        {!lockedScope && (
          <>
            <div className="space-y-1.5">
              <div className="text-ui font-medium text-ink-dim">Install to</div>
              <SegmentedControl
                aria-label="Install to"
                value={scope}
                onChange={setPicked}
                options={[
                  { value: "project", label: "This project", disabled: !enabled("project") },
                  { value: "global", label: "Global", disabled: !enabled("global") },
                ]}
              />
              {note ? (
                <div className="text-meta text-warn-ink">{conflictLine(preview.name, note)}</div>
              ) : (
                scope === "project" && (
                  <div className="text-meta text-ink-dim">{`Lands in ${projectName ?? "the project"} · commit it to share`}</div>
                )
              )}
            </div>
            <div className="space-y-1">
              <div className="text-ui font-medium text-ink-dim">For these agents</div>
              {SKILL_TARGETS.map((t) => (
                <Toggle
                  key={t}
                  className="w-full justify-between"
                  checked={targets.has(t)}
                  onChange={(on) =>
                    setTargets((prev) => {
                      const next = new Set(prev);
                      if (on) next.add(t);
                      else next.delete(t);
                      return next;
                    })
                  }
                >
                  <span className="text-body text-ink">{SKILL_TARGET_LABEL[t]}</span>
                </Toggle>
              ))}
            </div>
          </>
        )}
        {trust && (
          <Callout tone="warn" icon={<Icon name="warning" size="sm" />}>
            {trust}
          </Callout>
        )}
        {install.isError && <ErrorNote error={install.error} />}
        <div className="flex justify-end">
          {install.isError ? (
            <Button type="button" onClick={onPreviewAgain}>
              Preview again
            </Button>
          ) : (
            <Button
              type="button"
              variant="primary"
              busy={install.isPending}
              disabled={scope == null || (!lockedScope && targets.size === 0)}
              onClick={() => {
                if (scope == null) return;
                install.mutate(
                  {
                    projectPath,
                    token: preview.token,
                    scope,
                    targets: lockedScope ? [] : SKILL_TARGETS.filter((t) => targets.has(t)),
                  },
                  {
                    onSuccess: (r) => {
                      useToast.getState().show(
                        lockedScope ? (restoring ? "Skill restored" : "Skill updated") : "Skill added",
                        preview.name,
                      );
                      onInstalled(r);
                    },
                  },
                );
              }}
            >
              {lockedScope ? replaceVerb : "Add to skills"}
            </Button>
          )}
        </div>
      </Card>
      {preview.changes && <ChangeSummary changes={preview.changes} />}
      <div className="space-y-1.5">
        <div className="text-ui font-medium text-ink-dim">{`Files · ${preview.files.length}`}</div>
        <div className="max-h-56 overflow-y-auto rounded-md">
          <StepList>
            {preview.files.map((f) => {
              const flagged = f.script || f.executable;
              return (
                <div key={f.relPath} className="flex h-8 items-center gap-2.5 px-3">
                  <span className="flex h-4 w-4 shrink-0 items-center justify-center">
                    {flagged && <Icon name="warning" size="sm" className="text-warn-ink" />}
                  </span>
                  <span className={cx("min-w-0 flex-1 truncate font-mono text-meta", flagged ? "text-warn-ink" : "text-ink")}>
                    {f.relPath}
                  </span>
                  <span className="shrink-0 text-meta text-ink-dim">{formatBytes(f.bytes ?? 0)}</span>
                </div>
              );
            })}
          </StepList>
        </div>
      </div>
    </div>
  );
}
