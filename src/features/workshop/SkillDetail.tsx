import type { ReactNode } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { EditorSection, FormRow } from "@/components/editorForm";
import { Badge, IconButton, TabStrip } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { isSystem } from "@/features/agents/agents";
import type { AgentFile, SkillFile } from "@/lib/bindings";
import { useCopyFeedback } from "@/lib/clipboard";
import { formatBytes, plural } from "@/lib/format";
import { IS_MACOS } from "@/lib/platform";
import { useConnectionStatus } from "@/lib/queries";

import { SkillSource } from "./SkillSource";
import {
  frontmatterValue,
  MAX_SKILL_ASSETS,
  MODELLED_KEYS,
  preapprovedTools,
  SKILL_TABS,
  skillFileRows,
  skillLocation,
  skillSourceLabel,
  type SkillTab,
} from "./skills";

// Wide enough for the source beside its rail; every tab shares it so the header never shifts.
const COLUMN = "mx-auto w-full max-w-6xl";

type DetailProps = {
  skill: SkillFile;
  projectPath: string | null;
  projectName: string | null;
  attached: AgentFile[];
  onOpenAgent: (path: string, target: "agents" | "system") => void;
  tab: SkillTab;
  onTab: (tab: SkillTab) => void;
};

export function SkillDetail(props: DetailProps) {
  const { skill, projectPath, tab, onTab } = props;
  return (
    <div className="flex h-full min-w-0 flex-col overflow-hidden">
      <Header {...props} />
      <div className="shrink-0 border-b border-edge">
        <TabStrip tabs={SKILL_TABS} value={tab} onChange={onTab} className={`${COLUMN} border-b-0 px-7`} />
      </div>
      {tab === "source" ? (
        <div className={`${COLUMN} @container flex min-h-0 flex-1 px-7 py-5`}>
          <div className="flex min-h-0 w-full flex-1 flex-col gap-5 @[720px]:flex-row">
            <SkillSource skill={skill} projectPath={projectPath} />
            <Summary {...props} />
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto pb-8 pt-5">
          <div className={`${COLUMN} @container flex flex-col gap-5 px-7`}>
            {tab === "setup" && <SetupTab {...props} />}
            {tab === "tools" && <ToolsTab {...props} />}
            {tab === "files" && <FilesTab {...props} />}
          </div>
        </div>
      )}
    </div>
  );
}

function Header({ skill, projectName }: DetailProps) {
  const { copied, trigger } = useCopyFeedback();
  const revealLabel = IS_MACOS ? "Reveal in Finder" : "Show in folder";
  // The path is the server's; revealing it would act on this machine.
  const remote = useConnectionStatus().data?.host ?? null;
  const disabledReason = remote ? `The skill is on ${remote.label || remote.destination}` : null;

  return (
    <div className={`${COLUMN} shrink-0 px-7 pb-5 pt-6`}>
      <div className="flex items-center gap-2">
        <h1 className="truncate text-h3 text-ink">{skill.name}</h1>
        <Badge tone="dim">{skillSourceLabel(skill)}</Badge>
        {skill.managed && <Badge tone="accent">Managed</Badge>}
        {skill.userOnly && <Badge tone="dim">User-only</Badge>}
      </div>
      <div className="mt-0.5 flex min-w-0 items-center gap-1">
        <span className="min-w-0 truncate text-meta text-ink-dim">
          {skillLocation(skill, projectName)} · /{skill.slashName}
        </span>
        <IconButton
          className="h-6 w-6"
          title="Copy path"
          aria-label="Copy path"
          onClick={() => trigger(skill.dir)}
        >
          <Icon name="copy" size="sm" />
        </IconButton>
        <IconButton
          className="h-6 w-6"
          disabled={disabledReason != null}
          title={disabledReason ?? revealLabel}
          aria-label={revealLabel}
          onClick={() => void revealItemInDir(skill.dir).catch(() => {})}
        >
          <Icon name="folderOpen" size="sm" />
        </IconButton>
        {copied && <span className="shrink-0 text-ui-sm text-ok">Copied</span>}
      </div>
    </div>
  );
}

