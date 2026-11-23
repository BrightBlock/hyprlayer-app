
import { useLayoutEffect, useRef, useState } from "react";

import {
  Badge,
  Button,
  Callout,
  ContextMenu,
  ContextMenuItem,
  SegmentedControl,
  TabStrip,
  TextInput,
  Toggle,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
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
import { EditorSection, FormRow } from "@/components/editorForm";

import { FolderCombo, RadioRow } from "./parts";

const COLUMN = "mx-auto w-full max-w-6xl";

const HEADER_ICON =
  "grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-dim hover:text-ink disabled:opacity-50";

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
    isDefault,
    dirty,
    tab,
    onTab,
    onBack,
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
    <div className="flex h-full min-w-0 flex-1 flex-col overflow-hidden">
      <div className={`${COLUMN} flex shrink-0 items-center gap-3 px-7 pb-5 pt-6`}>
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-h3 text-ink">{draft.name || "Untitled agent"}</h1>
            {isDefault && <Badge tone="accent">Default</Badge>}
            {system && <Badge tone="dim">System</Badge>}
            {managed && <Badge tone="dim">Read-only</Badge>}
            {isDraftStatus && (
              <span title="Draft agents can't be spawned by sessions started while they're a draft. Publish to make this one available.">
                <Badge tone="warn">Draft</Badge>
              </span>
            )}
          </div>
          <div className="mt-0.5 flex min-w-0 text-meta text-ink-dim">
            <span className="truncate">
              {`${modelLabel(draft.model, catalog)} · ${
                isAllProjects(draft) ? "all projects" : (props.projectName ?? "this project")
              }`}
            </span>
            {dirty && <span className="shrink-0 whitespace-pre"> · Unsaved</span>}
          </div>
        </div>
        <div className="flex-1" />
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
            <Icon name="more" />
          </button>
        )}
        {(dirty || isNew) && <Button onClick={onBack}>Cancel</Button>}
        {!managed && (
          <Button
            variant="primary"
            onClick={onSave}
            busy={saving}
            disabled={!dirty || !draft.name.trim()}
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
          >
            Save
          </Button>
        )}
      </div>

      <div className="shrink-0 border-b border-edge">
        <TabStrip
          tabs={tabs}
          value={tab}
          onChange={onTab}
          className={`${COLUMN} border-b-0 px-7`}
        />
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
        <div className="min-h-0 flex-1 overflow-y-auto pb-8 pt-5">
          <div className={`${COLUMN} @container flex min-h-full flex-col gap-5 px-7`}>
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
        <Callout tone="warn" icon={<Icon name="permissions" size="sm" />}>
          Hyprlayer manages this linked agent as part of the Claude and Codex bundle. It is
          read-only here so edits cannot change the shared bundle store. Duplicate it from the
          agent menu to create a personal copy.
        </Callout>
      ) : isSystem(draft) ? (
        <Callout tone="warn" icon={<Icon name="permissions" size="sm" />}>
          This agent ships with the <span className="font-mono">{draft.plugin}</span> plugin. Edits
          are saved as a copy in your own library and take precedence over the shipped file.{" "}
          <strong className="font-semibold text-ink">Reset to default</strong> removes the copy at
          any time.
        </Callout>
      ) : null}

      <EditorSection title="Identity">
        <FormRow label="Name">
          <TextInput
            value={draft.name}
            onChange={(e) => patch({ name: e.target.value })}
            placeholder="e.g. Plan Reviewer"
          />
        </FormRow>
        <FormRow label="Description" hint="Claude reads this to decide when to use the agent.">
          <textarea
            ref={descRef}
            value={draft.description}
            onChange={(e) => patch({ description: e.target.value })}
            placeholder="When should this agent be used?"
            className="selectable w-full resize-none rounded-md border border-edge bg-panel-2 px-2.5 py-2 text-body text-ink placeholder:text-ink-dim/70 focus:border-ink-dim/70 focus:outline-none"
          />
        </FormRow>
      </EditorSection>

      <EditorSection title="Runtime">
        <FormRow label="Model" hint="Fixed when a session spawns.">
          <div className="grid grid-cols-2 gap-x-4 gap-y-0.5">
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
        </FormRow>
        <FormRow label="Reasoning effort">
          <SegmentedControl
            aria-label="Reasoning effort"
            value={overlay.effort}
            onChange={(effort) => patchOverlay({ effort })}
            options={EFFORTS.map((e) => ({ value: e.key, label: e.label }))}
          />
        </FormRow>
        <FormRow label="Harness">
          <SegmentedControl
            aria-label="Harness"
            value={overlay.harness}
            onChange={(harness) => patchOverlay({ harness })}
            options={HARNESSES.map((h) => ({ value: h.key as string, label: h.label }))}
          />
        </FormRow>
        <FormRow
          label="Persistent memory"
          hint={
            hasUnknownMemoryScope(draft.memory) ? (
              <span className="text-warn-ink">
                This file sets <code>memory: {draft.memory}</code>, a scope this editor doesn&apos;t
                know. It is left untouched unless you pick one above.
              </span>
            ) : (
              (MEMORY_SCOPES.find((s) => s.key === draft.memory)?.hint ??
              "Starts fresh every invocation")
            )
          }
        >
          <SegmentedControl
            aria-label="Persistent memory"
            value={draft.memory ?? "off"}
            onChange={(key) => patch({ memory: key === "off" ? null : key })}
            options={[{ value: "off", label: "Off" }, ...MEMORY_SCOPES.map((s) => ({ value: s.key, label: s.label }))]}
          />
        </FormRow>
      </EditorSection>

      <EditorSection title="Where it appears">
        <FormRow
          label="All projects"
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
        >
          <Toggle
            checked={isAllProjects(draft)}
            disabled={!projectName}
            onChange={(next) => patch({ source: next ? "user" : "project" })}
            aria-label="Available in all projects"
          />
        </FormRow>
        <FormRow
          label="Default agent"
          hint="The composer opens on this agent unless a stage picks another."
        >
          <Toggle checked={isDefault} onChange={onDefault} aria-label="Default agent" />
        </FormRow>
        <FormRow
          label="Folder"
          hint={
            isSystem(draft)
              ? "Groups this agent in the list only. The shipped file is untouched."
              : "Groups this agent in the list, and travels with the file wherever it goes."
          }
        >
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
        </FormRow>
      </EditorSection>
    </>
  );
}

