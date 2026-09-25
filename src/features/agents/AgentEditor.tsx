
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

import {
  Badge,
  Button,
  ContextMenu,
  ContextMenuItem,
  Spinner,
  TextInput,
  Toggle,
} from "@/components/ui";
import {
  CheckIcon,
  ChevronRightIcon,
  DocIcon,
  ShieldIcon,
  WarningTriangleIcon,
  WrenchIcon,
  XIcon,
} from "@/components/icons";
import { useSignedIn } from "@/lib/account";
import type {
  AgentContextFile,
  AgentFile,
  AgentMcpServer,
  AgentRule,
  AgentSkill,
} from "@/lib/bindings";
import { HARNESSES } from "@/lib/domain/harnessing";
import { claudeModels, EFFORTS } from "@/lib/domain/models";
import { PERMISSION_MODES } from "@/lib/domain/permissions";
import { PROMPT_VARS } from "@/lib/domain/promptVars";
import { useClaudeProbe } from "@/lib/queries";
import {
  AGENT_TOOLS,
  collaboratorFor,
  editorTabs,
  extraSkills,
  extraTools,
  hasUnknownMemoryScope,
  initialsOf,
  INHERIT_MODEL,
  isAllProjects,
  isSystem,
  isUnsaved,
  MEMORY_SCOPES,
  mcpEnabled,
  mcpTools,
  modelLabel,
  newRule,
  promptStats,
  readOnlyTools,
  setMcp,
  setSkill,
  setTool,
  shortDescription,
  skillEnabled,
  skillsRestricted,
  skillSummary,
  syncSkillRules,
  toolEnabled,
  toolSummary,
  type EditorTab,
  type Overlay,
} from "./agents";
import { Callout, CardGroup, EditorCard, FolderCombo, RadioRow, Segmented } from "./parts";

const COLUMN = "mx-auto w-full max-w-5xl";

const HEADER_ICON_BASE = "grid h-7 w-7 shrink-0 place-items-center rounded-md";
const HEADER_ICON = `${HEADER_ICON_BASE} text-ink-dim hover:text-ink disabled:opacity-50`;
const HEADER_ICON_PRIMARY = `${HEADER_ICON_BASE} text-accent hover:text-accent/80 disabled:text-ink-dim/50`;

const DESC_MIN_H = 3 * 20 + 16;
const DESC_MAX_H = 10 * 20 + 16;

type Props = {
  draft: AgentFile;
  overlay: Overlay;
  isDefault: boolean;
  patch: (patch: Partial<AgentFile>) => void;
  patchOverlay: (patch: Partial<Overlay>) => void;
  onDefault: (next: boolean) => void;
  dirty: boolean;
  tab: EditorTab;
  onTab: (tab: EditorTab) => void;
  onBack: () => void;
  breadcrumb: {
    folder: string;
    onAgents: () => void;
    onFolder: () => void;
  };
  onSave: () => void;
  onStatus: (status: "draft" | "published") => void;
  onReset: () => void;
  onExport: () => void;
  onCopyPath: () => void;
  saving: boolean;
  projectName: string | null;
  folders: string[];
  contextFiles: AgentContextFile[];
  mcpServers: AgentMcpServer[];
  skills: AgentSkill[];
  peers: AgentFile[];
};

