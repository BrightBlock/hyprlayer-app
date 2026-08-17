/**
 * The agent editor: Setup / Prompt / Capabilities / Permissions / Collaborators
 * over one working copy of a `.claude/agents/*.md` file and its app-side
 * launch settings, saved by the container.
 */

import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

import {
  Badge,
  Button,
  ContextMenu,
  ContextMenuItem,
  DropdownSelect,
  Spinner,
  TextInput,
  Toggle,
} from "@/components/ui";
import {
  CheckIcon,
  ChevronLeftIcon,
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
import { EFFORTS, MODELS } from "@/lib/domain/models";
import { PERMISSION_MODES } from "@/lib/domain/permissions";
import { PROMPT_VARS } from "@/lib/domain/promptVars";
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
import { Callout, CardGroup, EditorCard, RadioRow, Segmented } from "./parts";

/** The editor reads as a settings page: one centered column, unlike the full-width list. */
const COLUMN = "mx-auto w-full max-w-5xl";

/** Header glyphs take no fill, so the row reads as one strip of icons rather
 *  than a cluster of competing buttons — colour alone carries hover and rank.
 *  Split so the accent variant never fights the dim one over `text-*`, which
 *  Tailwind resolves by stylesheet order rather than class order. */
const HEADER_ICON_BASE = "grid h-7 w-7 shrink-0 place-items-center rounded-md";
const HEADER_ICON = `${HEADER_ICON_BASE} text-ink-dim hover:text-ink disabled:opacity-50`;
/** The commit action: accent-coloured rather than accent-filled, and visibly
 *  inert when there is nothing to save. */
const HEADER_ICON_PRIMARY = `${HEADER_ICON_BASE} text-accent hover:text-accent/80 disabled:text-ink-dim/50`;

/** Description field bounds, in whole 20px lines (`text-sm`) plus 16px of
 *  `py-2` — a fractional height is what clipped the last line before. */
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
  onPublish: () => void;
  onReset: () => void;
  onExport: () => void;
  onCopyPath: () => void;
  saving: boolean;
  /** Null when this agent has no project scope to move to. */
  projectName: string | null;
  folders: string[];
  contextFiles: AgentContextFile[];
  mcpServers: AgentMcpServer[];
  skills: AgentSkill[];
  /** Everything this agent could delegate to — its own library, minus itself. */
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
    onSave,
    onPublish,
    onReset,
    saving,
  } = props;
  const isNew = isUnsaved(draft);
  const system = isSystem(draft);
  const isDraftStatus = overlay.status === "draft";
  const tabs = editorTabs(useSignedIn());
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);

  return (
    <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
      {/* Title, tabs and actions share one row: a 5-tab strip and four text
          buttons don't fit the column, so the two rare actions (reset,
          publish) live behind the kebab and Cancel keeps only its glyph.
          The gap below is here rather than in the scroller's padding, which
          would scroll away and leave content touching the strip. */}
      <div className={`${COLUMN} flex shrink-0 items-center gap-3 px-7 pb-8 pt-4`}>
        <button type="button" onClick={onBack} aria-label="Back to agents" className={HEADER_ICON}>
          <ChevronLeftIcon size={15} />
        </button>
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-lg font-semibold tracking-tight text-ink">
              {isNew ? "New Agent" : system ? "Edit System Agent" : "Edit Agent"}
            </h1>
            {system && <Badge tone="dim">System</Badge>}
            {isDraftStatus && (
              <span title="Draft agents can't be spawned by sessions started while they're a draft — publish to make this one available.">
                <Badge tone="warn">Draft</Badge>
              </span>
            )}
          </div>
          <div className="mt-0.5 truncate font-mono text-xs text-ink-dim">
            {`${draft.name || "untitled"} · ${modelLabel(draft.model)} · ${
              isAllProjects(draft) ? "all projects" : (props.projectName ?? "this project")
            }`}
          </div>
        </div>
        <div className="flex-1" />
        <Segmented aria-label="Agent settings" value={tab} onChange={onTab} options={tabs} />
        {dirty && (
          <span
            aria-label="Unsaved changes"
            title="Unsaved changes"
            className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
          />
        )}
        {(system || isDraftStatus) && (
          <button
            type="button"
            aria-label="More actions"
            onClick={(e) => {
              // The menu closes on any window click, so the opener has to keep
              // its own click from reaching that listener. Anchor off the
              // button, not the pointer, so it lands the same place every time.
              e.stopPropagation();
              const at = e.currentTarget.getBoundingClientRect();
              setMenuAt({ x: at.right - 150, y: at.bottom + 6 });
            }}
            className={`${HEADER_ICON} tracking-widest`}
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
        <button
          type="button"
          onClick={onSave}
          disabled={!dirty || !draft.name.trim() || saving}
          aria-label="Save"
          // The label used to say why it was inert just by being greyed next to
          // the word "Save"; as a glyph it has to spell that out.
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
          {isDraftStatus && (
            <ContextMenuItem
              onClick={() => {
                setMenuAt(null);
                onPublish();
              }}
            >
              Publish
            </ContextMenuItem>
          )}
        </ContextMenu>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto pb-8 pt-2">
        {/* min-h-full so a tab can hand its tall control the leftover height. */}
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
  const folderOptions = [
    { value: "", label: "No folder" },
    ...folders.map((f) => ({ value: f, label: f })),
  ];

  // Descriptions run to whole paragraphs (that's why the list elides them), so
  // the field grows to its content instead of clipping at a fixed height that
  // wasn't even a whole number of lines. Past the cap it scrolls.
  const descRef = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = descRef.current;
    if (!el) return;
    el.style.height = "auto";
    // `height` sets the border box here but scrollHeight measures the padding
    // box, so the borders have to be added back or the last line stays clipped.
    const borders = el.offsetHeight - el.clientHeight;
    const fit = el.scrollHeight + borders;
    el.style.height = `${Math.min(Math.max(fit, DESC_MIN_H), DESC_MAX_H)}px`;
  }, [draft.description]);

  return (
    <>
      {isSystem(draft) && (
        <Callout tone="warn" icon={<ShieldIcon size={14} />}>
          This agent ships with the <span className="font-mono">{draft.plugin}</span> plugin. Edits
          are saved as a copy in your own library and take precedence over the shipped file —{" "}
          <strong className="font-semibold text-ink">Reset to default</strong> removes the copy at
          any time.
        </Callout>
      )}

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
            className="selectable w-full resize-none rounded-md border border-edge bg-panel-2 px-2.5 py-2 text-sm text-ink placeholder:text-ink-dim/70 focus:border-ink-dim/70 focus:outline-none"
          />
        </Labeled>

        <div>
          <div className="mb-2 flex items-baseline gap-2">
            <span className="text-xs font-medium text-ink-dim">Model</span>
            <span className="text-[11px] text-ink-dim opacity-70">Fixed when a session spawns</span>
          </div>
          <div className="grid grid-cols-2 gap-x-6">
            <RadioRow
              name="agent-model"
              checked={!draft.model || draft.model === INHERIT_MODEL}
              onSelect={() => patch({ model: null })}
              label="Inherit from session"
            />
            {MODELS.map((m) => (
              <RadioRow
                key={m.key}
                name="agent-model"
                checked={draft.model === m.key}
                onSelect={() => patch({ model: m.key })}
                label={m.label}
                hint={m.hint}
              />
            ))}
          </div>
        </div>

        <div>
          <div className="mb-2 flex items-baseline gap-2">
            <span className="text-xs font-medium text-ink-dim">Persistent memory</span>
            <span className="text-[11px] text-ink-dim opacity-70">
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
            <p className="mt-1.5 text-[11px] text-warn">
              This file sets <code>memory: {draft.memory}</code>, a scope this editor doesn&apos;t
              know. It is left untouched unless you pick one above.
            </p>
          )}
        </div>

        <div className="flex flex-wrap gap-8">
          <div>
            <div className="mb-1.5 text-xs font-medium text-ink-dim">Reasoning effort</div>
            <Segmented
              size="sm"
              aria-label="Reasoning effort"
              value={overlay.effort}
              onChange={(effort) => patchOverlay({ effort })}
              options={EFFORTS.map((e) => ({ key: e.key, label: e.label }))}
            />
          </div>
          <div>
            <div className="mb-1.5 text-xs font-medium text-ink-dim">Harness</div>
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
          hint={folders.length ? "Folders group agents in this list only." : "No folders yet."}
          control={
            <div className="w-55">
              <DropdownSelect
                value={overlay.folder}
                options={folderOptions}
                onChange={(folder) => patchOverlay({ folder })}
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
          <span className="font-mono text-[11px] text-ink-dim">{promptStats(draft.prompt)}</span>
        }
        className="flex min-h-0 flex-1 flex-col"
      >
        <textarea
          value={draft.prompt}
          spellCheck={false}
          onChange={(e) => patch({ prompt: e.target.value })}
          className="selectable min-h-75 w-full flex-1 resize-y rounded-lg border border-edge bg-panel-2 p-3.5 font-mono text-xs leading-relaxed text-ink focus:border-ink-dim/70 focus:outline-none"
        />
        <div className="mt-3 flex shrink-0 flex-wrap items-center gap-2">
          <span className="text-[11px] text-ink-dim">Insert variable</span>
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
              className="rounded-md border border-edge bg-panel-2 px-2 py-0.5 font-mono text-[11px] text-ink-dim hover:border-ink-dim hover:text-ink"
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
          <p className="text-xs text-ink-dim">
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
                  <span className="font-mono text-xs text-ink">{c.path}</span>
                  <span className="text-[11px] text-ink-dim">{c.note}</span>
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
    // Wider gap between groups than between the cards inside one, so the
    // eyebrows read as section breaks rather than more card titles.
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
                    <div className="font-mono text-xs text-ink">{t.key}</div>
                    <div className="truncate text-[11px] text-ink-dim">{t.desc}</div>
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
              <span className="text-[11px] text-ink-dim">Also in the file</span>
              {extras.map((t) => (
                <span
                  key={t}
                  className="rounded-md border border-edge bg-panel-2 px-2 py-0.5 font-mono text-[11px] text-ink-dim"
                >
                  {t}
                </span>
              ))}
            </div>
          )}
        </EditorCard>

        <EditorCard
          title="MCP servers"
          description="Servers claude is configured with. Turn one on to expose its tools to this agent."
        >
          {mcpServers.length === 0 ? (
            <p className="text-xs text-ink-dim">
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
                      <span className="font-mono text-xs text-ink">{s.name}</span>
                      <span className="text-[11px] text-ink-dim">
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
                            className="rounded-md border border-edge bg-panel px-1.5 py-0.5 font-mono text-[11px] text-ink"
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
          description="Preloaded into this agent when it runs, and matched by the rules below."
          action={
            <>
              <span className="font-mono text-[11px] text-ink-dim">{skillSummary(draft, skills)}</span>
              <label className="flex items-center gap-2 text-[11px] text-ink-dim">
                Only these skills
                <Toggle
                  checked={skillsRestricted(overlay.rules)}
                  aria-label="Only these skills"
                  onChange={(next) =>
                    patchOverlay({ rules: syncSkillRules(overlay.rules, draft.skills, next) })
                  }
                />
              </label>
            </>
          }
        >
          {skills.length === 0 ? (
            <p className="text-xs text-ink-dim">
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
                        <span className="truncate text-xs text-ink">{s.name}</span>
                        {s.userOnly && (
                          <span title="Reserved for direct user invocation — still preloaded into this agent">
                            <Badge tone="dim">user-only</Badge>
                          </span>
                        )}
                      </div>
                      <div className="truncate text-[11px] text-ink-dim">
                        {shortDescription(s.description)}
                      </div>
                    </div>
                    <div className="shrink-0 text-right text-[11px] text-ink-dim">
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
              <span className="text-[11px] text-ink-dim">Also in the file</span>
              {extraSkillNames.map((name) => (
                <span
                  key={name}
                  className="rounded-md border border-edge bg-panel-2 px-2 py-0.5 font-mono text-[11px] text-ink-dim"
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
            <p className="text-xs text-ink-dim">No other agents to delegate to yet.</p>
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
                      <div className="truncate text-xs text-ink">{p.name}</div>
                      <div className="truncate text-[11px] text-ink-dim">
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
          <p className="text-xs text-ink-dim">
            No rules — every tool call follows the mode above.
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
                      className="rounded px-1.5 py-0.5 text-[10.5px] font-semibold uppercase tracking-wide"
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
                  className="selectable w-full border-0 bg-transparent font-mono text-xs text-ink outline-none"
                />
                <button
                  type="button"
                  aria-label={`Remove ${r.pattern}`}
                  onClick={() =>
                    patchOverlay({ rules: overlay.rules.filter((x) => x.id !== r.id) })
                  }
                  className="grid h-6 w-6 place-items-center rounded-md text-ink-dim hover:bg-panel hover:text-bad"
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

/** One line each, keyed by the shared permission-mode vocabulary. */
const PERMISSION_DESCRIPTIONS: Record<string, string> = {
  auto: "Leave the posture to the stage's own policy.",
  default: "Every tool call outside the allow rules waits for approval.",
  acceptEdits: "File edits apply straight away; commands still ask.",
  plan: "Reads and reasons, but never writes files or runs commands.",
  bypassPermissions: "No prompts at all — worktree-only, for throwaway runs.",
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
                className={`grid h-6.5 w-6.5 place-items-center rounded-full text-[11px] font-semibold text-accent-ink ${
                  i % 2 ? "bg-research" : "bg-accent"
                }`}
              >
                {initialsOf(c.name)}
              </span>
              <div className="min-w-0">
                <div className="truncate text-xs text-ink">{c.name}</div>
                <div className="truncate font-mono text-[11px] text-ink-dim">{c.email}</div>
              </div>
              <div className="flex-1" />
              <span className="text-[11px] text-ink-dim">{c.role}</span>
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
      <div className="mb-1.5 text-xs font-medium text-ink-dim">{label}</div>
      {children}
    </div>
  );
}

/** A divided settings row inside a `bare` card. */
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
        <div className="text-sm text-ink">{title}</div>
        <p className="mt-0.5 text-xs text-ink-dim">{hint}</p>
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
      className="rounded-md border border-edge bg-panel-2 px-2 py-1 text-[11px] text-ink-dim hover:text-ink"
    >
      {children}
    </button>
  );
}
