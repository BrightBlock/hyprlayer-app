/**
 * Shared chrome for the prompt import/export dialogs: the little checkbox and
 * the per-row prompt glyph. Kept together so the two dialogs stay visually
 * identical without cross-importing each other. The modal shell itself is the
 * shared `Dialog` (`size="lg"`/`"xl"`, `align="top"`, `seams`) — it used to be
 * a fork here because `Dialog`'s fixed padding couldn't do full-bleed seams,
 * but Dialog grew bands (header/body/footer, each with its own inset) that
 * make the fork unnecessary.
 */

import { CheckIcon, GitCommitIcon, GitPullRequestIcon, ZapIcon } from "@/components/icons";

/** A square checkbox toggled by clicking. Accent-filled + check when on. */
export function ShareCheck({
  checked,
  onToggle,
  label,
}: {
  checked: boolean;
  onToggle: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-label={label}
      onClick={onToggle}
      className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors ${
        checked ? "border-accent bg-accent text-accent-ink" : "border-edge bg-panel-2 text-transparent"
      }`}
    >
      <CheckIcon size={10} />
    </button>
  );
}

/** The glyph for a prompt row: a built-in's own icon, else the custom bolt. */
export function PromptRowIcon({
  builtin,
  id,
  className,
}: {
  builtin: boolean;
  id: string;
  className?: string;
}) {
  if (builtin && id === "commit") return <GitCommitIcon size={14} className={className} />;
  if (builtin && id === "ship") return <GitPullRequestIcon size={14} className={className} />;
  return <ZapIcon size={14} className={className} />;
}
