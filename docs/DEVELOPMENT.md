# Development

Read [AGENTS.md](../AGENTS.md) for the architecture and the checks to run before
pushing.

Prerequisites: Rust (stable), Node 22+, pnpm 11+, Zig 0.16.0 (pinned in
`mise.toml`; `mise install` fetches it), and the
[Tauri platform deps](https://tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm tauri dev          # run the app (debug)

cargo test --workspace  # rust tests (also regenerates src/lib/bindings.ts); the release gate runs them with cargo nextest
pnpm typecheck && pnpm lint && pnpm test   # frontend checks
pnpm tauri build        # production bundle
```

To exercise the app against real data, install the hyprlayer CLI and initialize
a repository:

```sh
brew install brightblock/tap/hyprlayer   # or scoop/winget on Windows
cd ~/code/your-repo && hyprlayer thoughts init
```

## Dogfooding `main`

"Hyprlayer Dev" is a release-profile build of `origin/main` that installs beside
the released app under its own identifier — its own database, settings and
launcher entry, so the two never collide. It needs no checkout: anyone with
access to this repo can run it straight from GitHub, where `gh` supplies the
auth.

```sh
bash <(gh api repos/BrightBlock/hyprlayer-app/contents/scripts/dev-install-linux.sh \
         -H 'Accept: application/vnd.github.raw')     # macos: dev-install-macos.sh
```

```powershell
& ([scriptblock]::Create((gh api repos/BrightBlock/hyprlayer-app/contents/scripts/dev-install-windows.ps1 -H 'Accept: application/vnd.github.raw' | Out-String)))
```

It builds in a throwaway clone under the cache dir, so it can run while you have
uncommitted work. Rerun the same command to update (it no-ops if `main` has not
moved); pass `--uninstall` / `-Uninstall` to remove it, `--help` for the rest of
the flags. Prerequisites are the full build toolchain above — no signing or
publish credentials.

See [RELEASING.md](RELEASING.md) for how releases and in-app updates are published.
