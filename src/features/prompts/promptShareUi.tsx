
import { Icon } from "@/components/Icon";

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
      <Icon name="check" size="xs" />
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
  if (builtin && id === "commit") return <Icon name="commit" size="sm" className={className} />;
  if (builtin && id === "ship") return <Icon name="pullRequest" size="sm" className={className} />;
  return <Icon name="prompt" size="sm" className={className} />;
}
