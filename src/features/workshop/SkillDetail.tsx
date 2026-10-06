import { useMemo, useState } from "react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import CodeMirror from "@uiw/react-codemirror";

import {
  Badge,
  Button,
  ConfirmDialog,
  ContextMenu,
  ContextMenuItem,
  ErrorNote,
  IconButton,
  MenuSeparator,
  rowClass,
  Spinner,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
import { isSystem } from "@/features/agents/agents";
import { MarkdownPreview } from "@/features/library/MarkdownPreview";
import type { AgentFile, EditorTarget, SkillFile, SkillInstallPreview } from "@/lib/bindings";
import { readOnlyExtensions, useLangForPath } from "@/lib/codeView";
import { formatBytes, plural, shortSha } from "@/lib/format";
import { IS_MACOS } from "@/lib/platform";
import { log } from "@/lib/log";
import {
  useConnectionStatus,
  useEditorTargets,
  usePreviewSkillUpdate,
  useRemoveSkill,
  useSkillAssetText,
  useSkillFileText,
} from "@/lib/queries";
import { useToast } from "@/lib/toast";

import { RailRow, RailSection, RepoLink, SkillDetailFrame } from "./SkillDetailFrame";
import { SkillReviewDialog } from "./SkillReviewDialog";
import {
  frontmatterValue,
  MAX_SKILL_ASSETS,
  preapprovedTools,
  SKILL_TABS,
  SKILL_TARGET_BADGE,
  skillBody,
  skillFileRows,
  skillFolder,
  skillLocation,
  skillScope,
  skillSourceLabel,
  skillTokenLine,
  type SkillTab,
} from "./skills";

type DetailProps = {
  skill: SkillFile;
  projectPath: string | null;
  projectName: string | null;
  updateAvailable: boolean;
  attached: AgentFile[];
  onOpenAgent: (path: string, target: "agents" | "system") => void;
  tab: SkillTab;
  onTab: (tab: SkillTab) => void;
  onSelect: (path: string | null) => void;
};

export function SkillDetail(props: DetailProps) {
  const { skill, tab, onTab } = props;
  const tabs = SKILL_TABS.map((key) => ({
    key,
    label:
      key === "instructions" ? (
        "Instructions"
      ) : (
        <>
          Files <span className="text-ink-dim">{skill.assets.length + 1}</span>
        </>
      ),
  }));
  return (
    <SkillDetailFrame
      header={<Header {...props} />}
      tabs={tabs}
      tab={tab}
      onTab={onTab}
      body={tab === "files" ? <FilesTab {...props} /> : <InstructionsTab {...props} />}
      rail={<Rail {...props} />}
    />
  );
}

function openIn(app: EditorTarget, path: string) {
  const opened = app.openWith ? openPath(path, app.openWith) : openPath(path);
  void opened.catch((e) => log.error("skills", `opening in ${app.name} failed`, e));
}

function Header({ skill, projectPath, projectName, updateAvailable, onSelect }: DetailProps) {
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [removing, setRemoving] = useState(false);
  const [reviewing, setReviewing] = useState<SkillInstallPreview | null>(null);
  const remove = useRemoveSkill();
  const previewUpdate = usePreviewSkillUpdate();
  const scope = skillScope(skill);
  const revealLabel = IS_MACOS ? "Reveal in Finder" : "Show in folder";
  // The path is the server's; revealing it would act on this machine.
  const remote = useConnectionStatus().data?.host ?? null;
  const closeMenu = () => setMenuAt(null);
  const targets = useEditorTargets();
  // A remote server's path doesn't exist on this machine.
  const editors = useMemo(() => {
    const apps = (targets.data ?? []).filter((a) => a.id !== "files");
    const first = apps.find((a) => a.kind === "editor") ?? apps[0];
    return remote || !first ? [] : [first, ...apps.filter((a) => a.id !== first.id)];
  }, [targets.data, remote]);
  const [editor, ...otherEditors] = editors;

  return (
    <div className="flex items-start gap-4">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <h1 className="truncate text-h3 text-ink">{skill.name}</h1>
          <Badge tone="dim">{skillSourceLabel(skill)}</Badge>
          {skill.managed && <Badge tone="accent">Managed</Badge>}
          {skill.userOnly && <Badge tone="dim">User-only</Badge>}
        </div>
        {skill.description && (
          <p className="mt-1 line-clamp-2 max-w-[640px] text-body leading-prose text-ink-2">{skill.description}</p>
        )}
        {previewUpdate.isError && (
          <div className="mt-2">
            <ErrorNote error={previewUpdate.error} onDismiss={previewUpdate.reset} />
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        {skill.installed && updateAvailable && (
          <Button
            variant="primary"
            size="sm"
            busy={previewUpdate.isPending}
            onClick={() =>
              previewUpdate.mutate({ projectPath, scope, name: skillFolder(skill) }, { onSuccess: setReviewing })
            }
          >
            Review update
          </Button>
        )}
        {editor && (
          <Button size="sm" onClick={() => openIn(editor, skill.dir)}>
            {`Open in ${editor.name}`}
          </Button>
        )}
        <IconButton
          aria-label="More actions"
          title="More actions"
          onClick={(e) => {
            e.stopPropagation();
            const at = e.currentTarget.getBoundingClientRect();
            setMenuAt({ x: at.right - 180, y: at.bottom + 6 });
          }}
        >
          <Icon name="more" size="sm" />
        </IconButton>
      </div>
      {menuAt && (
        <ContextMenu position={menuAt} onClose={closeMenu}>
          {otherEditors.map((app) => (
            <ContextMenuItem
              key={app.id}
              onClick={() => {
                closeMenu();
                openIn(app, skill.dir);
              }}
            >
              {`Open in ${app.name}`}
            </ContextMenuItem>
          ))}
          {otherEditors.length > 0 && <MenuSeparator />}
          <ContextMenuItem
            onClick={() => {
              closeMenu();
              void navigator.clipboard.writeText(skill.dir).then(() => useToast.getState().show("Path copied", skill.dir));
            }}
          >
            Copy path
          </ContextMenuItem>
          <ContextMenuItem
            disabled={remote != null}
            title={remote ? `The skill is on ${remote.label || remote.destination}` : undefined}
            onClick={() => {
              closeMenu();
              void revealItemInDir(skill.dir).catch(() => {});
            }}
          >
            {revealLabel}
          </ContextMenuItem>
          {skill.installed && (
            <>
              <MenuSeparator />
              <ContextMenuItem
                danger
                onClick={() => {
                  closeMenu();
                  setRemoving(true);
                }}
              >
                Remove…
              </ContextMenuItem>
            </>
          )}
        </ContextMenu>
      )}
      <SkillReviewDialog
        preview={reviewing}
        scope={scope}
        projectPath={projectPath}
        projectName={projectName}
        onDone={() => setReviewing(null)}
        onCancel={() => setReviewing(null)}
      />
      <ConfirmDialog
        open={removing}
        onClose={() => setRemoving(false)}
        title={`Remove ${skill.name}?`}
        confirmLabel="Remove"
        confirmVariant="danger"
        busy={remove.isPending}
        error={remove.error}
        onDismissError={remove.reset}
        onConfirm={() =>
          remove.mutate(
            { projectPath, scope, name: skillFolder(skill) },
            {
              onSuccess: () => {
                setRemoving(false);
                onSelect(null);
                useToast.getState().show("Skill removed", skill.name);
              },
            },
          )
        }
      >
        <p className="text-body text-ink-dim">
          {scope === "project"
            ? `Deletes its files and links from ${projectName ?? "this project"}.`
            : "Deletes its files and links globally."}
        </p>
      </ConfirmDialog>
    </div>
  );
}

function InstructionsTab({ skill, projectPath }: DetailProps) {
  const text = useSkillFileText(projectPath, skill.path);
  if (text.isLoading) {
    return (
      <div className="flex h-24 items-center justify-center">
        <Spinner />
      </div>
    );
  }
  if (text.isError) return <ErrorNote error={text.error} />;
  const body = skillBody(text.data ?? "").trim();
  return (
    <div>
      {body ? <MarkdownPreview body={body} /> : <p className="text-ui text-ink-dim">SKILL.md has no instructions</p>}
    </div>
  );
}

function FilesTab({ skill, projectPath }: DetailProps) {
  const rows = skillFileRows(skill);
  const [picked, setPicked] = useState("SKILL.md");
  const selected = rows.some((r) => !r.dir && r.path === picked) ? picked : "SKILL.md";
  return (
    <div className="flex flex-col gap-4 @[960px]:flex-row @[960px]:items-start">
      <div className="flex shrink-0 flex-col @[960px]:w-[240px]">
        {rows.map((row) =>
          row.dir ? (
            <div
              key={row.path}
              className="flex h-7 items-center gap-2 px-2 text-meta text-ink-dim"
              style={{ paddingLeft: 8 + row.depth * 16 }}
            >
              <Icon name="folder" size="sm" />
              <span className="truncate">{row.name}</span>
            </div>
          ) : (
            <button
              key={row.path}
              type="button"
              aria-current={row.path === selected}
              onClick={() => setPicked(row.path)}
              className={rowClass({
                tier: "compact",
                selected: row.path === selected ? "tint" : "none",
                className: "flex w-full gap-2 px-2 text-left",
              })}
              style={{ paddingLeft: 8 + row.depth * 16 }}
            >
              <Icon name="doc" size="sm" className="shrink-0 text-ink-dim" />
              <span title={row.path} className="min-w-0 flex-1 truncate text-meta text-ink">
                {row.name}
              </span>
              {row.bytes != null && <span className="shrink-0 text-meta text-ink-dim">{formatBytes(row.bytes)}</span>}
            </button>
          ),
        )}
        {skill.assets.length === MAX_SKILL_ASSETS && (
          <p className="px-2 pt-2 text-ui text-ink-dim">Showing the first {MAX_SKILL_ASSETS} files found</p>
        )}
      </div>
      <FileViewer key={selected} projectPath={projectPath} skillPath={skill.path} relPath={selected} />
    </div>
  );
}

function FileViewer({ projectPath, skillPath, relPath }: { projectPath: string | null; skillPath: string; relPath: string }) {
  const asset = useSkillAssetText(projectPath, skillPath, relPath);
  const lang = useLangForPath(relPath);
  const extensions = useMemo(() => readOnlyExtensions(lang), [lang]);
  return (
    <div className="min-w-0 flex-1 overflow-hidden rounded-card border border-edge">
      <div className="flex h-9 items-center gap-3 border-b border-edge/60 bg-panel-2 px-3">
        <span className="min-w-0 flex-1 truncate font-mono text-meta text-ink">{relPath}</span>
        {asset.data && <span className="shrink-0 text-meta text-ink-dim">{formatBytes(asset.data.bytes ?? 0)}</span>}
      </div>
      {asset.isLoading ? (
        <div className="flex h-24 items-center justify-center">
          <Spinner />
        </div>
      ) : asset.isError ? (
        <div className="p-3">
          <ErrorNote error={asset.error} />
        </div>
      ) : asset.data?.text == null ? (
        <p className="px-3 py-6 text-center text-ui text-ink-dim">Binary or too large to show</p>
      ) : (
        <CodeMirror value={asset.data.text} theme="none" editable={false} extensions={extensions} basicSetup={false} />
      )}
    </div>
  );
}

function Rail({ skill, projectName, updateAvailable, attached, onOpenAgent }: DetailProps) {
  const tools = preapprovedTools(skill);
  const argument = frontmatterValue(skill, "argument-hint");
  const bytes = skill.assets.reduce((sum, a) => sum + (a.bytes ?? 0), skill.bytes ?? 0);
  const origin = skill.origin;

  return (
    <>
      <RailSection title="Source">
        {origin && (
          <>
            <RailRow
              label="Repository"
              mono
              value={<RepoLink repo={origin.repo} />}
            />
            {origin.path && <RailRow label="Path" value={origin.path} mono />}
            <RailRow
              label="Commit"
              mono
              value={
                <>
                  {shortSha(origin.commit)} <span className="text-ink-dim">{origin.ref}</span>
                  {updateAvailable && <span className="mt-0.5 block font-sans text-ui text-accent">Update available</span>}
                </>
              }
            />
          </>
        )}
        <RailRow label="Location" value={skillLocation(skill, projectName)} mono />
        {skill.installed && skill.targets.length > 0 && (
          <RailRow
            label="For"
            value={
              <span className="flex flex-wrap gap-1">
                {skill.targets.map((t) => (
                  <Badge key={t} tone="dim">
                    {SKILL_TARGET_BADGE[t]}
                  </Badge>
                ))}
              </span>
            }
          />
        )}
      </RailSection>
      <RailSection title="Loaded by">
        {attached.length === 0 ? (
          <span className="col-span-2 text-ui text-ink-dim">No agents preload it</span>
        ) : (
          <span className="col-span-2 flex flex-col items-start gap-1">
            {attached.map((agent) => (
              <button
                key={agent.path}
                type="button"
                onClick={() => onOpenAgent(agent.path, isSystem(agent) ? "system" : "agents")}
                className="max-w-full truncate text-left text-ui text-ink underline decoration-edge underline-offset-2 hover:decoration-ink-dim"
              >
                {agent.name}
              </button>
            ))}
          </span>
        )}
      </RailSection>
      <RailSection title="Invocation">
        <RailRow label="Invoked by" value={skill.userOnly ? "User only" : "Model and user"} />
        <RailRow label="Slash" value={`/${skill.slashName}`} mono />
        {argument && <RailRow label="Argument" value={argument} mono />}
        <RailRow label="Model" value={frontmatterValue(skill, "model") ?? "Inherit"} />
      </RailSection>
      <RailSection title="Access">
        <RailRow
          label="Pre-approved"
          value={
            tools.length === 0 ? (
              "None"
            ) : (
              <span className="flex flex-wrap gap-1">
                {tools.map((t) => (
                  <span key={t} className="rounded-md border border-edge bg-panel-2 px-1.5 font-mono text-meta text-ink">
                    {t}
                  </span>
                ))}
              </span>
            )
          }
        />
      </RailSection>
      <RailSection title="Size">
        <RailRow label="Instructions" value={skillTokenLine(skill)} />
        <RailRow label="Bundled" value={`${plural(skill.assets.length + 1, "file")} · ${formatBytes(bytes)}`} />
      </RailSection>
    </>
  );
}