export function AgentEditor(props: Props) {
  const {
    draft,
    overlay,
    dirty,
    tab,
    onTab,
    onBack,
    breadcrumb,
    onSave,
    onStatus,
    onReset,
    saving,
  } = props;
  const catalog = claudeModels(useClaudeProbe().data?.models);
  const isNew = isUnsaved(draft);
  const system = isSystem(draft);
  const managed = draft.managed;
  const isDraftStatus = overlay.status === "draft";
  const tabs = editorTabs(useSignedIn());
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);

  return (
    <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
      <Breadcrumb
        folder={breadcrumb.folder}
        agentName={draft.name}
        sectionLabel={tabs.find((t) => t.key === tab)?.label ?? tab}
        onAgents={breadcrumb.onAgents}
        onFolder={breadcrumb.onFolder}
        onAgent={() => onTab("setup")}
      />
      <div className={`${COLUMN} flex shrink-0 items-center gap-3 px-7 pb-8 pt-1.5`}>
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-h3 text-ink">
              {isNew
                ? "New Agent"
                : managed
                  ? "View Managed Agent"
                  : system
                    ? "Edit System Agent"
                    : "Edit Agent"}
            </h1>
            {system && <Badge tone="dim">System</Badge>}
            {managed && <Badge tone="dim">Read-only</Badge>}
            {isDraftStatus && (
              <span title="Draft agents can't be spawned by sessions started while they're a draft. Publish to make this one available.">
                <Badge tone="warn">Draft</Badge>
              </span>
            )}
          </div>
          <div className="mt-0.5 truncate text-meta text-ink-dim">
            {`${draft.name || "untitled"} · ${modelLabel(draft.model, catalog)} · ${
              isAllProjects(draft) ? "all projects" : (props.projectName ?? "this project")
            }`}
          </div>
        </div>
        <div className="flex-1" />
        {dirty && (
          <span
            aria-label="Unsaved changes"
            title="Unsaved changes"
            className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
          />
        )}
        {!managed && (
          <button
            type="button"
            aria-label="More actions"
            onClick={(e) => {
              e.stopPropagation();
              const at = e.currentTarget.getBoundingClientRect();
              setMenuAt({ x: at.right - 150, y: at.bottom + 6 });
            }}
            className={HEADER_ICON}
          >
            ⋮
          </button>
        )}
        <button
          type="button"
          onClick={onBack}
          aria-label="Cancel"
          title="Cancel"
          className={HEADER_ICON}
        >
          <XIcon size={15} />
        </button>
        {!managed && (
          <button
            type="button"
            onClick={onSave}
            disabled={!dirty || !draft.name.trim() || saving}
            aria-label="Save"
            title={
              saving
                ? "Saving…"
                : !draft.name.trim()
                  ? "Name the agent to save"
                  : !dirty
                    ? "No changes to save"
                    : "Save"
            }
            className={HEADER_ICON_PRIMARY}
          >
            {saving ? <Spinner size={15} /> : <CheckIcon size={16} strokeWidth={2.2} />}
          </button>
        )}
      </div>

      {menuAt && (
        <ContextMenu position={menuAt} onClose={() => setMenuAt(null)}>
          {system && (
            <ContextMenuItem
              onClick={() => {
                setMenuAt(null);
                onReset();
              }}
            >
              Reset to default
            </ContextMenuItem>
          )}
          {!system && (
            <ContextMenuItem
              onClick={() => {
                setMenuAt(null);
                onStatus(isDraftStatus ? "published" : "draft");
              }}
              hint={isDraftStatus ? undefined : "Sessions can't spawn a draft agent by name."}
            >
              {isDraftStatus ? "Publish" : "Unpublish"}
            </ContextMenuItem>
          )}
        </ContextMenu>
      )}

      <fieldset disabled={managed} className="contents">
        <div className="min-h-0 flex-1 overflow-y-auto pb-8 pt-2">
          <div className={`${COLUMN} flex min-h-full flex-col gap-3.5 px-7`}>
            {tab === "setup" && <SetupTab {...props} />}
            {tab === "prompt" && <PromptTab {...props} />}
            {tab === "capabilities" && <CapabilitiesTab {...props} />}
            {tab === "permissions" && <PermissionsTab {...props} />}
            {tab === "collaborators" && tabs.some((t) => t.key === tab) && (
              <CollaboratorsTab {...props} />
            )}
          </div>
        </div>
      </fieldset>
    </div>
  );
}

function Breadcrumb({
  folder,
  agentName,
  sectionLabel,
  onAgents,
  onFolder,
  onAgent,
}: {
  folder: string;
  agentName: string;
  sectionLabel: string;
  onAgents: () => void;
  onFolder: () => void;
  onAgent: () => void;
}) {
  const sep = (
    <span aria-hidden="true" className="shrink-0 text-ink-dim">
      <ChevronRightIcon size={12} />
    </span>
  );
  return (
    <nav
      aria-label="Breadcrumb"
      className={`${COLUMN} flex min-w-0 items-center gap-1.5 px-7 pt-4 text-ui`}
    >
      <button
        type="button"
        onClick={onAgents}
        className="max-w-[200px] shrink truncate font-medium text-ink hover:text-ink"
      >
        Agents
      </button>
      {folder && (
        <>
          {sep}
          <button
            type="button"
            onClick={onFolder}
            className="max-w-[200px] shrink truncate font-medium text-ink hover:text-ink"
          >
            {folder}
          </button>
        </>
      )}
      {sep}
      <button
        type="button"
        onClick={onAgent}
        className="max-w-[200px] shrink truncate font-medium text-ink hover:text-ink"
      >
        {agentName || "untitled"}
      </button>
      {sep}
      <span className="shrink truncate text-ink-dim">{sectionLabel}</span>
    </nav>
  );
}

