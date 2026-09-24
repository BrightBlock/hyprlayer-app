import type { OmarchyTheme } from "@/lib/bindings";
import { Card, CardTitle } from "@/components/ui";
import { THEMES, useThemeStore } from "@/lib/theme";

const SWATCHES: Record<string, [string, string, string]> = {
  system: ["#0c0c0c", "#1c1c1c", "#d97757"],
  dark: ["#0c0c0c", "#1c1c1c", "#d97757"],
  light: ["#f7f7f7", "#f5f5f5", "#111111"],
  claude: ["#1f1e1d", "#2a2926", "#d97757"],
  cobalt: ["#193549", "#234a6a", "#66c4ff"],
  matrix: ["#050905", "#161e16", "#3fb950"],
  solarized: ["#002b36", "#0a4756", "#cb4b16"],
};

function ThemeSwatch({ id, colors }: { id: string; colors?: [string, string, string] }) {
  const [surface, panel, accent] = colors ?? SWATCHES[id] ?? SWATCHES.dark;
  return (
    <span className="flex shrink-0 overflow-hidden rounded border border-edge">
      <span className="h-5 w-5" style={{ background: surface }} />
      <span className="h-5 w-5" style={{ background: panel }} />
      <span className="h-5 w-5" style={{ background: accent }} />
    </span>
  );
}

function omarchySwatch(theme: OmarchyTheme): [string, string, string] {
  const token = (name: string) => theme.tokens.find((t) => t.name === name)?.value ?? "";
  return [token("--color-surface"), token("--color-panel-2"), token("--color-accent")];
}

export function AppearanceSettings() {
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);
  const omarchy = useThemeStore((s) => s.omarchy);

  const showOmarchy = omarchy !== null || mode === "omarchy";
  const themes = showOmarchy ? THEMES : THEMES.filter((t) => t.id !== "omarchy");
  const omarchyColors = omarchy ? omarchySwatch(omarchy) : undefined;

  return (
    <Card>
      <CardTitle>Appearance</CardTitle>
      <p className="mt-1 text-xs text-ink-dim">
        “System” follows your OS light/dark setting.
        {showOmarchy &&
          " “Omarchy” follows your Linux desktop's active theme and updates automatically when it changes."}
      </p>
      <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
        {themes.map((theme) => {
          const active = mode === theme.id;
          const isOmarchy = theme.id === "omarchy";
          const suffix = isOmarchy ? (omarchy ? omarchy.name || null : "not detected") : null;
          return (
            <button
              key={theme.id}
              type="button"
              onClick={() => setMode(theme.id)}
              aria-pressed={active}
              className={`flex items-center gap-3 rounded-md border px-3 py-2 text-left text-sm transition-colors ${
                active
                  ? "border-accent bg-panel-2 text-ink"
                  : "border-edge text-ink-dim hover:bg-edge/60 hover:text-ink"
              }`}
            >
              <ThemeSwatch id={theme.id} colors={isOmarchy ? omarchyColors : undefined} />
              <span className="truncate">
                {theme.label}
                {suffix && <span className="text-ink-dim"> {suffix}</span>}
              </span>
            </button>
          );
        })}
      </div>
    </Card>
  );
}
