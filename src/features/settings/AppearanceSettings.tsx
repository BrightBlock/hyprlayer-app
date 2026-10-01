import type { OmarchyTheme } from "@/lib/bindings";
import { SETTINGS_CARD_DESCRIPTION, SettingsCardTitle } from "@/components/SettingsCard";
import {
  Button,
  Card,
  cx,
  DropdownSelect,
  IconButton,
  SegmentedControl,
  SettingRow,
} from "@/components/ui";
import { Icon } from "@/components/Icon";
import {
  MONO_FONTS,
  SANS_FONTS,
  TEXT_SIZES,
  useAppearance,
  ZOOM_DEFAULT,
  ZOOM_MAX,
  ZOOM_MIN,
} from "@/lib/appearance";
import {
  TERMINAL_FONT_DEFAULT,
  TERMINAL_FONT_MAX,
  TERMINAL_FONT_MIN,
  useTerminalFontSize,
} from "@/lib/terminal/terminalFontSize";
import { THEMES, useThemeStore } from "@/lib/theme";

const SWATCHES: Record<string, [string, string, string]> = {
  system: ["#111113", "#222226", "#ececee"],
  dark: ["#111113", "#222226", "#ececee"],
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

function Stepper({
  label,
  display,
  canDecrease,
  canIncrease,
  isDefault,
  onDecrease,
  onIncrease,
  onReset,
}: {
  label: string;
  display: string;
  canDecrease: boolean;
  canIncrease: boolean;
  isDefault: boolean;
  onDecrease: () => void;
  onIncrease: () => void;
  onReset: () => void;
}) {
  return (
    <div className="flex items-center gap-1">
      <IconButton
        className="h-6 w-6"
        aria-label={`Decrease ${label}`}
        disabled={!canDecrease}
        onClick={onDecrease}
      >
        <Icon name="decrease" size="sm" />
      </IconButton>
      <span className="w-12 text-center text-ui tabular-nums text-ink">{display}</span>
      <IconButton
        className="h-6 w-6"
        aria-label={`Increase ${label}`}
        disabled={!canIncrease}
        onClick={onIncrease}
      >
        <Icon name="increase" size="sm" />
      </IconButton>
      <Button size="sm" className="ml-2" disabled={isDefault} onClick={onReset}>
        Reset
      </Button>
    </div>
  );
}

export function AppearanceSettings() {
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);
  const omarchy = useThemeStore((s) => s.omarchy);

  const showOmarchy = omarchy !== null || mode === "omarchy";
  const themes = showOmarchy ? THEMES : THEMES.filter((t) => t.id !== "omarchy");
  const omarchyColors = omarchy ? omarchySwatch(omarchy) : undefined;

  const zoom = useAppearance((s) => s.zoom);
  const zoomIn = useAppearance((s) => s.zoomIn);
  const zoomOut = useAppearance((s) => s.zoomOut);
  const resetZoom = useAppearance((s) => s.resetZoom);
  const textSize = useAppearance((s) => s.textSize);
  const setTextSize = useAppearance((s) => s.setTextSize);
  const sansFont = useAppearance((s) => s.sansFont);
  const setSansFont = useAppearance((s) => s.setSansFont);
  const monoFont = useAppearance((s) => s.monoFont);
  const setMonoFont = useAppearance((s) => s.setMonoFont);
  const terminalFontSize = useTerminalFontSize((s) => s.fontSize);
  const terminalZoomIn = useTerminalFontSize((s) => s.zoomIn);
  const terminalZoomOut = useTerminalFontSize((s) => s.zoomOut);
  const resetTerminalFontSize = useTerminalFontSize((s) => s.reset);

  return (
    <>
      <Card>
        <SettingsCardTitle>Appearance</SettingsCardTitle>
        <p className={cx("mt-1", SETTINGS_CARD_DESCRIPTION)}>
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
                className={`flex items-center gap-3 rounded-md border px-3 py-2 text-left text-body transition-colors ${
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

      <Card>
        <SettingsCardTitle className="mb-0">Display</SettingsCardTitle>
        <div className="divide-y divide-edge/50">
          <SettingRow
            title="Zoom"
            control={
              <Stepper
                label="zoom"
                display={`${Math.round(zoom * 100)}%`}
                canDecrease={zoom > ZOOM_MIN}
                canIncrease={zoom < ZOOM_MAX}
                isDefault={zoom === ZOOM_DEFAULT}
                onDecrease={zoomOut}
                onIncrease={zoomIn}
                onReset={resetZoom}
              />
            }
          >
            Scales everything in the window on this computer.
          </SettingRow>
          <SettingRow
            title="Text size"
            control={
              <SegmentedControl
                aria-label="Text size"
                value={textSize}
                onChange={setTextSize}
                options={TEXT_SIZES.map((t) => ({ value: t.id, label: t.label }))}
              />
            }
          >
            Text only; spacing and the terminal keep their size.
          </SettingRow>
          <SettingRow
            title="Interface font"
            control={
              <DropdownSelect
                aria-label="Interface font"
                className="min-w-44"
                value={sansFont}
                onChange={(v) => {
                  const f = SANS_FONTS.find((x) => x.id === v);
                  if (f) setSansFont(f.id);
                }}
                options={SANS_FONTS.map((f) => ({ value: f.id, label: f.label }))}
              />
            }
          />
          <SettingRow
            title="Code font"
            control={
              <DropdownSelect
                aria-label="Code font"
                className="min-w-44"
                value={monoFont}
                onChange={(v) => {
                  const f = MONO_FONTS.find((x) => x.id === v);
                  if (f) setMonoFont(f.id);
                }}
                options={MONO_FONTS.map((f) => ({ value: f.id, label: f.label }))}
              />
            }
          >
            Code, the terminal, and metadata labels.
          </SettingRow>
          <SettingRow
            title="Terminal font size"
            control={
              <Stepper
                label="terminal font size"
                display={`${terminalFontSize}px`}
                canDecrease={terminalFontSize > TERMINAL_FONT_MIN}
                canIncrease={terminalFontSize < TERMINAL_FONT_MAX}
                isDefault={terminalFontSize === TERMINAL_FONT_DEFAULT}
                onDecrease={terminalZoomOut}
                onIncrease={terminalZoomIn}
                onReset={resetTerminalFontSize}
              />
            }
          >
            Separate from zoom and text size.
          </SettingRow>
        </div>
      </Card>
    </>
  );
}
