import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

const LUCIDE_ONLY_IN_ICON = {
  name: "lucide-react",
  message: "Use <Icon name=…> from @/components/Icon; it is the only lucide-react importer.",
};
// Every block that sets no-restricted-imports replaces the rule's options, so each lists this.
const NO_NATIVE_ALERTS = {
  name: "@tauri-apps/plugin-dialog",
  importNames: ["confirm", "ask", "message"],
  message:
    "No native alerts: use ConfirmDialog, Dialog or useConfirm from @/components/ui (ADR-0006). open and save are fine.",
};
const NO_ROUTE_IMPORTS = {
  group: ["@/routes/*", "**/routes/**", "@/router", "../router", "../../router"],
  message:
    "Shared modules must not import routes or the router singleton. Use lib/appNav's registered seam.",
};
const NO_INLINE_SVG = {
  selector: "JSXOpeningElement[name.name='svg']",
  message: "No inline <svg> icons; add a semantic name to @/components/Icon.",
};

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
      "no-restricted-imports": ["error", { paths: [LUCIDE_ONLY_IN_ICON, NO_NATIVE_ALERTS] }],
      "no-restricted-syntax": ["error", NO_INLINE_SVG],
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
            NO_ROUTE_IMPORTS,
          ],
          paths: [LUCIDE_ONLY_IN_ICON, NO_NATIVE_ALERTS],
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
          paths: [LUCIDE_ONLY_IN_ICON, NO_NATIVE_ALERTS],
        },
      ],
    },
  },
  {
    files: ["src/components/Icon.tsx"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@/features/*", "**/features/**"],
              message: "src/components must not import from src/features.",
            },
            NO_ROUTE_IMPORTS,
          ],
          paths: [NO_NATIVE_ALERTS],
        },
      ],
    },
  },
  {
    // Logos and a data chart, not icons.
    files: [
      "src/components/BrandMark.tsx",
      "src/components/Logo.tsx",
      "src/features/dashboard/KnowledgeGraph.tsx",
    ],
    rules: { "no-restricted-syntax": "off" },
  },
  {
    // Node-run memory benches and the tests that spawn them.
    files: ["src/**/*.mjs"],
    languageOptions: { globals: { ...globals.node } },
  },
);
