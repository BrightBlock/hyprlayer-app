# hyprlayer-app

## Build & verify

| Command | What it does |
|---|---|
| `cargo test` | Rust workspace tests (287) + regenerates `src/lib/bindings.ts` |
| `pnpm typecheck` | Frontend type-check (`tsc --noEmit`) |
| `pnpm lint` | ESLint (includes import-boundary rules) |
| `pnpm test` | Vitest (598 tests) |
| `pnpm build` | typecheck + vite build |

Always run all four before pushing. The release gate is `scripts/release.sh` which runs the same checks.

## Architecture: two thin shells

The backend mirrors ADR-0001: `crates/hyprlayer-core` (zero Tauri deps, all logic) + `src-tauri` (thin IPC shell). The frontend mirrors that same contract via ADR-0003: routes are wiring, features are content, `src/lib` and `src/components` are the shared foundation.

**Import direction is one-way downward.** No layer may import upward.

```
main.tsx / router.tsx   → may import anything
src/routes/              → may import features, lib, components
src/features/            → may import lib, components; never routes
src/lib/ src/components/ → shared foundation; never imports features or routes
```

When a shared module needs feature-side behavior, expose a registration seam
(see `lib/events.ts` / `lib/appNav.ts` / `lib/queries.ts`) and wire it in `main.tsx`.

## Where things live

| If it... | Put it in... |
|---|---|
| Is a shared vocabulary type (Stage, Harness, DocKind, PermissionMode) | `src/lib/domain/` — derived from Rust enum bindings, never re-declared |
| Is a shared UI primitive (popover, dialog, empty hint) | `src/components/ui.tsx` — check here before building one |
| Is a shared formatting helper | `src/lib/format.ts` |
| Is a shared path helper | `src/lib/path.ts` |
| Is a terminal/PTY concern | `src/lib/terminal/` |
| Is used by exactly one feature | that feature's directory |
| Is used by *two or more* features | graduate to `src/lib/` or `src/components/` |
| Is a route/page definition | `src/routes/` — keep it dumb: layout, params, pane wiring only. No domain types, no store imports. |

@docs/adr/0001-tauri-workspace-and-cli-types.md
@docs/adr/0003-frontend-layering.md
@docs/adr/0004-vocabulary-standard.md