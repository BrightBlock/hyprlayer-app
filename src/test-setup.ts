import { vi } from "vitest";

// @xterm packages reference `self` (browser global) at module-evaluation time
// in their UMD wrapper, which fails in the Node.js Vitest runtime before jsdom
// is up. Stub them globally so any test whose import graph reaches TerminalTab
// (e.g. via the workstreamTabs → ProjectPage type-import chain) does not crash
// on module load. Tests that actually exercise the terminal component should run
// in a jsdom environment and use their own more-specific mocks.
vi.mock("@xterm/xterm", () => ({
  Terminal: class Terminal {
    cols = 80;
    rows = 24;
    options: Record<string, unknown> = {};
    loadAddon() {}
    open() {}
    onData() {
      return { dispose() {} };
    }
    write() {}
    writeln() {}
    dispose() {}
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class FitAddon {
    fit() {}
  },
}));

// Node 22 defines a `localStorage` global that is undefined unless the runtime
// was started with `--localstorage-file`, and it wins over the one jsdom would
// otherwise install — so `zustand/persist`'s default storage resolves to
// undefined and any `useUiStore` write throws `Cannot read properties of
// undefined (reading 'setItem')`. Give the persisted stores a real in-memory
// backing instead.
if (!globalThis.localStorage) {
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, String(value)),
      removeItem: (key: string) => void store.delete(key),
      clear: () => store.clear(),
      key: (i: number) => [...store.keys()][i] ?? null,
      get length() {
        return store.size;
      },
    },
  });
}