function SetupTab({
  draft,
  overlay,
  isDefault,
  patch,
  patchOverlay,
  onDefault,
  projectName,
  folders,
}: Props) {
  const catalog = claudeModels(useClaudeProbe().data?.models);
  const descRef = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = descRef.current;
    if (!el) return;
    el.style.height = "auto";
    const borders = el.offsetHeight - el.clientHeight;
    const fit = el.scrollHeight + borders;
    el.style.height = `${Math.min(Math.max(fit, DESC_MIN_H), DESC_MAX_H)}px`;
  }, [draft.description]);

  return (
    <>
      {draft.managed ? (
        <Callout tone="warn" icon={<ShieldIcon size={14} />}>
          Hyprlayer manages this linked agent as part of the Claude and Codex bundle. It is
          read-only here so edits cannot change the shared bundle store. Duplicate it from the
          agent menu to create a personal copy.
        </Callout>
      ) : isSystem(draft) ? (
        <Callout tone="warn" icon={<ShieldIcon size={14} />}>
          This agent ships with the <span className="font-mono">{draft.plugin}</span> plugin. Edits
          are saved as a copy in your own library and take precedence over the shipped file.{" "}
          <strong className="font-semibold text-ink">Reset to default</strong> removes the copy at
          any time.
        </Callout>
      ) : null}

      <EditorCard className="flex flex-col gap-4">
        <Labeled label="Name">
          <TextInput
            value={draft.name}
            onChange={(e) => patch({ name: e.target.value })}
            placeholder="e.g. Plan Reviewer"
          />
        </Labeled>
        <Labeled label="Description">
          <textarea
            ref={descRef}
            value={draft.description}
            onChange={(e) => patch({ description: e.target.value })}
            placeholder="When should this agent be used? Claude reads this to decide."
            className="selectable w-full resize-none rounded-md border border-edge bg-panel-2 px-2.5 py-2 text-body text-ink placeholder:text-ink-dim/70 focus:border-ink-dim/70 focus:outline-none"
          />
        </Labeled>

        <div>
          <div className="mb-2 flex items-baseline gap-2">
            <span className="text-ui font-medium text-ink-dim">Model</span>
            <span className="text-ui-sm text-ink-dim opacity-70">Fixed when a session spawns</span>
          </div>
          <div className="grid grid-cols-2 gap-x-6">
            <RadioRow
              name="agent-model"
              checked={!draft.model || draft.model === INHERIT_MODEL}
              onSelect={() => patch({ model: null })}
              label="Inherit from session"
            />
            {catalog.map((m) => (
              <RadioRow
                key={m.key}
                name="agent-model"
                checked={draft.model === m.key}
                disabled={m.disabled}
                onSelect={() => patch({ model: m.key })}
                label={m.label}
                hint={m.disabled ? (m.description ?? "unavailable") : m.hint}
              />
            ))}
          </div>
        </div>

        <div>
          <div className="mb-2 flex items-baseline gap-2">
            <span className="text-ui font-medium text-ink-dim">Persistent memory</span>
            <span className="text-ui-sm text-ink-dim opacity-70">
              What this agent still knows next time it runs
            </span>
          </div>
          <div className="grid grid-cols-2 gap-x-6">
            <RadioRow
              name="agent-memory"
              checked={!draft.memory}
              onSelect={() => patch({ memory: null })}
              label="Off"
              hint="Starts fresh every invocation"
            />
            {MEMORY_SCOPES.map((s) => (
              <RadioRow
                key={s.key}
                name="agent-memory"
                checked={draft.memory === s.key}
                onSelect={() => patch({ memory: s.key })}
                label={s.label}
                hint={s.hint}
              />
            ))}
          </div>
          {hasUnknownMemoryScope(draft.memory) && (
            <p className="mt-1.5 text-ui-sm text-warn-ink">
              This file sets <code>memory: {draft.memory}</code>, a scope this editor doesn&apos;t
              know. It is left untouched unless you pick one above.
            </p>
          )}
        </div>

        <div className="flex flex-wrap gap-8">
          <div>
            <div className="mb-1.5 text-ui font-medium text-ink-dim">Reasoning effort</div>
            <Segmented
              size="sm"
              aria-label="Reasoning effort"
              value={overlay.effort}
              onChange={(effort) => patchOverlay({ effort })}
              options={EFFORTS.map((e) => ({ key: e.key, label: e.label }))}
            />
          </div>
          <div>
            <div className="mb-1.5 text-ui font-medium text-ink-dim">Harness</div>
            <Segmented
              size="sm"
              aria-label="Harness"
              value={overlay.harness}
              onChange={(harness) => patchOverlay({ harness })}
              options={HARNESSES.map((h) => ({ key: h.key as string, label: h.label }))}
            />
          </div>
        </div>
      </EditorCard>

      <EditorCard bare>
        <Row
          title="Available in all projects"
          hint={
            projectName ? (
              <>
                Off moves the file into <span className="font-mono">{projectName}</span>, scoping it
                to that repo.
              </>
            ) : (
              "Open a project to scope an agent to one repo."
            )
          }
          control={
            <Toggle
              checked={isAllProjects(draft)}
              disabled={!projectName}
              onChange={(next) => patch({ source: next ? "user" : "project" })}
              aria-label="Available in all projects"
            />
          }
        />
        <Row
          title="Default agent for new sessions"
          hint="The composer opens on this agent unless a stage picks another."
          control={
            <Toggle checked={isDefault} onChange={onDefault} aria-label="Default agent" />
          }
        />
        <Row
          last
          title="Folder"
          hint={
            isSystem(draft)
              ? "Groups this agent in the list only. The shipped file is untouched."
              : "Groups this agent in the list, and travels with the file wherever it goes."
          }
          control={
            <div className="w-55">
              <FolderCombo
                id="agent-folder"
                value={isSystem(draft) ? overlay.folder : (draft.category ?? overlay.folder)}
                onChange={(folder) => {
                  if (isSystem(draft)) {
                    patchOverlay({ folder });
                    return;
                  }
                  patch({ category: folder.trim() || null });
                  if (overlay.folder) patchOverlay({ folder: "" });
                }}
                folders={folders}
                placeholder="No folder"
                aria-label="Folder"
              />
            </div>
          }
        />
      </EditorCard>
    </>
  );
}

