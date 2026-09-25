import { useEffect, useRef, useState } from "react";

import {
  useBackendHealth,
  useBranchPrefixes,
  useProjectDetail,
  useSetBranchPrefixes,
  useSetWorktreeFiles,
  useWorktreeFiles,
} from "@/lib/queries";
import {
  Badge,
  Button,
  Dialog,
  ErrorNote,
  IconButton,
  PageError,
  PageSpinner,
  Spinner,
  TextInput,
} from "@/components/ui";
import { BranchPrefixEditor } from "@/components/BranchPrefixEditor";
import { PlusIcon } from "@/components/icons";
import type { CheckStatus, HealthCheck } from "@/lib/bindings";
import { isGitProject } from "@/lib/domain/projects";
import { ScriptsCard } from "./ScriptsCard";
import { SectionHeading, SettingsSection } from "./settingsSection";

const CHECK_TONE: Record<CheckStatus, "ok" | "warn" | "bad"> = {
  pass: "ok",
  warn: "warn",
  fail: "bad",
};

function HealthRow({ check }: { check: HealthCheck }) {
  return (
    <div className="flex items-start gap-3 py-1.5">
      <Badge tone={CHECK_TONE[check.status]}>{check.status}</Badge>
      <div className="min-w-0">
        <p className="text-body">{check.label}</p>
        {check.detail && (
          <p className="selectable truncate text-meta text-ink-dim" title={check.detail}>
            {check.detail}
          </p>
        )}
        {check.remedy && check.status !== "pass" && (
          <p className="mt-0.5 text-ui text-warn-ink">{check.remedy}</p>
        )}
      </div>
    </div>
  );
}

function WorktreeFilesCard({ projectPath }: { projectPath: string }) {
  const [newPattern, setNewPattern] = useState("");
  const files = useWorktreeFiles(projectPath);
  const setFiles = useSetWorktreeFiles(projectPath);
  const listRef = useRef<HTMLUListElement>(null);
  const prevLen = useRef(0);

  const list: string[] = files.data ?? [];

  useEffect(() => {
    if (list.length === prevLen.current + 1) {
      listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
    }
    prevLen.current = list.length;
  }, [list.length]);

  function add() {
    const p = newPattern.trim();
    if (!p || list.includes(p)) return;
    setFiles.mutate([...list, p]);
    setNewPattern("");
  }

  function remove(p: string) {
    setFiles.mutate(list.filter((x) => x !== p));
  }

  return (
    <SettingsSection>
      <SectionHeading>Files copied into new worktrees</SectionHeading>
      <p className="mb-3 mt-1.5 max-w-xl text-ui leading-prose text-ink-dim">
        Copied into every new worktree. Glob patterns supported.
      </p>
      {files.isLoading && <Spinner />}
      {files.isError && <ErrorNote error={files.error} />}
      {setFiles.isError && <ErrorNote error={setFiles.error} onDismiss={() => setFiles.reset()} />}
      {list.length > 0 && (
        <ul ref={listRef} className="mb-3 max-h-40 space-y-1 overflow-y-auto pr-1">
          {list.map((p) => (
            <li key={p} className="flex items-center gap-2">
              <span className="selectable flex-1 text-meta text-ink">{p}</span>
              <button
                type="button"
                className="shrink-0 rounded px-1 py-0.5 text-ui text-ink-dim hover:bg-edge/60 hover:text-bad"
                onClick={() => remove(p)}
                disabled={setFiles.isPending}
                aria-label={`Remove ${p}`}
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-2">
        <TextInput
          value={newPattern}
          onChange={(e) => setNewPattern(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") add();
          }}
          placeholder=".env, certs/, config/*.local.json"
          disabled={setFiles.isPending}
        />
        <IconButton
          onClick={add}
          disabled={!newPattern.trim() || setFiles.isPending}
          aria-label="Add file pattern"
          title="Add file pattern"
        >
          <PlusIcon size={15} />
        </IconButton>
      </div>
    </SettingsSection>
  );
}

function BranchPrefixesCard({ projectPath }: { projectPath: string }) {
  const q = useBranchPrefixes(projectPath);
  const save = useSetBranchPrefixes(projectPath);
  return (
    <SettingsSection>
      {q.isLoading && <Spinner />}
      {q.isError && <ErrorNote error={q.error} />}
      {save.isError && <ErrorNote error={save.error} onDismiss={() => save.reset()} />}
      <BranchPrefixEditor
        title={<SectionHeading>Branch types</SectionHeading>}
        description="Customize the branch types for what you're working on in this repo."
        value={q.data?.repo ?? []}
        onChange={(next) => save.mutate(next)}
        disabled={save.isPending}
        blockReseed={save.isPending || save.isError}
        inheritedNote={q.data?.source === "global" ? "Your global types" : "Built-in types"}
      />
    </SettingsSection>
  );
}

type Props = {
  projectPath: string;
  onClose: () => void;
};

export function ProjectSettingsDialog({ projectPath, onClose }: Props) {
  const detail = useProjectDetail(projectPath);
  const health = useBackendHealth(projectPath);

  const heading = detail.data
    ? `Project settings: ${detail.data.project.mappedName}`
    : "Project settings";
  const gitBacked = isGitProject(detail.data?.project);

  return (
    <Dialog
      open
      onClose={onClose}
      seams
      size="3xl"
      title={
        <>
          {heading}
          {detail.data && (
            <span className="selectable mt-0.5 block text-meta font-normal text-ink-dim">
              {detail.data.project.path}
            </span>
          )}
        </>
      }
    >
      {detail.isLoading ? (
        <PageSpinner />
      ) : detail.isError ? (
        <PageError error={detail.error} />
      ) : (
        <div className="flex flex-col">
          <SettingsSection first>
            <SectionHeading>Storage</SectionHeading>
            <div className="mt-2 text-body">
              <div className="flex min-w-0 items-center gap-2">
                <Badge tone={detail.data!.storage ? "accent" : "dim"}>
                  {detail.data!.storage?.backend ?? "local folder"}
                </Badge>
                {detail.data!.project.profile && detail.data!.storage && (
                  <span className="shrink-0 text-ink-dim">
                    profile {detail.data!.project.profile}
                  </span>
                )}
                {detail.data!.thoughtsRoot && (
                  <span className="selectable min-w-0 truncate text-meta text-ink-dim">
                    {detail.data!.thoughtsRoot}
                  </span>
                )}
              </div>
              {!detail.data!.thoughtsRoot && !gitBacked && (
                <p className="mt-1 text-ink-dim">
                  Not a git repository, so no branches, worktrees or thoughts. Agents run in the
                  folder itself.
                </p>
              )}
            </div>
          </SettingsSection>

          {gitBacked && <WorktreeFilesCard projectPath={projectPath} />}

          {gitBacked && <BranchPrefixesCard projectPath={projectPath} />}

          <ScriptsCard projectPath={projectPath} />

          <SettingsSection>
            <div className="flex items-center justify-between gap-3">
              <SectionHeading>Backend health</SectionHeading>
              <Button onClick={() => void health.refetch()} busy={health.isFetching}>
                Run checks
              </Button>
            </div>
            <div className="mt-2 divide-y divide-edge/50 pr-1">
              {health.isLoading && <Spinner />}
              {health.data?.checks.map((c) => <HealthRow key={c.id} check={c} />)}
              {health.isError && <ErrorNote error={health.error} />}
            </div>
          </SettingsSection>
        </div>
      )}
    </Dialog>
  );
}