function Summary({ skill, attached }: DetailProps) {
  const tools = preapprovedTools(skill);
  const bytes = skill.assets.reduce((sum, a) => sum + (a.bytes ?? 0), skill.bytes ?? 0);
  return (
    <aside className="flex shrink-0 flex-col gap-4 overflow-y-auto @[720px]:w-[280px]">
      <SummaryBlock title="Fires when">
        <p className="text-ui leading-prose text-ink">
          {skill.description || <span className="text-ink-dim">No description</span>}
        </p>
      </SummaryBlock>
      <SummaryBlock title="Invocation">
        <SummaryRow label="Invoked by" value={skill.userOnly ? "User only" : "Model and user"} />
        <SummaryRow label="Slash" value={`/${skill.slashName}`} mono />
        <SummaryRow label="Argument" value={frontmatterValue(skill, "argument-hint") ?? "—"} mono />
        <SummaryRow label="Model" value={frontmatterValue(skill, "model") ?? "Inherit"} />
      </SummaryBlock>
      <SummaryBlock title="Access">
        <SummaryRow label="Pre-approved" value={tools.length ? plural(tools.length, "tool") : "None"} />
        <SummaryRow label="Preloaded by" value={plural(attached.length, "agent")} />
      </SummaryBlock>
      <SummaryBlock title="Files">
        <SummaryRow label="Bundled" value={`${plural(skill.assets.length + 1, "file")} · ${formatBytes(bytes)}`} />
      </SummaryBlock>
    </aside>
  );
}

function SummaryBlock({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 text-label text-ink-dim">{title}</div>
      <div className="flex flex-col gap-1">{children}</div>
    </div>
  );
}

function SummaryRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="grid grid-cols-[96px_minmax(0,1fr)] gap-2 text-ui">
      <span className="text-ink-dim">{label}</span>
      <span className={`truncate text-ink ${mono ? "font-mono text-meta" : ""}`}>{value}</span>
    </div>
  );
}

function Value({ children, mono }: { children: ReactNode; mono?: boolean }) {
  return (
    <div className={`pt-1.5 text-body text-ink ${mono ? "font-mono text-meta" : ""}`}>{children}</div>
  );
}

function SetupTab({ skill, projectName }: DetailProps) {
  const extra = skill.frontmatter.filter((f) => !MODELLED_KEYS.includes(f.key));
  return (
    <>
      <EditorSection title="Identity">
        <FormRow label="Name" hint="Must match the folder name.">
          <Value mono>{skill.name}</Value>
        </FormRow>
        <FormRow label="Description" hint="The only text the agent reads before choosing the skill.">
          <Value>{skill.description || <span className="text-ink-dim">No description</span>}</Value>
        </FormRow>
      </EditorSection>

      <EditorSection title="Invocation">
        <FormRow
          label="Invoked by"
          hint={
            skill.userOnly
              ? "Kept out of automatic matching; it runs only as a slash command."
              : "The agent matches requests against the description."
          }
        >
          <Value>{skill.userOnly ? "User only" : "Model and user"}</Value>
        </FormRow>
        <FormRow label="Slash command">
          <Value mono>/{skill.slashName}</Value>
        </FormRow>
        <FormRow label="Argument hint">
          <Value mono>{frontmatterValue(skill, "argument-hint") ?? "—"}</Value>
        </FormRow>
      </EditorSection>

      <EditorSection title="Runtime">
        <FormRow label="Model" hint="Overrides the session's model while the skill runs.">
          <Value>{frontmatterValue(skill, "model") ?? "Inherit from session"}</Value>
        </FormRow>
      </EditorSection>

      <EditorSection title="Location">
        <FormRow label="Installed in">
          <Value>{skillSourceLabel(skill)}</Value>
        </FormRow>
        <FormRow label="Folder">
          <Value mono>{skillLocation(skill, projectName)}</Value>
        </FormRow>
      </EditorSection>

      {extra.length > 0 && (
        <EditorSection title="Also in the file" description="Other frontmatter keys, shown as written.">
          {extra.map((f, i) => (
            <FormRow key={`${f.key}:${i}`} label={f.key}>
              <Value mono>{f.value || "—"}</Value>
            </FormRow>
          ))}
        </EditorSection>
      )}
    </>
  );
}

