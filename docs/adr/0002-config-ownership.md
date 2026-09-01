# ADR 0002: The CLI owns its config; the app reads and watches

**Status:** accepted · 2026-06-12 · amended 2026-07-14 (profile creation routes through
`hyprlayer thoughts profile create` — no new exception needed) · amended 2026-08-20
(the agent-bundle SHA probe is read-only against GitHub — not a new exception) ·
amended 2026-08-24 (version pinning classifies `ai reinstall`'s install source from its
stdout — not a new exception) · amended 2026-08-24 (freshness is a version comparison on a
1.6.0 CLI; the SHA probe narrows to older binaries) · amended 2026-08-30 (the SHA probe
now runs in `hyprlayer-core` — ADR-0008 moved the bundle network calls out of the shell)

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
- Bundle freshness is answered without a check-only CLI command, because there is none. On a
  1.6.0-or-newer CLI it is a local string comparison — `assetsVersion` against
  `agent_bundle::desired_version` (the pin, else `binaryVersion`) — so nothing is written,
  nothing is fetched, and the answer is correct offline. Older binaries keep the read-only
  GitHub ref-advertisement probe of `BrightBlock/hyprlayer-cli@master` (parsed in
  `crates/hyprlayer-core/src/agent_bundle.rs`, fetched in
  `crates/hyprlayer-core/src/runtime/agents.rs` through the `ProviderHttp` port),
  compared against the CLI's own `agentsInstalledSha` — not a new exception to the
  single-writer rule.

  The fetch used to live in `src-tauri/src/commands/agents.rs` because ADR-0001 kept core
  dependency-light. ADR-0008 reverses that: core owns `reqwest` and every service's network
  calls, and the desktop shell no longer has an HTTP client of its own — `tauri-plugin-updater`
  still reaches the network from inside the shell, as a carve-out ADR-0008 keeps there
  deliberately. Where the probe runs does not change what it does — it still writes nothing
  and reads no config.

  The split is not an optimisation: 1.6.0's `AgentTool::install` records `fetch_master_sha()`
  as `agentsInstalledSha` regardless of which release asset it unpacked, so there the SHA
  answers "has master moved", not "is my bundle behind" — one README commit on the CLI repo
  would report a new bundle to every machine already holding the current one. Upstream ask:
  `hyprlayer ai check --json`, which would retire the remaining SHA logic too.
- The version picker's silent-fallback tripwire (`reinstall_source` in
  `crates/hyprlayer-core/src/agent_bundle.rs`) classifies what `hyprlayer ai reinstall
  --version` installed by parsing the CLI's stdout, because the command has no `--json` and
  both its exit code and the config it writes read as success even when its source chain fell
  back to master's tree. The parse is coupled to CLI 1.6.0's wording (verified against
  9a2a33c) and degrades to an explicit `Unknown` rather than guessing when the wording moves.
  Not a new exception: it writes nothing and reads no config. Upstream ask: `hyprlayer ai
  reinstall --json` (a machine-readable install summary naming the source), which retires the
  parse.
