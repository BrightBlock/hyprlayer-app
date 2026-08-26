import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default tseslint.config(
  {
    ignores: ["dist/", "node_modules/", "src-tauri/", "src/lib/bindings.ts"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
      "no-empty": ["error", { allowEmptyCatch: true }],
    },
    languageOptions: {
      globals: { ...globals.browser },
    },
  },
  {
    files: ["src/lib/**/*.{ts,tsx}", "src/components/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@/features/*", "**/features/**"],
              message:
                "src/lib and src/components are the shared foundation — they must not import from src/features. Register a hook from main.tsx instead (see lib/events.registerEventBridge / lib/queries.onWorkstreamArchived).",
            },
            {
              group: ["@/routes/*", "**/routes/**", "@/router", "../router", "../../router"],
              message:
                "Shared modules must not import routes or the router singleton. Use lib/appNav's registered seam.",
            },
          ],
        },
      ],
    },
  },
  {
    files: ["src/features/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@/routes/*", "**/routes/**", "@/router", "../router", "../../router"],
              message:
                "Features must not import routes or the router singleton — routes are wiring above features. Move the shared type into the feature, or use lib/appNav.",
            },
          ],
        },
      ],
    },
  },
);