function ToolsTab({ skill, attached, onOpenAgent }: DetailProps) {
  const tools = preapprovedTools(skill);
  return (
    <>
      <EditorSection
        title="Pre-approved tools"
        description="Run without a permission prompt during the turn that invokes the skill. Other tools stay callable under your normal permissions."
      >
        {tools.length === 0 ? (
          <p className="text-ui text-ink-dim">
            None. Every tool asks under the session&apos;s permission settings.
          </p>
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {tools.map((t) => (
              <span
                key={t}
                className="rounded-md border border-edge bg-panel-2 px-2 py-0.5 text-meta text-ink"
              >
                {t}
              </span>
            ))}
          </div>
        )}
      </EditorSection>

      <EditorSection title="Preloaded by" description="Agents that load this skill when they start.">
        {attached.length === 0 ? (
          <p className="text-ui text-ink-dim">No agents preload this skill</p>
        ) : (
          <div className="flex flex-col gap-0.5">
            {attached.map((agent) => (
              <button
                key={agent.path}
                type="button"
                onClick={() => onOpenAgent(agent.path, isSystem(agent) ? "system" : "agents")}
                className="flex w-full items-center gap-2 rounded-md border border-transparent px-2.5 py-1.5 text-left text-body text-ink transition-colors hover:bg-edge/40"
              >
                <span className="min-w-0 flex-1 truncate">{agent.name}</span>
                {isSystem(agent) && <Badge tone="dim">System</Badge>}
                <Icon name="chevronRight" size="sm" className="text-ink-dim" />
              </button>
            ))}
          </div>
        )}
      </EditorSection>
    </>
  );
}

function FilesTab({ skill, onTab }: DetailProps) {
  return (
    <EditorSection
      title="Files"
      description="Name and description are always in context; SKILL.md loads when the skill fires; the rest only when it asks."
    >
      <div className="flex flex-col">
        <FileRow name="name + description" step="Always loaded" depth={0} plain />
        {skillFileRows(skill).map((row) => (
          <FileRow
            key={row.path}
            name={row.name}
            depth={row.depth}
            dir={row.dir}
            size={row.bytes == null ? undefined : formatBytes(row.bytes)}
            step={row.step}
            onClick={row.path === "SKILL.md" ? () => onTab("source") : undefined}
          />
        ))}
        {skill.assets.length === MAX_SKILL_ASSETS && (
          <p className="pt-2 text-ui text-ink-dim">Showing the first {MAX_SKILL_ASSETS} files found</p>
        )}
      </div>
    </EditorSection>
  );
}

function FileRow({
  name,
  depth,
  dir,
  plain,
  size,
  step,
  onClick,
}: {
  name: string;
  depth: number;
  dir?: boolean;
  plain?: boolean;
  size?: string;
  step: string;
  onClick?: () => void;
}) {
  const body = (
    <>
      <span className="flex min-w-0 flex-1 items-center gap-2" style={{ paddingLeft: depth * 16 }}>
        {!plain && (
          <span className="shrink-0 text-ink-dim">
            <Icon name={dir ? "folder" : "doc"} size="sm" />
          </span>
        )}
        <span className={`truncate text-meta ${dir ? "text-ink-dim" : "text-ink"}`}>{name}</span>
      </span>
      {size && <span className="shrink-0 text-meta text-ink-dim">{size}</span>}
      <span className="w-24 shrink-0 text-right text-ui text-ink-dim">{step}</span>
    </>
  );
  const cls = "flex w-full items-center gap-3 border-b border-edge/60 py-1.5 text-left last:border-b-0";
  return onClick ? (
    <button type="button" onClick={onClick} className={`${cls} hover:bg-edge/40`}>
      {body}
    </button>
  ) : (
    <div className={cls}>{body}</div>
  );
}
