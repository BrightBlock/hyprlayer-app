
import { CheckIcon, GitCommitIcon, GitPullRequestIcon, ZapIcon } from "@/components/icons";

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
