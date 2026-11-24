import { Icon } from "@/components/Icon";

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
      className={`relative rounded-lg border p-4 text-left ${
        selected ? "border-accent bg-panel-2" : "border-edge hover:bg-edge/40"
      }`}
    >
      {selected && (
        <Icon name="check" size="sm" className="absolute right-3 top-3 text-accent" />
      )}
      <p className="text-body font-medium text-ink">{label}</p>
      <p className="mt-0.5 text-ui text-ink-dim">{sublabel}</p>
    </button>
  );
}

export function AuthDetailRows({ rows }: { rows: Array<[string, string | null | undefined]> }) {
  return (
    <div className="overflow-hidden rounded-md border border-edge">
      {rows.map(([k, v], i) => (
        <div
          key={k}
          className={`flex items-center justify-between px-3 py-2 text-body ${
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
