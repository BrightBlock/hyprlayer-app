import { cx } from "@/components/ui";

/** An error's remedy; a `quoted` command selects whole in one click, for pasting into a terminal. */
export function Remedy({ text, className }: { text: string; className?: string }) {
  return (
    <p className={cx("selectable break-words text-body text-ink-dim", className)}>
      {text.split(/(`[^`\n]+`)/g).map((part, i) =>
        part.length > 2 && part.startsWith("`") && part.endsWith("`") ? (
          <code key={i} className="select-all break-all font-mono text-ink">
            {part.slice(1, -1)}
          </code>
        ) : (
          part
        ),
      )}
    </p>
  );
}
