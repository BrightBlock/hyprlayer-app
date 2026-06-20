import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

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
    setupFiles: ["./src/test-setup.ts"],
  },
}));
