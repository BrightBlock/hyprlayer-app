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
