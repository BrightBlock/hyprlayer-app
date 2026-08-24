# ADR 0002: The CLI owns its config; the app reads and watches

**Status:** accepted · 2026-06-12 · amended 2026-07-14 (profile creation routes through
`hyprlayer thoughts profile create` — no new exception needed) · amended 2026-08-20
(the agent-bundle SHA probe is read-only against GitHub — not a new exception) ·
amended 2026-08-24 (version pinning classifies `ai reinstall`'s install source from its
stdout — not a new exception)

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

Profile creation (the Add-project dialog's "New profile…") is NOT an exception: it shells out
to `hyprlayer thoughts profile create <name> --repo … --repos-dir … --global-dir …` (all flags
always passed — an omitted flag would make dialoguer prompt, and with no TTY it errors). The
app only pre-validates names the CLI would accept or silently sanitize (`default`, `__`
sentinels, unsafe charset) and reads the created entry back from the config the CLI wrote.

## Consequences

- No config lock protocol needed; the CLI's atomic rename keeps reads consistent.
- CLI-driven changes (user runs `hyprlayer thoughts init` in a terminal) appear in the app
  within ~2s without restart.
- The Agents view's bundle-freshness chip resolves `BrightBlock/hyprlayer-cli@master`'s current
  SHA itself (a GitHub ref-advertisement probe, parsed in
  `crates/hyprlayer-core/src/agent_bundle.rs`, fetched in `src-tauri/src/commands/agents.rs`)
  rather than reading it from the config, because the CLI has no check-only command to ask "is a
  newer bundle available" without also installing it. This is not a new exception to the
  single-writer rule: the probe is read-only against GitHub, writes nothing to `config.json`, and
  the installed SHA it compares against still comes from the CLI's own
  `agentsInstalledSha`. Upstream ask: `hyprlayer ai check --json`, which would let the app retire
  its copy of the SHA-resolution logic.
- The version picker's silent-fallback tripwire (`reinstall_source` in
  `crates/hyprlayer-core/src/agent_bundle.rs`) classifies what `hyprlayer ai reinstall
  --version` installed by parsing the CLI's stdout, because the command has no `--json` and
  both its exit code and the config it writes read as success even when its source chain fell
  back to master's tree. The parse is coupled to CLI 1.6.0's wording (verified against
  9a2a33c) and degrades to an explicit `Unknown` rather than guessing when the wording moves.
  Not a new exception: it writes nothing and reads no config. Upstream ask: `hyprlayer ai
  reinstall --json` (a machine-readable install summary naming the source), which retires the
  parse.
