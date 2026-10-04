import { useMemo, useRef, useState } from "react";

import type { CustomPrompt, Project } from "@/lib/bindings";
import { useAllWorkstreams, useWorkstreamPromptContext } from "@/lib/queries";
import { basename } from "@/lib/path";
import { Callout, Card, EmptyState, ErrorNote, MenuItem, Popover, Spinner } from "@/components/ui";
import { Icon } from "@/components/Icon";
import {
  bodyHasVars,
  defaultPreviewTarget,
  effectiveCustomBody,
  previewArgs,
  previewCandidates,
  resolvePromptBody,
  worktreeStatusLine,
  type BuiltinPromptId,
} from "./prompts";

export function PromptPreview({
  kind,
  builtinId,
  customPrompt,
  builtinBody,
  stock,
  scopeProject,
  projects,
  target,
  onTarget,
}: {
  kind: "builtin" | "custom";
  builtinId?: BuiltinPromptId;
  customPrompt?: CustomPrompt;
  builtinBody: string;
  stock: boolean;
  scopeProject: string | null;
  projects: Project[];
  target: string | null;
  onTarget: (id: string) => void;
}) {
  const all = useAllWorkstreams();
  const candidates = useMemo(() => previewCandidates(all.data ?? []), [all.data]);
  // A stored target can be archived since; fall back rather than render nothing.
  const view =
    candidates.find((v) => v.item.id === target) ?? defaultPreviewTarget(candidates, scopeProject);
  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerRef = useRef<HTMLButtonElement>(null);

  const body =
    kind === "custom" && customPrompt
      ? effectiveCustomBody(customPrompt, view?.item.projectPath ?? null)
      : builtinBody;
  const ctx = useWorkstreamPromptContext(view?.item.id ?? null, bodyHasVars(body));
  const text = useMemo(
    () => resolvePromptBody(body, ctx.data ?? null, previewArgs(body)),
    [body, ctx.data],
  );

  const projectName = (path: string) =>
    projects.find((p) => p.path === path)?.mappedName ?? basename(path);
  const usesOverride = Boolean(
    view && kind === "custom" && customPrompt?.overrides?.[view.item.projectPath] !== undefined,
  );

  return (
    <Card>
      <div className="mb-3 flex items-center gap-2.5">
        <h2 className="text-body font-semibold text-ink">Preview</h2>
        {view && (
          <>
            <button
              ref={pickerRef}
              type="button"
              onClick={() => setPickerOpen((v) => !v)}
              className="inline-flex min-w-0 items-center gap-1 text-ui text-ink-dim hover:text-ink"
            >
              <span className="truncate">
                How it resolves in {projectName(view.item.projectPath)} · {view.item.branch}
              </span>
              <Icon name="chevronDown" size="xs" />
            </button>
            <Popover
              open={pickerOpen}
              onClose={() => setPickerOpen(false)}
              anchorRef={pickerRef}
              className="min-w-56"
            >
              {candidates.map((c) => (
                <MenuItem
                  key={c.item.id}
                  onClick={() => {
                    onTarget(c.item.id);
                    setPickerOpen(false);
                  }}
                >
                  {projectName(c.item.projectPath)} · {c.item.branch}
                </MenuItem>
              ))}
            </Popover>
          </>
        )}
        {(all.isLoading || ctx.isLoading) && (
          <span className="ml-auto">
            <Spinner size="sm" />
          </span>
        )}
      </div>
      {view ? (
        <div className="space-y-2">
          {ctx.isError && <ErrorNote error={ctx.error} />}
          <pre className="selectable max-h-80 overflow-auto whitespace-pre-wrap rounded-md border border-edge bg-panel-2 px-3 py-2.5 text-meta leading-prose text-ink">
            {text}
          </pre>
          <p className="text-ui text-ink-dim">
            {worktreeStatusLine(view.git, view.item.baseBranch)}
            {usesOverride && " · Uses project override"}
          </p>
        </div>
      ) : (
        !all.isLoading && <EmptyState title="No active workstreams" />
      )}
      {kind === "builtin" && builtinId === "commit" && stock && (
        <div className="mt-3">
          <Callout tone="info" icon={<Icon name="info" size="sm" />}>
            Codex and OpenCode sessions use their own stock commit prompt.
          </Callout>
        </div>
      )}
    </Card>
  );
}
