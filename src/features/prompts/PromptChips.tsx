import { useMemo } from "react";

import type { CustomPrompt, WorkstreamView } from "../../lib/bindings";
import { useBootstrap } from "../../lib/queries";
import { ChordHint } from "../../components/ui";
import { SparkleIcon } from "../../components/icons";
import { customPrompts, effectiveCustomBody } from "./prompts";
import { runCustomPrompt } from "./runPrompt";

/**
 * The pinned prompts that should surface as composer chips for a project. An
 * empty effective body (for THIS project) would run as a silent no-op, so
 * those stay hidden — matching the palette's "no text yet" gate. Exported so
 * SessionChat can reserve the drawer strip's space only when chips will show.
 */
export function usePinnedPrompts(projectPath: string): CustomPrompt[] {
  const settings = useBootstrap().data?.settings;
  return useMemo(
    () =>
      customPrompts(settings).filter(
        (p) => p.pinned && effectiveCustomBody(p, projectPath).trim(),
      ),
    [settings, projectPath],
  );
}

/**
 * Pinned custom prompts as one-click chips. SessionChat anchors this to the
 * composer's top edge (`absolute bottom-full right-*`) so the chips share the
 * strip the task drawer's handle occupies on the left — one line, chips right,
 * TASKS tab left — instead of stacking their own row above it. No edit
 * affordance here on purpose: the library is already reachable three ways
 * (nav rail ⚡, agent-bar ⚡, the ⌘K palette's manage row). Renders nothing
 * when no usable prompt is pinned.
 */
export function PromptChips({
  view,
  shellId,
  className,
}: {
  view: WorkstreamView;
  shellId: number;
  /** Positioning classes from the host (e.g. the composer-anchored absolute). */
  className?: string;
}) {
  const pinned = usePinnedPrompts(view.item.projectPath);
  if (pinned.length === 0) return null;

  return (
    <div
      className={`flex flex-wrap items-center justify-end gap-1.5 ${className ?? ""}`}
    >
      {pinned.map((p) => (
        <button
          key={p.id}
          type="button"
          onClick={() => void runCustomPrompt(view, p, shellId)}
          title="Runs immediately in this session"
          className="inline-flex items-center gap-1.5 rounded-md border border-edge bg-panel-2 px-2 py-1 text-xs text-ink hover:border-accent/50 hover:bg-accent/10"
        >
          <span className="shrink-0 text-accent">
            <SparkleIcon size={12} />
          </span>
          <span className="max-w-44 truncate">{p.name || "Prompt"}</span>
          {p.chord && (
            <ChordHint
              chord={p.chord}
              className="rounded bg-edge/60 px-1 py-0.5 text-[11px] text-ink-dim"
            />
          )}
        </button>
      ))}
    </div>
  );
}
