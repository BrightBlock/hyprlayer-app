# ADR 0003: Frontend layering — routes are wiring, features are content

**Status:** accepted · 2026-07-05

## Decision

The frontend mirrors ADR-0001's thin-shell contract. Import direction is one-way:

```
main.tsx / router.tsx   (wiring — may import anything)
        ↓
src/routes/*            (route shells: layout, params, pane wiring — no domain types)
        ↓
src/features/*          (feature content: components, stores, domain logic)
        ↓
src/lib/*  src/components/*   (shared foundation: IPC, queries, stores, design system)
```

- **`src/lib` and `src/components` never import from `src/features` or `src/routes`.**
  When the foundation needs feature behavior (an event side-effect, a mutation
  cleanup, a navigation), it exposes a registration seam and `main.tsx` /
  `router.tsx` wires the feature in at startup: `lib/events.registerEventBridge`,
  `lib/queries.onWorkstreamArchived`, `lib/appNav.registerAppNav`.
- **`src/features` never imports from `src/routes` or the `router` singleton.**
  Domain types live in the feature that owns their persistence (e.g. `ShellTab`
  in `features/session/workstreamTabs.ts`), never in a route file.
- **Cross-feature imports are allowed only for another feature's exported domain
  API** (types, store actions), not its components — and shared *vocabulary*
  (stages, harness/model options, doc kinds, formatters, the terminal stack)
  belongs in `src/lib`, not in whichever feature used it first.
- **Cross-directory imports use the `@/` alias** (`@/lib/...`, `@/features/...`)
  so the layer being imported is visible at the import site; `./` siblings stay
  relative.

## Enforcement

`eslint.config.js` encodes the two hard rules with `no-restricted-imports`
(both `@/` and relative spellings). `pnpm lint` fails on violations; run it
before pushing. The alias is defined in `tsconfig.json` (`paths`) and
`vite.config.ts` (`resolve.alias`) — keep the two in sync.

## Consequences

- A new feature author has one answer to "where does this go": route shells
  stay dumb, feature code lives in its feature, shared vocabulary graduates to
  `src/lib` the moment a second feature needs it.
- The registration seams add one line of wiring in `main.tsx` per hook, in
  exchange for a foundation layer that can be reasoned about (and tested)
  without loading any feature.
