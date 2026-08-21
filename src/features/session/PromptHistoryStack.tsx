/**
 * A subtle stacked-lines graphic in the top-right corner of the agent pane that
 * visualises the current session's prompt history. One thin line per prompt,
 * oldest at top and newest at bottom (within the visible window). Hovering a
 * line previews the prompt text; clicking recalls it into the composer. The
 * line for the currently parked-on entry (via keyboard or click) is highlighted
 * with the accent colour.
 *
 * It floats over the transcript's empty right gutter (the pane is the positioned
 * ancestor); `pointer-events-none` on the container with `pointer-events-auto`
 * on the lines lets clicks fall through the gaps to the transcript underneath.
 *
 * Purely presentational, holds no history state. Parent supplies `prompts`,
 * `activeIndex`, and `onRecall`.
 */
import { useState } from "react";
import { stackWindow } from "./promptHistory";

const MAX_VISIBLE = 8;

export function PromptHistoryStack({
  prompts,
  activeIndex,
  onRecall,
}: {
  prompts: string[];
  activeIndex: number | null;
  onRecall: (index: number) => void;
}) {
  const [hovered, setHovered] = useState<number | null>(null);

  if (prompts.length === 0) return null;

  const { start, end } = stackWindow(prompts.length, activeIndex, MAX_VISIBLE);
  const hasMore = start > 0;

  return (
    <div
      className="pointer-events-none absolute right-4 top-3 z-10 flex flex-col items-end gap-[3px] opacity-60 transition-opacity hover:opacity-100"
    >
      {hasMore && (
        <span className="pointer-events-none select-none text-[9px] leading-none text-ink-dim/40">
          ···
        </span>
      )}
      {Array.from({ length: end - start }, (_, i) => {
        const idx = start + i;
        const isActive = idx === activeIndex;
        const isNewest = idx === prompts.length - 1;
        const isHovered = hovered === idx;

        return (
          <div key={idx} className="relative">
            {isHovered && (
              <div className="pointer-events-none absolute right-full top-0 mr-2 w-72 max-h-40 overflow-hidden rounded-lg border border-edge bg-panel p-2 text-xs text-ink shadow-xl whitespace-pre-wrap break-words">
                {prompts[idx]}
              </div>
            )}
            <button
              type="button"
              className="pointer-events-auto flex items-center py-1"
              aria-label={`Recall prompt ${idx + 1} of ${prompts.length}`}
              onMouseEnter={() => setHovered(idx)}
              onMouseLeave={() => setHovered(null)}
              onClick={() => onRecall(idx)}
            >
              <span
                className={`h-[2px] w-5 rounded-full ${
                  isActive
                    ? "bg-accent"
                    : isNewest
                      ? "bg-ink-dim/60"
                      : "bg-ink-dim/30"
                }`}
              />
            </button>
          </div>
        );
      })}
    </div>
  );
}
