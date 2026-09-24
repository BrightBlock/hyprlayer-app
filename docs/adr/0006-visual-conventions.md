# ADR 0006: Visual conventions — row tiers, hover, dividers, dialogs

**Status:** accepted · 2026-08-21 · amended 2026-09-24 (v4 light palette and radii, native-checkbox
`Toggle`, `SegmentedControl`, `Popover` focus trap and caret, the status bar and Settings sizes)

## Decision

The app had never written down its row heights, hover values, dialog widths, or
icon-button sizes. The 2026-07-05 IA review called the result "undocumented
folklore." The visual-polish-v1 pass (this ADR's companion implementation)
fixed a real bug in the process — `cx()` at `src/components/ui.tsx:24-26` was a
plain `filter().join(" ")`, so a caller's `className="max-w-2xl"` lost to
`Dialog`'s own `max-w-md` base whenever Tailwind's stylesheet order favoured
the base, which is how five dialogs (three of them CLI sign-in flows embedding
an `h-80` terminal) shipped at 448px regardless of what their author wrote.
This records the vocabulary that came out of fixing it.

### `cx()` merges

`cx()` (`ui.tsx:24-26`) now delegates to `twMerge`, so a later class wins over
an earlier one in the same utility group — the same resolution Tailwind's own
cascade can't give you across a base class and a `className` override.
Reproduced with `npx vite build && grep -bo '\.max-w-md{' dist/assets/index-*.css`:
`max-w-2xl` sits at byte 15152, `max-w-3xl` at 15194, `max-w-lg` at 15919, and
`max-w-md` — `Dialog`'s base — at 15959, after all three; `mb-0` sits at 11152
and `CardTitle`'s base `mb-3` at 11436, so all ten `CardTitle className="mb-0"`
overrides (e.g. `Settings.tsx:632`, `:718`, `:781`) were dead until this
landed. Both defects are receipts for the same root cause, not two bugs.

The one workaround this obsoletes: `DropdownSelect` used to carry a
`hasWidthClass` regex to suppress its own `w-full` when a caller passed a
width. `cx()` merging correctly makes the regex redundant; it's gone, and
`w-full` is unconditional again.

### No NUL bytes in source

`ui.tsx` briefly carried a literal NUL in a React key (`DropdownSelect`'s
option key), which put ripgrep's binary-file detection between every
directory-wide search and the app's most important design file — a `rg
'pattern' src/components/` would exit 1 with no output, silently. The key now
joins group and value with U+001F (unit separator, `ui.tsx:361-364`) — same
"can't occur in a group name or option value" guarantee, without tripping
binary detection. If a composite key ever needs a separator again, use
U+001F, never `\0`.

### Row tiers

Two heights, not three: `compact` (`h-7`, 28px) and `standard` (`h-9`, 36px),
via `rowClass()` (`ui.tsx:864-883`). An earlier draft of the
implementation plan proposed a third, 48px tier for the Agents view's agent
row on the assumption it held two lines of content. It doesn't: `AgentList.tsx`'s
agent row (`:530-548`; `AgentList.tsx` no longer exists, deleted by the
sidebar-shell plan — see "Agents: sidebar shell" below) was one `text-sm`
line box (20px) inside what was a 52px (`h-13`) box — the extra 32px of
whitespace was never doing anything. Both Agents-view row kinds (folder,
`:270-273`; agent, `:544-548`) built on `rowClass({ tier: "standard", ... })`
at the time; the sidebar that replaced the table uses a different row idiom
(see "Agents: sidebar shell" below), and the app's other 36px rows (the
sidebar workstream row, `Root.tsx:2009-2019`; the two command palettes;
`PromptLibrary`'s sidebar; `Archive`) already used that height before the
row primitive existed. Resist adding a third tier for a row that merely
*feels* crowded — check what it actually holds first, the way this one didn't.

Left unmigrated, by design, not oversight:

- **The 32px row band** — `Root.tsx`'s pinned-workstream row (`:1824-1838`),
  `LibrarySidebar.tsx`'s theme/profile switcher rows, and
  `ComposerAutocomplete.tsx`'s autocomplete row (`:396-398`). A third row tier
  would defeat the two-tier point; these stay hand-rolled.
- **The multi-line rows** — `WorkstreamDocsPanel`'s doc row (~63px),
  `LibrarySidebar`'s doc row (~73px), `ArchivedAgentsView`'s row (~70px).
  None of these are single-line content, so the 28/36px vocabulary doesn't
  apply to them.
- **`CommitsPanel`'s 20px indent step** (`pl-[26px]`, e.g. `:41`) — see
  "Tree indent" below.
- **`Root.tsx`'s hand-tuned `pl-10`** (`style={{ paddingLeft: 40 }}` at
  `:2020`) — see "Tree indent" below.
- **Three of the four `RowSpine`-shaped literals.** `RowSpine` (`ui.tsx:1059-1061`)
  replaced the inline `<span className="absolute inset-y-0 left-0 w-[3px]
  bg-accent">` only at its one canonical call site, `Root.tsx:2035`. The
  identical literal still stands, un-migrated, at `Settings.tsx:1138`,
  `Root.tsx:1901` (the pinned row, deliberately off the row tiers per above),
  and `LibrarySidebar.tsx:146`. Recorded here rather than silently claiming
  the spine is a single mechanism everywhere it appears.

