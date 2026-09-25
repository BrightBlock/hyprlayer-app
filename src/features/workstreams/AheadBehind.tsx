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
    <span className={`flex items-center gap-2 text-meta tabular-nums ${className}`}>
      <span
        className={ahead > 0 ? "text-accent" : "text-ink-dim/60"}
        title={`${ahead} commit(s) ahead of base`}
      >
        ↑{ahead}
      </span>
      <span
        className={behind > 0 ? "text-warn-ink" : "text-ink-dim/60"}
        title={`${behind} commit(s) behind base`}
      >
        ↓{behind}
      </span>
    </span>
  );
}
