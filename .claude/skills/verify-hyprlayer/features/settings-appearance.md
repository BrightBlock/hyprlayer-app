# Appearance: theme, zoom, text size

Settings → Appearance lets the user pick a theme from swatch tiles. It also sets the app zoom, the text size, the interface and code fonts, and the terminal font size. The popover's Light/Dark/Auto control sets the same theme. All of it applies at once and is stored per device in webview localStorage, not in `settings.json`.

## Sub-features

- `app-theme`: a theme tile sets `html`'s theme class (`light`, `dark`, `claude`, `cobalt`, `matrix`, `solarized`). `System` follows the OS.
- `app-theme-popover`: the popover's `Appearance` radiogroup, where Light, Dark and Auto map to `light`, `dark` and `system`.
- `app-zoom`: steps of 1.2× from 100% up to 1.2⁸, plus Reset. Applied through the webview's `setZoom`.
- `app-text-size`: Small, Default, Large or Larger sets `--text-scale` on `html`.
- `app-fonts`: the interface and code font dropdowns set `--font-sans` and `--font-mono`.
- `app-terminal-font`: a stepper from 8 to 32 px, default 12.

## How to get to it (user POV)

- Settings popover, then the `Appearance` row: Light, Dark or Auto.
- Settings popover, then `All settings`, then `Appearance` in the nav. It also works while disconnected.

## Driving it with the MCP bridge

Preconditions:

- Any doctor-passed instance. Appearance renders without a server connection or a seed. <!-- src: src/features/settings/SettingsView.tsx:56 -->
- Before changing anything, save `localStorage.getItem('hyprlayer:theme')`, `'hyprlayer:appearance'` and `'hyprlayer.terminalFontSize'` to the evidence dir. Restore them at the end through the UI: they outlive cleanup in isolated mode too (see Gotchas). <!-- src: src/lib/theme.ts:94 --> <!-- src: src/lib/appearance.ts:92 -->

- **Open.** Click `All settings` in the popover, then `Appearance` in `nav[aria-label="Settings sections"]`. `location.search` is `?section=appearance`. <!-- src: src/features/settings/SettingsNav.tsx:21 -->
- **Theme tile.** Click the button with text `Cobalt` (`strategy: "text"`). It gets `aria-pressed="true"`. `document.documentElement.classList` contains `cobalt` and no other theme class, and `JSON.parse(localStorage.getItem('hyprlayer:theme')).state.mode === "cobalt"`. <!-- src: src/features/settings/AppearanceSettings.tsx:143 --> <!-- src: src/features/settings/AppearanceSettings.tsx:144 --> <!-- src: src/lib/theme.ts:142 --> <!-- src: src/lib/theme.ts:145 -->
- **Popover segment.** Open the Settings popover and click `Light` inside `[role="radiogroup"][aria-label="Appearance"]`. That `[role="radio"]` gets `aria-checked="true"`, `html` has class `light`, and `mode` is `light`. While Cobalt was active, none of the three radios is checked. <!-- src: src/features/settings/SettingsPopover.tsx:115 --> <!-- src: src/lib/useRovingRadio.ts:46 --> <!-- src: src/lib/theme.ts:52 -->
- **Terminal follows.** If a terminal is open, screenshot it after switching to Light. It must turn light too, which is the proof that the store drove it.
- **Text size.** In `[role="radiogroup"][aria-label="Text size"]`, click `Large`. `document.documentElement.style.getPropertyValue('--text-scale') === "1.12"`, and `hyprlayer:appearance` has `textSize: "large"`. Click `Default` to restore. <!-- src: src/features/settings/AppearanceSettings.tsx:186 --> <!-- src: src/lib/appearance.ts:32 --> <!-- src: src/lib/appearance.ts:136 -->
- **Zoom.** Click `button[aria-label="Increase zoom"]`. The display reads `120%`, `hyprlayer:appearance` has `zoom` ≈ `1.2`, and the page re-lays out (a `resize` fires after `setZoom`). Click `Reset`, the first `Reset` in the Display card, before taking proof shots. `Decrease zoom` is disabled at 100%. <!-- src: src/features/settings/AppearanceSettings.tsx:88 --> <!-- src: src/features/settings/AppearanceSettings.tsx:169 --> <!-- src: src/lib/appearance.ts:17 --> <!-- src: src/lib/appearance.ts:120 -->
- **Fonts.** Click `button[aria-haspopup="listbox"][aria-label="Code font"]`, then the option `Fira Code`. `--font-mono` starts with `"Fira Code Variable"`. Restore `JetBrains Mono`. <!-- src: src/features/settings/AppearanceSettings.tsx:214 --> <!-- src: src/lib/appearance.ts:53 --> <!-- src: src/lib/appearance.ts:138 -->
- **Terminal font.** Click `button[aria-label="Increase terminal font size"]`. The display reads `13px`, and `localStorage.getItem('hyprlayer.terminalFontSize')` changes. Reset it. <!-- src: src/features/settings/AppearanceSettings.tsx:231 --> <!-- src: src/lib/terminal/terminalFontSize.ts:6 -->
- **Proof.** Take a screenshot per theme or size change, plus the localStorage JSON and `classList` before and after.

## Gotchas

- Never toggle `html.light` by hand. Terminals read the theme store, so they stay dark. Drive the tiles or radios, or as a seed only, the live store's `useThemeStore.getState().setMode('<mode>')` loaded through the page's module URL (SKILL.md, Drive step 5). <!-- src: src/lib/theme.ts:90 -->
- Appearance is localStorage, per Vite origin. An isolated run on a reused port inherits an earlier run's theme and zoom, and cleanup doesn't reset it. Restore everything you changed.
- `Dark` in the popover is the theme the tiles call `Dark+`. Named themes leave the popover's segment empty. <!-- src: src/lib/theme.ts:33 -->
- `System` resolves through `prefers-color-scheme`. With the OS in dark mode it adds `dark`, not `system`. <!-- src: src/lib/theme.ts:57 -->
- Zoom rescales the webview, so the bridge's coordinates and screenshot size change. `⌘=`, `⌘-` and `⌘0` drive the same store. <!-- src: src/lib/appearance.ts:161 -->
- The Omarchy tile shows only on Linux with Omarchy detected. On a Mac, expect seven tiles. <!-- src: src/features/settings/AppearanceSettings.tsx:107 -->
