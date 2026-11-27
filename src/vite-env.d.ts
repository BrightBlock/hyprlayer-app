/// <reference types="vite/client" />

// Missing from TypeScript's DOM lib.
interface FocusOptions {
  focusVisible?: boolean;
}

interface ImportMetaEnv {
  readonly VITE_PERF_PROBE?: string;
}