function PromptTab({ draft, overlay, patch, patchOverlay, contextFiles }: Props) {
  return (
    <>
      <EditorCard
        title="System prompt"
        description="Prepended to every session this agent starts, after the harness's own system prompt."
        action={
          <span className="text-meta text-ink-dim">{promptStats(draft.prompt)}</span>
        }
        className="flex min-h-0 flex-1 flex-col"
      >
        <textarea
          value={draft.prompt}
          spellCheck={false}
          onChange={(e) => patch({ prompt: e.target.value })}
          className="selectable min-h-75 w-full flex-1 resize-y rounded-lg border border-edge bg-panel-2 p-3.5 text-meta leading-prose text-ink focus:border-ink-dim/70 focus:outline-none"
        />
        <div className="mt-3 flex shrink-0 flex-wrap items-center gap-2">
          <span className="text-ui-sm text-ink-dim">Insert variable</span>
          {PROMPT_VARS.map((v) => (
            <button
              key={v.token}
              type="button"
              title={v.desc}
              onClick={() =>
                patch({
                  prompt:
                    draft.prompt + (draft.prompt && !draft.prompt.endsWith("\n") ? " " : "") + v.token,
                })
              }
              className="rounded-md border border-edge bg-panel-2 px-2 py-0.5 text-meta text-ink-dim hover:border-ink-dim hover:text-ink"
            >
              {v.token}
            </button>
          ))}
        </div>
      </EditorCard>

      <EditorCard
        title="Appended context"
        description="Docs and rules stitched onto the prompt at launch, in this order."
      >
        {contextFiles.length === 0 ? (
          <p className="text-ui text-ink-dim">
            No CLAUDE.md, <span className="font-mono">.claude/rules</span> or ADRs found in this
            project.
          </p>
        ) : (
          <div className="flex flex-col gap-2">
            {contextFiles.map((c) => {
              const on = overlay.context.includes(c.path);
              return (
                <div
                  key={c.path}
                  className="flex items-center gap-2.5 rounded-lg border border-edge bg-panel-2 px-3 py-2"
                >
                  <span className="grid text-ink-dim">
                    <DocIcon size={14} />
                  </span>
                  <span className="text-meta text-ink">{c.path}</span>
                  <span className="text-ui-sm text-ink-dim">{c.note}</span>
                  <div className="flex-1" />
                  <Toggle
                    checked={on}
                    aria-label={c.path}
                    onChange={(next) =>
                      patchOverlay({
                        context: next
                          ? [...overlay.context, c.path]
                          : overlay.context.filter((p) => p !== c.path),
                      })
                    }
                  />
                </div>
              );
            })}
          </div>
        )}
      </EditorCard>
    </>
  );
}

