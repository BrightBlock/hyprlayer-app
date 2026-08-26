import { fileURLToPath, URL } from "node:url";

import { configDefaults, defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },

  build: {
    chunkSizeWarningLimit: 700,
  },

  clearScreen: false,
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
      ignored: ["**/src-tauri/**", "**/target/**", "**/archived-contexts/**"],
    },
  },

  test: {
    exclude: [...configDefaults.exclude, "archived-contexts/**"],
    setupFiles: ["./src/test-setup.ts"],
  },
}));
