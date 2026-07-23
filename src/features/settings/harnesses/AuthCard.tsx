/** One selectable authentication method tile. Shared by the harness tabs so
 *  "sign in with your subscription" vs "use an API key" reads identically
 *  whichever harness you're configuring. */
export function AuthCard({
  label,
  sublabel,
  selected,
  onSelect,
}: {
  label: string;
  sublabel: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={`relative rounded-lg border p-4 text-left transition-colors ${
        selected ? "border-accent bg-panel-2" : "border-edge hover:bg-edge/40"
      }`}
    >
      {selected && (
        <span className="absolute right-3 top-3 text-accent" aria-hidden>
          ✓
        </span>
      )}
      <p className="text-sm font-medium text-ink">{label}</p>
      <p className="mt-0.5 text-xs text-ink-dim">{sublabel}</p>
    </button>
  );
}

/** The bordered key/value table both tabs use to show who's signed in. */
export function AuthDetailRows({ rows }: { rows: Array<[string, string | null | undefined]> }) {
  return (
    <div className="overflow-hidden rounded-md border border-edge">
      {rows.map(([k, v], i) => (
        <div
          key={k}
          className={`flex items-center justify-between px-3 py-2 text-sm ${
            i > 0 ? "border-t border-edge/60" : ""
          }`}
        >
          <span className="text-ink-dim">{k}</span>
          <span className="selectable text-ink">{v ?? "—"}</span>
        </div>
      ))}
    </div>
  );
}