function CapabilitiesTab({ draft, overlay, patch, patchOverlay, mcpServers, skills, peers }: Props) {
  const extras = extraTools(draft.tools);
  const extraSkillNames = extraSkills(draft.skills, skills);
  return (
    <div className="flex flex-col gap-6">
      <CardGroup label="Tools">
        <EditorCard
          title="Built-in tools"
          description={toolSummary(draft)}
          action={
            <>
              <MiniButton onClick={() => patch({ tools: [] })}>Enable all</MiniButton>
              <MiniButton onClick={() => patch({ tools: readOnlyTools(draft) })}>
                Read-only preset
              </MiniButton>
            </>
          }
        >
          <div className="grid grid-cols-2 gap-x-5">
            {AGENT_TOOLS.map((t) => {
              const on = toolEnabled(draft, t.key);
              return (
                <div
                  key={t.key}
                  className="flex items-center gap-3 border-b border-edge/45 py-2.5"
                >
                  <span className={`grid shrink-0 ${on ? "text-accent" : "text-ink-dim/55"}`}>
                    <WrenchIcon size={13} />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="text-meta text-ink">{t.key}</div>
                    <div className="truncate text-ui-sm text-ink-dim">{t.desc}</div>
                  </div>
                  <Toggle
                    checked={on}
                    aria-label={t.key}
                    onChange={(next) => patch({ tools: setTool(draft, t.key, next) })}
                  />
                </div>
              );
            })}
          </div>
          {extras.length > 0 && (
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <span className="text-ui-sm text-ink-dim">Also in the file</span>
              {extras.map((t) => (
                <span
                  key={t}
                  className="rounded-md border border-edge bg-panel-2 px-2 py-0.5 text-meta text-ink-dim"
                >
                  {t}
                </span>
              ))}
            </div>
          )}
        </EditorCard>

        <EditorCard
          title="MCP servers"
          description="Servers Claude Code is configured with. Turn one on to expose its tools to this agent."
        >
          {mcpServers.length === 0 ? (
            <p className="text-ui text-ink-dim">
              No MCP servers in <span className="font-mono">~/.claude.json</span> or this project&apos;s{" "}
              <span className="font-mono">.mcp.json</span>.
            </p>
          ) : (
            <div className="flex flex-col gap-2.5">
              {mcpServers.map((s) => {
                const on = mcpEnabled(draft, s.name);
                const chips = mcpTools(draft, s.name);
                return (
                  <div key={s.name} className="rounded-lg border border-edge bg-panel-2 p-3">
                    <div className="flex items-center gap-2.5">
                      <span
                        aria-hidden="true"
                        className={`h-[7px] w-[7px] rounded-full ${on ? "bg-ok" : "bg-ink-dim/55"}`}
                      />
                      <span className="text-meta text-ink">{s.name}</span>
                      <span className="text-ui-sm text-ink-dim">
                        {s.transport} · {s.scope}
                      </span>
                      <div className="flex-1" />
                      <Toggle
                        checked={on}
                        aria-label={s.name}
                        onChange={(next) => patch({ tools: setMcp(draft, s.name, next) })}
                      />
                    </div>
                    {chips.length > 0 && (
                      <div className="mt-2.5 flex flex-wrap gap-1.5">
                        {chips.map((t) => (
                          <span
                            key={t}
                            className="rounded-md border border-edge bg-panel px-1.5 py-0.5 text-meta text-ink"
                          >
                            {t}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </EditorCard>
      </CardGroup>

      <CardGroup label="Knowledge">
        <EditorCard
          title="Skills"
          description="Preloaded into this agent when it runs."
          action={
            <>
              <span className="text-meta text-ink-dim">{skillSummary(draft, skills)}</span>
              <Toggle
                className="text-ui-sm text-ink-dim"
                checked={skillsRestricted(overlay.rules)}
                onChange={(next) =>
                  patchOverlay({ rules: syncSkillRules(overlay.rules, draft.skills, next) })
                }
              >
                Only these skills
              </Toggle>
            </>
          }
        >
          {skills.length === 0 ? (
            <p className="text-ui text-ink-dim">
              No skills found in <span className="font-mono">.claude/skills</span> or{" "}
              <span className="font-mono">~/.claude/skills</span>.
            </p>
          ) : (
            <div className="flex flex-col">
              {skills.map((s, i) => {
                const on = skillEnabled(draft, s.name);
                return (
                  <div
                    key={s.name}
                    className={`flex items-center gap-3 py-2.5 ${
                      i === skills.length - 1 ? "" : "border-b border-edge/45"
                    }`}
                  >
                    <span
                      aria-hidden="true"
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${on ? "bg-accent" : "bg-ink-dim/55"}`}
                    />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="truncate text-ui text-ink">{s.name}</span>
                        {s.userOnly && (
                          <span title="Reserved for direct user invocation, but still preloaded into this agent">
                            <Badge tone="dim">user-only</Badge>
                          </span>
                        )}
                      </div>
                      <div className="truncate text-ui-sm text-ink-dim">
                        {shortDescription(s.description)}
                      </div>
                    </div>
                    <div className="shrink-0 text-right text-ui-sm text-ink-dim">
                      <div>{s.source}</div>
                      <div className="font-mono">~{(s.chars / 3.8 / 1000).toFixed(1)}k tokens</div>
                    </div>
                    <Toggle
                      checked={on}
                      aria-label={s.name}
                      onChange={(next) => {
                        const { skills: nextSkills, tools } = setSkill(draft, s.name, next);
                        patch({ skills: nextSkills, tools });
                        patchOverlay({
                          rules: syncSkillRules(overlay.rules, nextSkills, skillsRestricted(overlay.rules)),
                        });
                      }}
                    />
                  </div>
                );
              })}
            </div>
          )}
          {extraSkillNames.length > 0 && (
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <span className="text-ui-sm text-ink-dim">Also in the file</span>
              {extraSkillNames.map((name) => (
                <span
                  key={name}
                  className="rounded-md border border-edge bg-panel-2 px-2 py-0.5 text-meta text-ink-dim"
                >
                  {name}
                </span>
              ))}
            </div>
          )}
        </EditorCard>
      </CardGroup>

      <CardGroup label="Delegation">
        <EditorCard
          title="Can delegate to"
          description={
            <>
              Agents this one may spawn with the <span className="font-mono">Task</span> tool.
            </>
          }
        >
          {peers.length === 0 ? (
            <p className="text-ui text-ink-dim">No other agents to delegate to yet.</p>
          ) : (
            <div className="flex flex-col">
              {peers.map((p, i) => {
                const on = overlay.delegates.includes(p.path);
                return (
                  <div
                    key={p.path}
                    className={`flex items-center gap-3 py-2.5 ${
                      i === peers.length - 1 ? "" : "border-b border-edge/45"
                    }`}
                  >
                    <span
                      aria-hidden="true"
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${on ? "bg-accent" : "bg-ink-dim/55"}`}
                    />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-ui text-ink">{p.name}</div>
                      <div className="truncate text-ui-sm text-ink-dim">
                        {shortDescription(p.description)}
                      </div>
                    </div>
                    <Toggle
                      checked={on}
                      aria-label={p.name}
                      onChange={(next) =>
                        patchOverlay({
                          delegates: next
                            ? [...overlay.delegates, p.path]
                            : overlay.delegates.filter((d) => d !== p.path),
                        })
                      }
                    />
                  </div>
                );
              })}
            </div>
          )}
        </EditorCard>
      </CardGroup>
    </div>
  );
}

const RULE_ACTIONS: ReadonlyArray<{ key: string; tone: string }> = [
  { key: "allow", tone: "var(--color-ok)" },
  { key: "ask", tone: "var(--color-warn)" },
  { key: "deny", tone: "var(--color-bad)" },
];

function PermissionsTab({ overlay, patchOverlay }: Props) {
  const setRule = (id: string, patch: Partial<AgentRule>) =>
    patchOverlay({ rules: overlay.rules.map((r) => (r.id === id ? { ...r, ...patch } : r)) });

  return (
    <>
      <EditorCard
        title="Permission mode"
        description="The posture a session starts in. Rules below still take precedence."
      >
        <div className="flex flex-col gap-0.5">
          {PERMISSION_MODES.map((p) => (
            <RadioRow
              key={p.key}
              name="agent-permission"
              checked={overlay.permissionMode === p.key}
              onSelect={() => patchOverlay({ permissionMode: p.key })}
              label={p.label}
              labelClassName={p.danger ? "text-bad" : "text-ink"}
              description={PERMISSION_DESCRIPTIONS[p.key]}
            />
          ))}
        </div>
        {overlay.permissionMode === "bypassPermissions" && (
          <div className="mt-3">
            <Callout tone="bad" icon={<WarningTriangleIcon size={14} />}>
              This agent will run every command without asking. Hyprlayer will only launch it inside
              a worktree, never on your main checkout.
            </Callout>
          </div>
        )}
      </EditorCard>

      <EditorCard
        title="Rules"
        description={
          <>
            Matched top to bottom; the first hit wins. Patterns use the harness&apos;s{" "}
            <span className="font-mono">Tool(arg:*)</span> syntax.
          </>
        }
        action={
          <Button onClick={() => patchOverlay({ rules: [...overlay.rules, newRule(overlay.rules)] })}>
            Add rule
          </Button>
        }
      >
        {overlay.rules.length === 0 ? (
          <p className="text-ui text-ink-dim">
            No rules yet. Every tool call follows the mode above.
          </p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {overlay.rules.map((r) => (
              <div
                key={r.id}
                className="grid grid-cols-[auto_minmax(0,1fr)_28px] items-center gap-2.5 rounded-lg border border-edge bg-panel-2 px-2 py-1.5"
              >
                <div className="inline-flex gap-0.5 rounded-md bg-panel p-0.5">
                  {RULE_ACTIONS.map((a) => (
                    <button
                      key={a.key}
                      type="button"
                      title={a.key}
                      onClick={() => setRule(r.id, { action: a.key })}
                      className="rounded px-1.5 py-0.5 text-label"
                      style={
                        r.action === a.key
                          ? { background: `color-mix(in oklab, ${a.tone} 18%, transparent)`, color: a.tone }
                          : { color: "color-mix(in oklab, var(--color-ink-dim) 75%, transparent)" }
                      }
                    >
                      {a.key}
                    </button>
                  ))}
                </div>
                <input
                  value={r.pattern}
                  spellCheck={false}
                  onChange={(e) => setRule(r.id, { pattern: e.target.value })}
                  aria-label="Rule pattern"
                  className="selectable w-full border-0 bg-transparent text-meta text-ink outline-none"
                />
                <button
                  type="button"
                  aria-label={`Remove ${r.pattern}`}
                  onClick={() =>
                    patchOverlay({ rules: overlay.rules.filter((x) => x.id !== r.id) })
                  }
                  className="grid h-6 w-6 place-items-center rounded-md text-ink-dim hover:bg-panel-2 hover:text-bad"
                >
                  <XIcon size={12} />
                </button>
              </div>
            ))}
          </div>
        )}
      </EditorCard>

      <EditorCard bare>
        <Row
          title="Network access"
          hint="Allow WebFetch and MCP calls to leave the machine."
          control={
            <Toggle
              checked={overlay.network}
              onChange={(network) => patchOverlay({ network })}
              aria-label="Network access"
            />
          }
        />
        <Row
          last
          title="Reach outside the worktree"
          hint="Off confines every read and write to this workstream's worktree."
          control={
            <Toggle
              checked={overlay.outsideWorktree}
              onChange={(outsideWorktree) => patchOverlay({ outsideWorktree })}
              aria-label="Reach outside the worktree"
            />
          }
        />
      </EditorCard>
    </>
  );
}

const PERMISSION_DESCRIPTIONS: Record<string, string> = {
  auto: "Leave the posture to the stage's own policy.",
  default: "Every tool call outside the allow rules waits for approval.",
  acceptEdits: "File edits apply straight away; commands still ask.",
  plan: "Reads and reasons, but never writes files or runs commands.",
  bypassPermissions: "No prompts at all. Worktree-only, for throwaway runs.",
};

function CollaboratorsTab({ overlay, patchOverlay, onExport, onCopyPath }: Props) {
  const [invite, setInvite] = useState("");
  return (
    <>
      <EditorCard
        title="Visibility"
        description="Team agents sync through the shared config repo; private ones stay on this machine."
      >
        <Segmented
          size="sm"
          aria-label="Visibility"
          value={overlay.visibility}
          onChange={(visibility) => patchOverlay({ visibility })}
          options={[
            { key: "private", label: "Private" },
            { key: "team", label: "Team" },
          ]}
        />
      </EditorCard>

      <EditorCard title="People">
        <div className="flex flex-col gap-1.5">
          {overlay.collaborators.map((c, i) => (
            <div
              key={c.email}
              className="flex items-center gap-2.5 rounded-lg border border-edge bg-panel-2 px-3 py-2"
            >
              <span
                className={`grid h-6 w-6 place-items-center rounded-full text-ui-sm font-semibold text-accent-ink ${
                  i % 2 ? "bg-research" : "bg-accent"
                }`}
              >
                {initialsOf(c.name)}
              </span>
              <div className="min-w-0">
                <div className="truncate text-ui text-ink">{c.name}</div>
                <div className="truncate text-meta text-ink-dim">{c.email}</div>
              </div>
              <div className="flex-1" />
              <span className="text-ui-sm text-ink-dim">{c.role}</span>
              <button
                type="button"
                aria-label={`Remove ${c.email}`}
                onClick={() =>
                  patchOverlay({
                    collaborators: overlay.collaborators.filter((x) => x.email !== c.email),
                  })
                }
                className="grid h-6 w-6 place-items-center rounded-md text-ink-dim hover:bg-panel hover:text-bad"
              >
                <XIcon size={12} />
              </button>
            </div>
          ))}
        </div>
        <div className="mt-3 flex gap-2">
          <div className="flex-1">
            <TextInput
              value={invite}
              onChange={(e) => setInvite(e.target.value)}
              placeholder="name@brightblock.ai"
              aria-label="Invite by email"
            />
          </div>
          <Button
            disabled={!invite.trim()}
            onClick={() => {
              patchOverlay({
                collaborators: [...overlay.collaborators, collaboratorFor(invite.trim())],
              });
              setInvite("");
            }}
          >
            Invite
          </Button>
        </div>
      </EditorCard>

      <EditorCard
        title="Share outside the team"
        description={
          <>
            Exports prompt, model, tools and rules as a portable{" "}
            <span className="font-mono">.agent.json</span>. Credentials are never included.
          </>
        }
        action={
          <>
            <Button onClick={onCopyPath}>Copy path</Button>
            <Button onClick={onExport}>Export</Button>
          </>
        }
      />
    </>
  );
}

function Labeled({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 text-ui font-medium text-ink-dim">{label}</div>
      {children}
    </div>
  );
}

function Row({
  title,
  hint,
  control,
  last,
}: {
  title: ReactNode;
  hint: ReactNode;
  control: ReactNode;
  last?: boolean;
}) {
  return (
    <div
      className={`flex items-start justify-between gap-4 py-3.5 ${last ? "" : "border-b border-edge/60"}`}
    >
      <div className="min-w-0">
        <div className="text-body text-ink">{title}</div>
        <p className="mt-0.5 text-ui text-ink-dim">{hint}</p>
      </div>
      <div className="shrink-0 pt-0.5">{control}</div>
    </div>
  );
}

function MiniButton({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-md border border-edge bg-panel-2 px-2 py-1 text-ui-sm text-ink-dim hover:text-ink"
    >
      {children}
    </button>
  );
}
