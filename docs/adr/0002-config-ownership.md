# ADR 0002: The CLI owns its config; the app reads and watches

**Status:** accepted · 2026-06-12

## Decision

The hyprlayer CLI config (`dirs::config_dir()/hyprlayer/config.json`) has a single writer: the
CLI. The app reads it (tolerant parse, mtime-cached), watches it for changes (2s mtime poll →
`config-changed` event → query invalidation), and routes every mutation through CLI commands
(`thoughts init`, `thoughts sync`, ...). At startup the app runs `hyprlayer storage info --json`
once, which forces the CLI's load→migrate→save path so direct reads always see the current
schema.

Two temporary, narrowly-scoped exceptions (both atomic tmp+rename, both with upstream PRs
planned to eliminate them):

1. **Bootstrap** (phase 1): `thoughts init --yes` requires an existing config, so true first-run
   writes a minimal v4 config before handing over to the CLI. Upstream ask: `--user` flag.
2. **Worktree mappings** (phase 2): skills inside a worktree resolve their project via
   cwd-keyed `repoMappings`, so worktree provisioning adds/removes a mapping entry. Upstream
   ask: `hyprlayer thoughts link --worktree`.

## Consequences

- No config lock protocol needed; the CLI's atomic rename keeps reads consistent.
- CLI-driven changes (user runs `hyprlayer thoughts init` in a terminal) appear in the app
  within ~2s without restart.