function PromptTab({ draft, overlay, patch, patchOverlay, contextFiles }: Props) {
  return (
    <>
      <EditorSection
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
          className="selectable min-h-75 w-full flex-1 resize-none rounded-lg border border-edge bg-panel-2 p-3.5 text-meta leading-prose text-ink focus:border-ink-dim/70 focus:outline-none"
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
      </EditorSection>

      <EditorSection
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
                    <Icon name="doc" size="sm" />
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
      </EditorSection>
    </>
  );
}

function CapabilitiesTab({ draft, overlay, patch, patchOverlay, mcpServers, skills, peers }: Props) {
  const extras = extraTools(draft.tools);
  const extraSkillNames = extraSkills(draft.skills, skills);
  return (
    <div className="flex flex-col gap-6">
      <EditorSection
        title="Built-in tools"
        description={toolSummary(draft)}
        action={
          <>
            <Button size="sm" onClick={() => patch({ tools: [] })}>Enable all</Button>
            <Button size="sm" onClick={() => patch({ tools: readOnlyTools(draft) })}>
              Read-only preset
            </Button>
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
                  <Icon name="tool" size="sm" />
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
      </EditorSection>

      <EditorSection
        title="MCP servers"
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
      </EditorSection>

      <EditorSection
        title="Skills"
        description={`Preloaded into this agent when it runs · ${skillSummary(draft, skills)}`}
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
        <div className="mt-3 border-t border-edge/60 pt-3">
          <Toggle
            className="text-ui text-ink-dim"
            checked={skillsRestricted(overlay.rules)}
            onChange={(next) =>
              patchOverlay({ rules: syncSkillRules(overlay.rules, draft.skills, next) })
            }
          >
            Only allow these skills
          </Toggle>
        </div>
      </EditorSection>

      <EditorSection
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
      </EditorSection>
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
      <EditorSection
        title="Permission mode"
        description="Rules below take precedence."
      >
        <div className="grid gap-0.5 @[560px]:grid-cols-2 @[560px]:gap-x-2.5">
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
            <Callout tone="bad" icon={<Icon name="warning" size="sm" />}>
              This agent will run every command without asking. Hyprlayer will only launch it inside
              a worktree, never on your main checkout.
            </Callout>
          </div>
        )}
      </EditorSection>

      <EditorSection
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
                  className="grid h-6 w-6 place-items-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-bad-ink"
                >
                  <Icon name="close" size="xs" />
                </button>
              </div>
            ))}
          </div>
        )}
      </EditorSection>

      <EditorSection title="Reach">
        <FormRow label="Network access" hint="Allow WebFetch and MCP calls to leave the machine.">
          <Toggle
            checked={overlay.network}
            onChange={(network) => patchOverlay({ network })}
            aria-label="Network access"
          />
        </FormRow>
        <FormRow
          label="Outside the worktree"
          hint="Off confines every read and write to this workstream's worktree."
        >
          <Toggle
            checked={overlay.outsideWorktree}
            onChange={(outsideWorktree) => patchOverlay({ outsideWorktree })}
            aria-label="Reach outside the worktree"
          />
        </FormRow>
      </EditorSection>
    </>
  );
}

const PERMISSION_DESCRIPTIONS: Record<string, string> = {
  stage: "Each stage uses its own permission mode.",
  auto: "A safety check approves safe actions and blocks risky ones. Nothing waits for your approval.",
  dontAsk: "Only actions your allow rules cover can run. Anything else is refused without asking you.",
  default: "Every tool call outside the allow rules waits for approval.",
  acceptEdits: "File edits apply straight away; commands still ask.",
  plan: "Reads and reasons, but never writes files or runs commands.",
  bypassPermissions: "No prompts at all. Worktree-only, for throwaway runs.",
};

function CollaboratorsTab({ overlay, patchOverlay, onExport, onCopyPath }: Props) {
  const [invite, setInvite] = useState("");
  return (
    <>
      <EditorSection
        title="Visibility"
        description="Team agents sync through the shared config repo; private ones stay on this machine."
      >
        <SegmentedControl
          aria-label="Visibility"
          value={overlay.visibility}
          onChange={(visibility) => patchOverlay({ visibility })}
          options={[
            { value: "private", label: "Private" },
            { value: "team", label: "Team" },
          ]}
        />
      </EditorSection>

      <EditorSection title="People">
        <div className="flex flex-col gap-1.5">
          {overlay.collaborators.map((c, i) => (
            <div
              key={c.email}
              className="flex items-center gap-2.5 rounded-lg border border-edge bg-panel-2 px-3 py-2"
            >
              <span
                className={`grid h-6 w-6 place-items-center rounded-full text-ui-sm font-semibold leading-none text-accent-ink ${
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
                <Icon name="close" size="xs" />
              </button>
            </div>
          ))}
        </div>
        <div className="mt-3 flex gap-2">
          <div className="flex-1">
            <TextInput
              value={invite}
              onChange={(e) => setInvite(e.target.value)}
              placeholder="Email"
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
      </EditorSection>

      <EditorSection
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

