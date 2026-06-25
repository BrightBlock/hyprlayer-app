/** Ahead/behind of the base branch — the `↑2 ↓0` chip. Rendered in the GIT
 *  panel header and mirrored into the app header's right cluster so divergence
 *  stays visible when the changes sidebar is collapsed. Both legs always render;
 *  the live one takes an accent, a zero leg stays dim. */
export function AheadBehind({
  ahead,
  behind,
  className = "",
}: {
  ahead: number;
  behind: number;
  className?: string;
}) {
  return (
    <span className={`flex items-center gap-2 font-mono text-xs tabular-nums ${className}`}>
      <span
        className={ahead > 0 ? "text-accent" : "text-ink-dim/60"}
        title={`${ahead} commit(s) ahead of base`}
      >
        ↑{ahead}
      </span>
      <span
        className={behind > 0 ? "text-warn" : "text-ink-dim/60"}
        title={`${behind} commit(s) behind base`}
      >
        ↓{behind}
      </span>
    </span>
  );
}