### Hover and selection

Two neutral hover values, split by *what kind of surface is hovering*, not by
which one happened to be more common:

- **`hover:bg-edge/50`** — list rows. A row tints toward the divider colour.
  Baked into `rowClass()`'s `ROW_HOVER` constant (`ui.tsx:1019`), so every row
  built on it gets this for free. `AgentList`'s agent row used to hover
  `bg-panel-2` — the menu tint, on a row — before this landed; it didn't
  anymore, and `AgentList.tsx` itself is gone entirely since (see "Agents:
  sidebar shell" below) — its replacement uses the Prompts-anatomy row idiom,
  not `rowClass`.
- **`hover:bg-panel-2`** — menu items and chrome. The surface raises to the
  next level up. `ContextMenuItem` (`:871`), `IconButton` (`:749`),
  `DropdownSelect`'s options (`:385`) all use it.

`selected: "tint"` is `bg-accent/15`; `selected: "spine"` is the same tint plus
a `<RowSpine />` accent bar at the row's left edge, requiring `position:
relative` on the row (`rowClass` always emits it). The sidebar's active
workstream row and the Agents view's selected agent row both land on
`bg-accent/15` — previously `bg-accent/10` on the agent row, a small,
deliberate value change to bring the two in line.

**A third, deliberately uncollapsed hover value survives: `hover:bg-edge/60`,
for the "inline tier" of small (`h-6`/`h-7`/`h-8`) icon-only controls that sit
*inside* another surface whose own resting background is `bg-panel-2`.**
`hover:bg-panel-2` on one of these would be an invisible no-op against a
`bg-panel-2` backdrop, so they keep the divider-tinted value instead. This is
not an oversight in the two-value split above — neither the row rule nor the
menu/chrome rule claims this tier, and it was named explicitly as an exception
rather than swept, because sweeping it would mean auditing every such
control's actual resting surface one at a time for a value nobody reported as
wrong. Verified examples: `Dialog`'s close button (`ui.tsx:593`, on the
panel's `bg-panel` surface but styled as inline chrome), `RefreshButton`'s
icon-only branch (`:787`), `ScriptDock.tsx`'s Run/Configure button-group
controls (`:302`, `:476`, `:495`, `:555`, `:566`, all resting on `bg-panel-2`),
and `DocDetail.tsx`'s two header icon buttons (`:152`, `:168`). A sibling,
unnamed case — `hover:bg-panel` on `AgentEditor.tsx:997`'s collaborator-remove
button — was left alone for the identical reason and is recorded here rather
than pretending the app converged on exactly two values everywhere.

Semantic tints (`hover:bg-bad/10`, `hover:bg-warn/10`, `hover:bg-warn/20`,
`hover:bg-ship/25`, `hover:bg-revise/20`, `hover:bg-accent/*`,
`hover:bg-transparent`) are out of scope entirely — they encode meaning
(danger, ship-state, an active selection), not a neutral surface step, and
collapsing them into the row/menu split would erase that meaning.
`ErrorNote`'s dismiss button (`ui.tsx:699`, `hover:bg-bad/15` against its own
`bg-bad/10` note) is one of these, not an instance of the third inline-tier
exception above, however similar its `h-6 w-6 rounded` shape looks.

### Dividers

Container-level `divide-y divide-edge/60`, never a per-row `border-b`.
`Archive.tsx:191`, `WorkstreamDocsPanel.tsx:104`, and `ArchivedAgentsView.tsx:145`
all use it (a fourth example, `AgentList.tsx:252`, is gone along with the rest
of that file — see "Agents: sidebar shell" below; its replacement is
gap-based, `space-y-0.5`/`space-y-4`, the dividerless camp named below, not
this one). The reason: a per-row border needs a
`last:border-b-0` or an index guard to avoid a trailing rule after the final
row, and both hacks existed independently in the tree before this — a
container-level divider can't produce a trailing rule in the first place, so
there's nothing to guard.

"One mechanism" means *when a list wants dividers, use this one* — lists that
use gaps instead of rules (`Root`'s workstream tree, `ChangesPanel`,
`CommitsPanel`, `PromptLibrary`, the transcript) stay dividerless; that's a
different, and unrelated, layout choice.

### Sections inside a dialog

A dialog long enough to need sections gets them from
`SettingsSection` / `SectionHeading`
(`src/features/projects/settingsSection.tsx`), not from `Card`/`CardTitle`:

- The heading is `text-2xs font-semibold uppercase tracking-wider text-ink-dim`
  — the same treatment the Agents view used at the time on its table's column
  header (`AgentList.tsx:256`, since deleted — see "Agents: sidebar shell"
  below; its replacement carries the identical class string on the drilled-in
  sidebar's section label, `AgentSidebar.tsx:294`). It is
  a size step *below* the body copy it introduces, which is the point: a
  heading that matches its own prose at `text-sm font-medium` marks nothing.
- The boundary is a `border-t border-edge/60` rule run full-bleed against the
  dialog body's `p-4` inset (`-mx-4 px-4`), so it reads as structure rather
  than as an underline of the heading.
- The gap above and below that rule (`mt-5 pt-5`) is deliberately double the
  spacing used *inside* a section. The contrast is the grouping signal.

Recorded because the obvious first fix is the wrong one. `ProjectSettingsDialog`
originally sectioned with `Card`/`CardTitle`; dropping those was correct — a
card inside a dialog stacks two borders — but nothing replaced the separation
they had incidentally been carrying, and the dialog shipped as one flat wall of
text with four same-size headings 12px apart. Removing a container is not the
same as removing the need for the boundary it drew.

### Tree indent

`treeIndent(depth, base = 8)` (`src/lib/layout.ts:101-103`) returns
`base + depth * 14`. The 14px step was already what three of the app's trees
used independently; `base` stays a caller argument rather than a constant
because it encodes a real layout fact per tree — the width of whatever chrome
(a checkbox, a disclosure chevron) the row's text has to clear. Call sites:
`ChangesPanel.tsx:88`, `:201` (`base 6`), `:409`, `:433`, `:462` (`base 8`),
and the Agents sidebar's tree via `agents.ts:491`, `:506` (`base 0`) — that
caller was `AgentList.tsx` until the sidebar-shell plan replaced it with
`AgentSidebar.tsx` (see "Agents: sidebar shell" below); `agentRows()`'s own
`treeIndent` calls, and their line numbers, are unchanged.

Two trees deliberately stay off the helper:

- **`CommitsPanel`** — `paddingLeft: 6` at its top level (`:33`) and
  `pl-[26px]` for children (`:41`, `:48`, `:52`, `:56`, `:75`), a 20px step.
  `treeIndent(depth, 6)` can only produce 6 and 20 — never 26 — so expressing
  this as `treeIndent` would silently shift the child rows 6px left. It never
  used the 14px step to begin with.
- **`Root.tsx`'s workstream row** — the literal `paddingLeft: 40` (`:2020`),
  commented in place as putting the row's title on the project title's own
  x-position (trailing the header's chevron and avatar). That number is
  derived from sibling layout, not tree depth, so `treeIndent(...)` would
  misrepresent what it means.

### Icon buttons

Three tiers:

- **Chrome — `h-8 w-8 rounded-md`.** `IconButton` (`ui.tsx:741-755`), for a
  control that sits in a toolbar or header.
- **Inline — `h-6 w-6 rounded`.** A control that sits *inside* another
  surface: `RefreshButton`'s icon-only branch (`:787`), `Dialog`'s close
  (`:593`), `ErrorNote`'s dismiss (`:699`), and the analogous small controls
  named under "Hover and selection" above. No exported component — just the
  class pair, applied at each site (documented on `IconButton` itself,
  `:735-740`).
- **Rail — `h-8 w-8 rounded-[10px]`.** `RAIL_ICON_BASE` and `railNavClass`
  (`ui.tsx:655-662`, graduated out of `Root.tsx` by the remote-host-UI plan's
  Phase 3 so a feature component could reach them without a `routes/`
  import), for the left icon rail's Projects/Library/Prompts/Agents buttons
  (the host indicator, Settings gear and Scripts dock toggle moved to the status
  bar, and Archive to the header, left of the editor selector; see "v4" below). Selected state is `RAIL_ACTIVE_BAR`
  (`ui.tsx:657-659`), a left `::before` bar, not a background fill — the one
  place this vocabulary marks "active" without a fill.

Two off-scale sizes the app used to carry are gone: `h-6.5 w-6.5` (the Agents
row kebab, `AgentList.tsx:599` — that file, and the per-row kebab it drew, no
longer exist; see "Agents: sidebar shell" below — and `AgentEditor.tsx`'s
numbered step badge) both moved onto the `h-6 w-6` inline tier — `rg -n
'h-6\.5|w-6\.5' src` still returns nothing.

Two controls that *look* like they belong to this vocabulary and deliberately
don't:

- **`Root.tsx:1329`'s window mark** — an `h-7 w-7 aria-hidden` drag region
  sized to the app's logo, not a button. Resizing it risks the Linux
  titlebar layout PR #213 tuned.
- **`Toast.tsx:126`'s dismiss pill** — `h-6 rounded-full`, carrying a
  countdown label, not a square icon button. On-tier for height; its shape
  stays a pill on purpose.

### Floating panels

Two panel radii, split by what the surface *is*: `rounded-lg` for a surface
(`Card`, the `Dialog` panel, `Toast`, both command palettes — all converted
from `rounded-xl`) and `rounded-md` for a menu (`DropdownSelect`'s menu,
`ContextMenu`, `Popover`). Items inside a menu keep `rounded` — a third value,
but for an item, not a panel, and already uniform before this pass.

Every floating panel now renders at `z-50` and closes on `useEscape` — the
LIFO command stack at `src/lib/keybindings/registry.ts:64`. `Popover`
(`ui.tsx:921-980`) is the primitive: an anchored panel sibling to the
point-anchored `ContextMenu` and the listbox `DropdownSelect`, carrying the
same four things those two already needed — Escape via the shared registry,
a portal to `document.body` (so a click inside a `Field`'s `<label>` can't
re-toggle the trigger, and so the panel escapes a dialog's `backdrop-blur`),
a measure-before-paint layout effect, and a clamp into the viewport using the
one `MARGIN` constant (`ui.tsx:148`, previously duplicated as a local
`margin` inside `ContextMenu`). Ten hand-rolled panels migrated onto it; nine
of them had no Escape handler at all before this, and six were rendering at
`z-30` — below a dialog's own overlay.

`MenuItem` (`ui.tsx:985`) is `ContextMenuItem` under a name that doesn't imply
right-click, and `MenuSeparator` (`ui.tsx:990-992`) is the app's one menu
separator (`my-1 h-px bg-edge`), replacing two idioms (`my-1 border-t
border-edge` was the other) that had drifted into the tree independently.

### The Dialog API

`Dialog` (`ui.tsx:537-617`) takes `size: keyof typeof DIALOG_SIZE`
(`sm`/`md`/`lg`/`xl`/`2xl`/`3xl`, `ui.tsx:487-494`) instead of a `className`
width override — never hand a `Dialog` a `max-w-*` in `className`; before
`cx()` merged correctly that pattern is exactly what shipped the five
448px-wide dialogs described above. `align="top"` pins the panel near the
viewport top for a dialog whose body scrolls a long list; `seams` adds a
`border-b` under the header and a `border-t` above the footer, for a panel
running full-bleed (`padded={false}`) that still needs a visible seam against
its inset header/footer bands; `footer` renders in its own `shrink-0` band
below the body, outside the scroll region, so a long dialog's action buttons
never scroll away with the content; `padded` and `scroll` govern the body
band only. `closable` adds a × beside the title for dialogs with no footer
at all.

`ConfirmDialog` (`ui.tsx:643-675`) is the one confirm-footer shape —
dismissive action, then any `extraActions`, then the committing action —
covering the four hand-composed confirms that each reimplemented this footer
independently before it existed. `RenameWorkstreamDialog` deliberately stays
off `ConfirmDialog`: it's a `<form>` with a `type="submit"` primary action,
not a confirm, and reconsidering that would touch the app's otherwise
100%-consistent dialog button ordering, which this pass does not revisit.

### Typography

`--text-2xs` (`src/index.css:62-63`, 0.6875rem / 11px, with a paired
`--text-2xs--line-height: 1rem`) is the one named step below `text-xs`. It
replaced five arbitrary sizes that were each improvising the same idea with
no paired line-height (`text-[10px]`, `[10.5px]`, `[11px]`, `[11.5px]`) at the
sites named in the plan's ride-along scope (`ui.tsx:369`, `:398`,
`AgentList.tsx:254`, `:487` — two of the five, in a file since deleted by the
sidebar-shell plan; see "Agents: sidebar shell" below — and `Toast.tsx:129`)
plus the further sweep in
`AgentEditor.tsx` and `parts.tsx`, both now free of any `text-[Npx]` under
13px. Elsewhere in the tree, a sub-13px arbitrary size converts to `text-xs`
and a 13px-or-above arbitrary size converts to `text-sm`, by nearest named
step — applied within `ScriptDock.tsx`, `Observatory.tsx`, and `Dashboard.tsx`.

This is a scoped sweep, not an app-wide guarantee: files this plan's phases
never opened (`TranscriptView.tsx`, `SessionChat.tsx`'s remaining `text-[10/11px]`
labels, `PromptLibrary.tsx`'s, `GitPanel.tsx`'s, and many others) still carry
their own arbitrary sub-`text-sm` sizes. Converting those was never in scope
here; do not read this ADR as claiming the whole app is on `--text-2xs` now.

Deliberate, named exceptions that stay arbitrary:

- **Four `Button className="text-[12.5px]"` sites** — `Dashboard.tsx:328`,
  `:334`, `Observatory.tsx:454`, `:458`. Measured in the built stylesheet,
  `font-size:12.5px` already sits after `.text-sm` in the cascade, so these
  already won before `cx()` merged correctly; there is nothing to fix and
  converting them to `text-sm` would be a visual change nobody asked for.
- **`ScriptEditor.tsx:63`'s `text-[12.5px] leading-[22px]`** — paired with a
  hand-tuned line height for gutter alignment against line numbers. Converting
  the font size alone without the matching leading breaks that alignment.
- **`ScriptDock.tsx:524`'s Configure-menu item** — still `text-[12.5px]`,
  the one arbitrary size left in that file. It was never converted to
  `MenuItem`/`text-sm` by the `Popover` migration, unlike this ADR's
  companion plan originally assumed; it remains a plain `<button>` inside the
  Configure popover, off both the typography step and the `MenuItem`
  component.
- **`PromptHistoryStack.tsx:42`'s `text-[9px]`** — an ellipsis ("more
  prompts above") indicator smaller than `--text-2xs` itself; not part of the
  10–13.5px family this pass named.
- **`Dashboard.tsx:315`'s `text-[30px]`/`lg:text-[36px]`** — a display
  heading, not part of the sub-`text-sm` family this convention covers.

### Agents: sidebar shell

Later the same day, the `agents-sidebar-shell` plan rebuilt the Agents surface
this document describes throughout: the table (`AgentList.tsx`) is gone,
replaced by a resizable sidebar (root nav, folder/agent tree, and — once an
agent is open — a drilled-in configuration nav) beside a centered
`max-w-5xl` content column. Every citation to `AgentList.tsx` below predates
that rewrite and points at a file that no longer exists; each is annotated
in place rather than silently left dangling. `layout.ts`'s two content-pane
tier predicates, `isContentNarrow` and `isContentCompact`, existed only for
that table's column-dropping and are gone with it — `treeIndent` (see "Tree
indent" below) survives, called from the same place in `agents.ts` it always
was.

The page-column question the reverted `agents-view-page-column` plan raised —
should `/agents` cap its content width — is answered by the new layout, not
by a `max-w-*` on the page: the sidebar is a fixed-width aside and the
content column is `max-w-5xl` by construction, the same width
`PromptLibrary.tsx` and `AgentEditor.tsx` already use. Once the list is a
288px aside there is no wide table left to cap.

The new sidebar's own rows — root nav, folder/agent tree, drilled-in
configuration sections — do not build on `rowClass`. They follow the
Prompts-anatomy bordered-row pattern instead (`border-accent/30 bg-accent/10`
selected, `border-transparent … hover:bg-edge/40` otherwise,
`PromptLibrary.tsx:429-433`), matching the surface the sidebar copies its
anatomy from. This is not a gap in "Row tiers" above — `rowClass` still
governs the rest of the app's rows — it is the same second idiom
`PromptLibrary`'s own sidebar was already carrying before this note existed.

### v4: palette, radii, and the Settings and status-bar surfaces (2026-09-24)

The Redesign v4 canvas's tokens apply app-wide on the light theme. Dark and the
named themes keep their palettes.

**Palette.** Spec names map onto existing tokens rather than duplicating them:

| v4 | Token | Light |
|---|---|---|
| `--bg` / `--panel` / `--muted` | `surface` / `panel` / `panel-2` | `#f7f7f7` / `#ffffff` / `#f5f5f5` |
| `--line` / `--line-2` | `edge` / `edge-2` (new) | `#e9e9e9` / `#f0f0f0` |
| `--ink` / `--ink-2` / `--ink-3` / `--ink-4` | `ink` / `ink-2` (new) / `ink-dim` / `ink-4` (new) | `#111` / `#303030` / `#5e5e5e` / `#8a8a8a` |
| `--accent` / `--accent-soft` | `accent` / `accent-soft` (new) | `#111` / `#f0f0f0` |
| `--info` / `--ok` / `--warn` / `--bad` | `info` (new) / `ok` / `warn` / `bad` | `#206eec` / `#368813` / `#e0a400` / `#ef0028` |
| diff add / remove | `diff-add` / `diff-del` (new) | `#2c6f10` / `#c4001f` |

The new tokens are `color-mix()`es of each theme's own tokens in the base
`@theme`, tuned so the light values land exactly on the spec. `html.light`
overrides only what no mix reaches: `info`, `warn-ink`, `bad-ink`, `diff-add`,
`diff-del`, and the lighter `accent-soft`/`track` mixes. `--color-ansi-magenta`
is the first ansi slot a theme overrides, because the base aliases it to
accent, which is ink on light.

`#e0a400` is a fill. As text on white it is 2.2:1, so warning text is
`text-warn-ink` (`#7a5a00` on light, `warn` elsewhere), and every `text-warn`
was swept to it. `bad-ink` does the same for danger text. `info` is the
semantic token for information; `brand` stays for syntax, terminal and
file-icon hues.

**Radii.** `rounded-card` (5px) is Card, the Dialog panel, Popover, the
DropdownSelect menu and ContextMenu. That collapses "Floating panels"' two
panel radii into one. `rounded-control` (3px) is Button, TextInput, the
DropdownSelect trigger and options, Badge, IconButton, menu items and the
inline close/dismiss buttons. `cx()` registers both names with tailwind-merge;
unregistered, `rounded-card` would survive a caller's `rounded-none`. Left
unmigrated: Toast, the two palettes, the `rowClass` tiers, the rail
(`rounded-[10px]`).

**Typography.** `--text-3xs` (10px) joins `--text-2xs`. The `lbl` utility is
the v4 label: mono 10px, 600, uppercase, `.07em`, `ink-dim`. The v4 surfaces
(status bar, Settings popover, the Settings view, Servers) use the canvas's
exact 9.5, 10.5, 12.5, 13, 13.5 and 19px as arbitrary values, a named
exception like those under "Typography".

**Primitives.**
- `Toggle` is a native `<input type="checkbox" role="switch">` inside a
  `<label>`: 34×18 track, 12px knob, and `children` makes a whole row its
  label. Space toggles; Enter does not. `isEditableTarget` no longer counts
  checkbox, radio, button or range inputs as text entry, so a focused switch
  does not swallow bare-key shortcuts.
- `SegmentedControl` and `HarnessPicker`'s radio row share `useRovingRadio`
  (`src/lib/useRovingRadio.ts`): one tab stop, arrows move and select,
  wrapping past disabled options. The feature-local segmented controls in
  `agents/parts.tsx` and `CreateWorkstreamDialog.tsx` are unmigrated.
- `Popover` gains opt-in `caret` and `trapFocus` (with `initialFocusRef`,
  `restoreFocusRef`, dialog role) through `useFocusTrap`. Tab is cycled by
  hand, because WKWebView skips buttons unless macOS keyboard navigation is
  on, and only while focus is inside the panel, so a nested DropdownSelect
  listbox keeps its own keys. There is deliberately no `focusin` guard.
- `Button` gains `size="sm"` (`h-6`). `SettingsCard`
  (`src/components/SettingsCard.tsx`) is the v4 card: title 14px/700,
  description 12.5px `ink-dim`.
- `rowClass`'s selection tint is `bg-accent-soft` (the same colour as
  `bg-accent/15` on dark).

**Sizes recorded as exceptions.**
- The status bar is 34px (`STATUS_BAR_HEIGHT`, `src/lib/layout.ts`).
- Its gear and the Scripts dock toggle beside it are 26×24; the permission pill 24px; a meter's track 34×4; the avatar 20px.
- The Settings nav's rows are 34px, which is not a third `rowClass` tier.
- Toast sits `STATUS_BAR_HEIGHT + 24` above the window's bottom edge.
- The header's Archive button is 26×26 beside 28×28 neighbours. Its Archived
  worktrees popover is 440px with a `0 12px 30px` shadow, and caps its own list
  at the window height, since `Popover` has no cap. Its only new tokens are
  `tag-purge-bg` and `destructive-border`; the spec's `tag-purge-text` and
  `destructive-text` are `warn-ink` and `bad-ink`.

**Icons.** `lucide-react` at 16px and stroke 1.5 (`ICON` in
`src/components/lucide.ts`) on the v4 surfaces. `icons.tsx` stays everywhere
else and for brand marks (`harnessIcon`).

## Rationale

Every value recorded above already existed somewhere in the tree before this
pass; none were invented. The point of writing them down is that they existed
in *several* places with *small, undocumented* differences — `h-13` next to
`h-7` rows with no shared name for either, four idioms for the same margin
constant, ten dialogs each free to pick a width class that may or may not
survive Tailwind's cascade. A convention nobody wrote down can't be checked,
so each new dialog, row, or menu re-derived its own answer, and the answers
drifted (the Agents view's 15px, 41% adjacent-row-height mismatch is the
clearest symptom). This ADR is the answer key: it is a description of a
converged app, not a proposal, and every claim above was checked against the
current tree at the file:line cited rather than transcribed from an
implementation plan.

`CheckStatus::Warn` (`src-tauri/src/commands/projects.rs:109-121`) is worth
one line here even though it's a backend, not a visual, change: the same
pass removed the four global `binary.*` health checks from the per-project
`ProjectSettingsDialog` (they duplicated Settings → Binaries and never
varied by project), which took `Warn`'s only constructor with it. The variant
stays, `#[allow(dead_code)]`, because it's part of the specta-exported
`CheckStatus` binding the frontend still switches on.

## Consequences

- Adding a new dialog means picking a `size` from the named scale, never
  reaching for a `max-w-*` `className`. `cx()` will *let* a conflicting
  `className` override win now, which is exactly the trap that shipped the
  bug this ADR opens with — the named scale exists so nobody needs to
  discover that the hard way twice.
- Adding a new row means picking `compact` or `standard` via `rowClass`
  and reaching for the module's hover/selection constants, not re-deriving a
  height or a hover class from scratch.
- Adding a new floating panel means reaching for `Popover`, not hand-rolling
  another one with its own Escape story (or lack of one) and its own z-index.
- **Explicitly out of scope, so nobody "fixes" these later:**
  - **Uneven theme-token override coverage.** The default `@theme` declares
    47 `--color-*` tokens, of which 31 are the semantic palette: `light`
    overrides all 31, `claude`/`cobalt`/`matrix` override 18, `solarized` 20.
    `src/index.css:104-110` and `:113-119` explain why — the
    stage/doc-type/activity hues deliberately inherit the Dark+ defaults on
    dark themes and are deepened only for the light surface. This is by
    design, not an oversight, and this pass added exactly one new token
    (`--text-2xs`), which is typography, not colour.

    The remaining 16 are the `--color-ansi-*` terminal slots
    (`src/index.css:143-165`), added later by the Omarchy theming work. No
    `html.<name>` block overrides any of them, and none needs to: eight are
    `var()` aliases onto semantic tokens and so follow whatever theme is
    active, and eight are fixed xterm Tango literals with no semantic
    equivalent to alias. They are not part of the design vocabulary — nothing
    in the app may use `bg-ansi-*`; they exist solely for
    `xtermThemeFromCss()` to read.
  - **The two `h-11` Library headers** (`DocDetail.tsx:108`,
    `LibrarySidebar.tsx:70`), 4px taller than the 40px window bar. PR #213
    recorded this as known-and-left; still left.
  - **The 32px row band and the multi-line rows** — named above under "Row
    tiers." Three tiers would defeat a two-tier vocabulary.
  - **Every arbitrary sub-`text-sm` size outside the files this pass swept**
    — named above under "Typography." The convention (`--text-2xs`, then
    `text-xs`/`text-sm` by nearest step) applies going forward; it was not
    retrofitted app-wide.
  - **The third hover value, `hover:bg-edge/60`, on the inline icon-button
    tier** — named above under "Hover and selection." Not a gap in the
    two-value split; a third, deliberate value for a case the two-value split
    was never meant to cover.
  - **Three of the four `RowSpine`-shaped literals** (`Settings.tsx:1138`,
    `Root.tsx:1901`, `LibrarySidebar.tsx:146`) — named above under "Row
    tiers." Only `Root.tsx:2035` runs through the `RowSpine` component.
