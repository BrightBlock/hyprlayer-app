---
paths:
  - "src/**/*.{ts,tsx}"
---

# Frontend layering rules

**One-way import direction.** No layer may import upward. This is enforced by ESLint
(`pnpm lint` catches violations).

```
main.tsx / router.tsx   → may import anything
src/routes/              → may import features, lib, components
src/features/            → may import lib, components; never routes
src/lib/ src/components/ → shared foundation; never imports features or routes
```

## Registration seams

When `src/lib` or `src/components` needs behavior from a feature (event side-effects,
mutation cleanup, navigation), it exposes a registration seam. `main.tsx` wires the
feature in at startup. Examples: `lib/events.registerEventBridge`,
`lib/queries.onWorkstreamArchived`, `lib/appNav.registerAppNav`.

## Domain types

- Shared vocabulary (Stage, Harness, DocKind, PermissionMode) lives in `src/lib/domain/`,
  derived from Rust enum bindings. Never re-declare.
- Feature-owned domain types (e.g. `ShellTab`, `WsTabState`) live in the feature that
  owns their persistence, never in a route file.
- The moment a type is used by two features, graduate it to `src/lib/domain/`.

## Routes

Route files (`src/routes/*.tsx`) are *wiring only*:
- Layout, params, pane wiring
- No domain types, no store imports, no business logic

## Imports

Cross-directory imports use `@/` alias (`@/lib/...`, `@/features/...`) so the layer
being imported is visible at the import site. `./` siblings stay relative.

## Shared primitives

Before building a new popover, dialog, empty state, palette, or keybinding recorder,
check `src/components/ui.tsx` and `src/lib/` for existing implementations.

## See also

@docs/adr/0003-frontend-layering.md