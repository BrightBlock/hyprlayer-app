import { fileURLToPath, URL } from "node:url";

import { configDefaults, defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    // Mirrors tsconfig's "@/*" → "src/*". Cross-directory imports use the
    // alias so the layer being imported is visible at the import site
    // (docs/adr/0003-frontend-layering.md).
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },

  build: {
    // Vite's 500 kB warning is a heuristic about download latency on a web
    // deploy. Nothing here is downloaded: Tauri serves the bundle off local
    // disk through its custom protocol, and only the entry chunk (~190 kB
    // gzipped: react-dom, the router, the query client, the app shell) is
    // fetched at startup. Everything above the old limit is behind a dynamic
    // import — the CodeMirror core (doc/file/diff panes), the oniguruma WASM
    // and the C++ grammar (first highlighted chat fence). Left high enough
    // that a genuinely eager regression — something heavy landing in the
    // entry chunk — still trips it.
    chunkSizeWarningLimit: 700,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. don't watch Rust sources or build output. `target/` is at the
      //    workspace root (not under src-tauri/), and on Windows watching a
      //    cargo-locked .dll there throws EBUSY and crashes the dev server.
      //    `archived-contexts/` only exists when you dogfood Hyprlayer on its
      //    own repo: archiving a workstream moves its worktree (a full repo
      //    checkout, `src/*.tsx` and all) into it, which would otherwise trip
      //    a full-reload — the dev-only "screen flash" on archive.
      ignored: ["**/src-tauri/**", "**/target/**", "**/archived-contexts/**"],
    },
  },

  // Vitest — run with `pnpm test`
  test: {
    exclude: [...configDefaults.exclude, "archived-contexts/**"],
    setupFiles: ["./src/test-setup.ts"],
  },
}));
