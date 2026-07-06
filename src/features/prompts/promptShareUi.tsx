/**
 * Shared chrome for the prompt import/export dialogs: the modal shell (overlay,
 * Escape/click-outside close, header/body/footer bands), the little checkbox,
 * and the per-row prompt glyph. Kept together so the two dialogs stay visually
 * identical without cross-importing each other.
 */

import type { ReactNode } from "react";

import { CheckIcon, GitCommitIcon, GitPullRequestIcon, XIcon, ZapIcon } from "@/components/icons";
import { useEscape } from "@/lib/keybindings/registry";

/** Full-screen modal shell matching the app's `Dialog` overlay, but with a
 *  flex column so callers can lay out their own header/scroll-body/footer
 *  bands (the shared `Dialog`'s fixed padding can't do full-bleed seams). */
export function ShareModal({
  onClose,
  title,
  icon,
  width = "max-w-lg",
  children,
}: {
  onClose: () => void;
  title: string;
  icon: ReactNode;
  width?: string;
  children: ReactNode;
}) {
  useEscape(onClose);
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-surface/70 p-4 pt-[6vh] backdrop-blur-sm"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={`flex max-h-[88vh] w-full ${width} flex-col overflow-hidden rounded-xl border border-edge bg-panel shadow-2xl`}
      >
        <div className="flex items-center gap-2.5 border-b border-edge px-4 py-3">
          <span className="shrink-0 text-accent">{icon}</span>
          <h2 className="text-sm font-semibold text-ink">{title}</h2>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="ml-auto flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
          >
            <XIcon size={13} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

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
