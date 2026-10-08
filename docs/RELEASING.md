# Releasing Hyprlayer

Hyprlayer ships in-app, signed self-updates via the Tauri updater plugin.
Binaries are published to **GitHub Releases**; a tiny signed manifest lives on
**brightblock.ai**, which running apps poll to discover and install updates.

The source repo stays **private**. Because a private repo's release assets
can't be downloaded anonymously (and the updater fetches them unauthenticated),
binaries are published to a **separate public repo**, `BrightBlock/hyprlayer-releases`,
which holds releases only — no source.

## How it fits together

There is **no CI release** — GitHub Actions was removed to save minutes.
Everything is built, signed, and published from the release Mac by
`scripts/release-local.sh`, which builds the two non-Mac bundles in Docker
containers on the same Mac and signs their output locally, so the ed25519
signing key never leaves it.

```
private: BrightBlock/hyprlayer-app  (source)
public:  BrightBlock/hyprlayer-releases  (binaries only)
public:  brightblock.ai  (manifest JSON, served by Vercel)
public:  aur.archlinux.org/hyprlayer-desktop-bin  (PKGBUILD only)

./scripts/release.sh patch        # bump version + tag, locally (no push)
        ▼
./scripts/release-local.sh        # build + sign + publish, all from the Mac
  ├─ mac    (here)      → Hyprlayer_<v>_aarch64.dmg + Hyprlayer.app.tar.gz(.sig)
  │                       Developer ID–signed + notarized during the build
  ├─ win  (container)   → Hyprlayer_<v>_x64-setup.exe  built unsigned in Docker,
  │                                                    ed25519-signed here
  ├─ linux  (container) → Hyprlayer_<v>_amd64.deb + Hyprlayer-<v>-1.x86_64.rpm
  │                       + a rendered PKGBUILD/.SRCINFO (no signature — see below)
  ├─ --smoke (optional) → install-tests both before publishing (see "Smoke tests")
  ├─ release      → GitHub Release on hyprlayer-releases (public) w/ assets
  ├─ manifest     → latest.json + changelog.json → jtsiros/hyprlayer.dev
  │                 (darwin-aarch64 + windows-x86_64) → Vercel
  ├─ cask         → BrightBlock/homebrew-tap version + dmg sha256 bump
  └─ aur          → hyprlayer-desktop-bin PKGBUILD + .SRCINFO
        ▼
Running apps  check() → https://brightblock.ai/hyprlayer/updates/latest.json
                        → asset URLs on hyprlayer-releases (public)
              downloadAndInstall() → relaunch()
```

Both container legs compile a clean `git archive` of the release tag — never the
working tree — so they build exactly what was tagged. Neither holds any secret.

| Flag | Builds |
| --- | --- |
| *(none)* | macOS arm64 + Windows x64 + Linux x64 + AUR |
| `--no-linux` | macOS + Windows (implies `--no-aur`) |
| `--no-windows` | macOS + Linux + AUR |
| `--no-aur` | everything, but don't push the AUR package |
| `--mac-only` | macOS only |
| `--rebuild` | ignore cached artifacts, force a clean build of everything |
| `--smoke` | install-test the Windows and Linux builds before publishing (see "Smoke tests") |

A platform left out is simply absent from `latest.json`, so its installs see
"no update" until the next all-platform release — they are never served a
mismatched build.

Each platform's signed output for the version being published is checked for
on disk before that leg builds, so re-running after a transient failure (a
flaky builder, a missing tool) reuses whatever already succeeded instead of
rebuilding everything — mac and Windows artifacts are keyed by their
version-stamped filename, Linux by its deb/rpm filenames plus the rendered
AUR PKGBUILD/.SRCINFO. A release stopped by `--smoke` reruns the same way.
Pass `--rebuild` to ignore all of that and force a fresh build.

Update packages are **ed25519-signed**; the public key is baked into the app
(`src-tauri/tauri.conf.json` → `plugins.updater.pubkey`) and verifies every
download before install. The app also reads `changelog.json` from the same
host to show release notes — non-fatal if it's missing, except on Linux where
it *is* the update signal (see "How Linux updates work").

## Cutting a release

```sh
./scripts/release.sh patch     # or minor / major / an explicit version
```

The script first runs the verification gate, which `RELEASE_SKIP_CHECKS=1` skips.
It needs Docker running, cargo-nextest 0.9.145+ (`brew install cargo-nextest`), and
`git` and the hyprlayer CLI installed, and it stops at once if Docker or nextest is
missing or a `tauri dev` from this checkout is running (the test build rewrites
`target/debug/Hyprlayer` with the production identifier). It builds the server
sidecar, points the E2E scenarios at it through `HYPRLAYER_E2E_SERVER_BIN`, and
checks that its nextest filter admits exactly the five expected ignored tests (the
container tier, the desktop's ssh sign-in and the opt-in `pr_lifecycle`). One
`cargo nextest run --workspace --profile gate --run-ignored all` then runs every Rust
test in its own process: the local E2E scenarios in `crates/hyprlayer-e2e`, that
crate's container tier (a Linux host in Docker reached over real SSH, capped by the
`docker` test group in `.config/nextest.toml`) and the desktop's ssh sign-in
scenario. A standalone `cargo check` of the two library crates (see "Library target
checks"), `pnpm typecheck`, `pnpm lint` and `pnpm test` follow, and each step prints
its duration. A failing E2E scenario leaves its artifact in `target/e2e/<scenario>/`,
and the run's JUnit report is `target/nextest/gate/junit.xml`. A test killed by the
hang guard skips its cleanup: remove its containers with
`docker rm -f $(docker ps -aq --filter label=hyprlayer-e2e=1)` and any stray server
with `pkill -f "$(pwd -P)/src-tauri/binaries/hyprlayer-server-"` (the gate runs the
staged sidecar copy, which neither the release app nor `tauri dev` starts). It then gates
on `changelog.json`: it scaffolds a stub entry for the new version if missing and refuses to proceed until you've written a `summary`
(≤15 words) and at least one `notes` bullet. It then bumps the version in
`package.json`, `Cargo.toml` (`[workspace.package]`), and
`src-tauri/tauri.conf.json` in lockstep, commits, and tags. Push to trigger CI:

```sh
git push && git push --tags
```

## One-time setup (required before the first release)

### 0. Create the public releases repo

Create `BrightBlock/hyprlayer-releases` as a **public** repo with a single
initial commit on `main` (a README explaining it holds Hyprlayer binaries is
enough). CI publishes every Release here; it never pushes source.

### 1. Repository secrets — `BrightBlock/hyprlayer-app`

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the ed25519 private key (`~/.tauri/hyprlayer_updater.key`). |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The key's password — empty here, so set it to an empty string (or omit). |
| `RELEASES_REPO_TOKEN` | A PAT with `contents:write` on `BrightBlock/hyprlayer-releases`, so the release job can create the public Release across repos (the built-in `GITHUB_TOKEN` can't write to another repo). |
| `WEBSITE_REPO_TOKEN` | A PAT with `contents:write` on `jtsiros/brightblock.ai` (it's private), so `bump-website` can push the manifest. |

The signing keypair was generated with `pnpm tauri signer generate`. **Back up
the private key** — if it's lost, existing installs can no longer verify
updates and you'd have to ship a new public key in a fresh build.

### 2. Manifest path on brightblock.ai

The Astro site serves `public/` at the root, so the first release creates
`public/hyprlayer/updates/{latest,changelog}.json`, reachable at
`https://brightblock.ai/hyprlayer/updates/latest.json` — which is the endpoint
configured in `tauri.conf.json`. No site change needed; Vercel deploys on push.

### 3. AUR access (Linux)

Already set up — no new key, no new account. `release-local.sh` pushes
`hyprlayer-desktop-bin` over SSH as the last step, authenticating with the
**`aur-ci` key in the 1Password SSH agent**. `~/.ssh/config` points `Host *` at
that agent, which is the same path `nexus` and GitHub already take, so AUR needs
no entry of its own:

```sh
ssh-add -l                       # aur-ci (ED25519) should be listed
ssh aur@aur.archlinux.org help   # prints the AUR command list
```

The agent prompts for approval on first use in a session, so an unattended `-y`
run needs 1Password already unlocked and the key approved. Override the target
with `HYPRLAYER_AUR_SSH` (e.g. an ssh_config alias) to pin a different identity.

The AUR repo is created by the first push — no separate "new package" step. The
script preflights auth and refuses to start without it; pass `--no-aur` to cut a
release without touching the AUR. AUR only accepts pushes to `master`, which the
script forces (`checkout -B master`) since a clone of a not-yet-created package
lands on whatever your local `init.defaultBranch` is.

If a push ever fails after the GitHub Release is live, the rendered files are
kept at `target/linux-release/aur/` — clone the AUR repo and push them by hand.

## Code signing

**macOS builds are Developer ID–signed and notarized** by `release-local.sh`, so
first-launch users get no Gatekeeper warning. **Windows is still unsigned**, so
SmartScreen shows an "unknown publisher" prompt (*More info → Run anyway*). The
in-app updater is independent of OS code signing — it verifies the separate
ed25519 signature baked into the app, regardless of Developer ID status.

### macOS (Developer ID + notarization)

One-time setup on the build Mac:

1. **Developer ID Application certificate** — create it in the
   [Apple Developer portal](https://developer.apple.com/account/resources/certificates/list)
   (certificate type **Developer ID Application** — *not* "Apple Distribution",
   which is App-Store-only). Generate the CSR from Keychain Access so the private
   key stays local, then double-click the downloaded `.cer` to install it into
   your **login** keychain. Confirm with `security find-identity -v -p codesigning`.
2. **App Store Connect API key** for notarization — generate a Team Key under
   [App Store Connect → Integrations](https://appstoreconnect.apple.com/access/integrations/api),
   download the one-time `.p8`, and save it (default `~/.tauri/AuthKey_*.p8`).

`release-local.sh` then signs + notarizes automatically — Tauri reads these from
the env during `tauri build`. They're wired as env-overridable defaults in the
script's config block:

| Variable | Holds |
| --- | --- |
| `APPLE_SIGNING_IDENTITY` | Developer ID identity string (`Developer ID Application: … (TEAMID)`) |
| `APPLE_API_KEY` | Notary key ID |
| `APPLE_API_ISSUER` | Notary issuer ID |
| `APPLE_API_KEY_PATH` | Path to the `.p8` notary key (the only secret — kept off the repo) |

Tauri enables the hardened runtime automatically and staples the notarization
ticket to the `.app` (and the updater tarball) during the build — no extra
entitlements needed for a standard webview app. A plain `pnpm tauri build` (for
local testing) stays ad-hoc-signed and skips notarization; only the release path
signs. Verify a cut build with `spctl -a -vvv -t install <app>` and
`xcrun stapler validate <dmg>`.

> **Back up** both the Developer ID `.p12` (export from Keychain Access) and the
> notary `.p8` — the `.p8` can only be downloaded once.

### Windows (not yet signed)

Still unsigned. To drop the SmartScreen prompt, add an Authenticode step with an
EV/OV (or Azure Trusted Signing) certificate to the Windows container leg
(`scripts/builders/windows/build.sh`), through `osslsigncode` or `jsign`.

### Linux (no OS signing)

Distro packages carry no OS-level code signature of ours. The `.deb` and `.rpm`
go up unsigned; the AUR package's integrity is the `sha256sum` the PKGBUILD pins
against the published `.deb`. There is no ed25519 signature either, because
nothing on Linux self-installs — see below.

## Build containers

`release-local.sh` builds the Windows and Linux legs in Docker containers on the
Mac, through `scripts/builders/run.sh <windows|linux> <source.tgz> <out-dir>`.
Both cross-compile at native arm64 speed, so neither needs Rosetta, and neither
holds a secret, a Git credential or the signing key.

| Leg | Image (`scripts/builders/`) | Produces |
| --- | --- | --- |
| Windows | `windows/`: `rust:<version>-slim-trixie` with cargo-xwin (the MSVC CRT and Windows SDK baked in), LLVM and Debian's NSIS | `target/windows-release/`: `Hyprlayer_<v>_x64-setup.exe`, `hyprlayer-server-x86_64-pc-windows-msvc.exe` |
| Linux | `linux/`: `rust:<version>-slim-bookworm` with an amd64 bookworm sysroot | `target/linux-release/`: the `.deb`, the `.rpm`, `hyprlayer-server-x86_64-unknown-linux-gnu`, `aur/` and `aur-server/` |

`run.sh` passes the Mac's `rustc` version and `package.json`'s pnpm version as
build args, so all three legs compile with the same toolchain. Each leg keeps
named volumes for its cargo target dir, registry, pnpm store and `~/.cache`, so a
release after the first rebuilds only the workspace crates; budget about 16 GB of
Docker disk. The image layers are cached too: after changing a Dockerfile's pinned
versions, rebuild it with `docker build --no-cache` so its downloads run again.

`run.sh linux <source.tgz> <out-dir> --server-only` runs the image's second
entrypoint, which builds only `hyprlayer-server` against the same volumes. Because
trees other than a release tag then build into `/target`, both entrypoints hold a
lock on it for the whole run and extract with `tar --touch`: cargo judges workspace
crates by source mtime, and a tarball's commit-time mtimes could otherwise pass
another tree's build off as current. `scripts/stage-server-binary.sh` uses it for
the Linux dev server, so a stage starts from the release's compiled dependencies.

**Memory.** The legs peak near 5.6 GB (Windows) and 4.2 GB (Linux). With Docker
Desktop at 12 GB or more, `release-local.sh` builds them at once; below that it
builds them one after the other and says so. It reads `MemTotal` from
`docker info`, which reports a little less than the setting (8 GB reads 7.75 GiB,
16 GB reads 15.59), so the cut-off is 11 GiB.

**Windows.** `makensis` crashes on macOS arm64, so the whole leg runs in the
container, where Debian's works. The installer matches an official one apart from
NSIS's stubs, which Debian builds with mingw (`uninstall.exe` is about 149 KB
rather than 79 KB). Tauri calls cross-compiling experimental and cannot
cross-build an MSI; the release ships NSIS only.

**Linux.** `mmdebstrap --variant=extract` unpacks the sysroot without running
anything. Debian's `makepkg` fails on these PKGBUILDs, so `run.sh` renders each
`.SRCINFO` with Arch's, in a two-second amd64 step (`scripts/builders/aur/`).

### Smoke tests (`--smoke`)

`release-local.sh --smoke` install-tests what it built before the `Proceed?`
prompt, and before publishing under `-y`. Any failure, including a Windows test
host that is unreachable or busy, stops the release with nothing published; the
rerun reuses every built artifact. The Windows host is checked up front
(`windows.sh --check`), so a stopped or busy one fails before anything builds.

- **Linux** (`scripts/builders/smoke/linux.sh`) needs only Docker. It installs the
  deb on Ubuntu 24.04 and Debian 13, the rpm on Fedora (`fedora:latest`, which
  floats) and the AUR package on Arch through `makepkg -si`, then launches the
  Arch install under Xvfb and checks that it reaches its server. The distros run
  as amd64 containers under emulation. Expect 6–18 minutes, mostly Ubuntu's apt
  mirror.
- **Windows** (`scripts/builders/smoke/windows.sh`) installs, launches, closes and
  uninstalls the build on a real Windows host over SSH (`HYPRLAYER_WIN_TEST_HOST`,
  default `nexus-win`), then puts back whatever install was there. It refuses to
  start while Hyprlayer or its server runs on the host, because installing closes
  the app and uninstalling kills the server along with any agents it hosts. For
  the run it moves the host's Hyprlayer profile
  (`%APPDATA%\ai.brightblock.hyprlayer` and `%LOCALAPPDATA%\ai.brightblock.hyprlayer`)
  into `%LOCALAPPDATA%\hl-smoke-backup`, so the build under test starts fresh and
  never opens the real database, settings or `hosts.json`; the CLI's shared config
  and skill stores under `%APPDATA%\hyprlayer` stay where they are. If the restore
  fails, the script says where the profile is and how to move it back. The app's
  quit confirmation keeps it open after WM_CLOSE, so the test stops it before
  uninstalling.

**The Windows test host.** `nexus-win` is the omarchy-managed `dockurr/windows`
guest on `nexus`: one VM and one disk (`~/.windows`), started with
`omarchy-windows-vm launch`. Nothing in the release scripts starts or stops it.
Launch it with `--keep-alive` before a `--smoke` run, or it stops when its RDP
window closes.

The omarchy compose publishes only `8006` and `3389`, both on loopback, so the
guest is reached at its docker-bridge address, which docker changes on every
container recreate. `~/.ssh/nexus-win-proxy.sh` resolves it on `nexus` per
connect and is the `nexus-win` `ProxyCommand`, so `ssh`/`scp nexus-win` work
unchanged; `Host nexus` carries `ControlMaster auto` to amortize the extra hop.
The `nexus-win` entry also needs `IdentitiesOnly yes`, because the 1Password
agent offers enough keys to trip sshd's `MaxAuthTries 6`, and
`HostKeyAlias nexus-win`.

```sh
omarchy-windows-vm launch --keep-alive   # start the guest for a smoke run
omarchy-windows-vm stop

# RDP from the Mac: tunnel first, then point Windows App at localhost:13389
ssh -N -L 13389:127.0.0.1:3389 nexus

# RDP from nexus itself
xfreerdp3 /u:jt /p:admin /v:127.0.0.1:3389 /cert:ignore /dynamic-resolution
```

Inside the guest:

- **OpenSSH Server**: `Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0`,
  then `Set-Service sshd -StartupType Automatic; Start-Service sshd`.
- **The public key goes in `C:\ProgramData\ssh\administrators_authorized_keys`**,
  not `~/.ssh/authorized_keys`, because the account is an Administrator. Its ACL
  must grant only `Administrators` and `SYSTEM`
  (`icacls <file> /inheritance:r /grant Administrators:F /grant SYSTEM:F`).
  A wrong ACL is a silent auth failure, not an error.
- **Leave sshd's `DefaultShell` alone — it must stay `cmd.exe`.** The smoke test
  addresses its working dir as `%LOCALAPPDATA%\Temp\hl-smoke`, and that variable
  only expands under `cmd`.

### Library target checks

`hyprlayer-models` and `hyprlayer-core` are checked on their own before anything
is published:

```sh
cargo check -p hyprlayer-models -p hyprlayer-core                                    # host target
cargo check -p hyprlayer-models -p hyprlayer-core --target x86_64-pc-windows-msvc   # Windows container
```

This is not redundant with the bundle build that follows it, though it looks it.
`hyprlayer-app` depends on core with `features = ["specta"]`, and cargo unifies
features across a workspace build — so `cargo test --workspace`, the gate's
`cargo nextest run --workspace` and `tauri build` only ever compile core *with*
specta. A headless embedder gets neither the app nor that feature, and this is the
only check that compiles core the way they will.

It runs in `release.sh`'s gate (the Mac) and at the start of both container legs:
`scripts/builders/linux/build.sh`, and `scripts/builders/windows/build.sh`, where
cargo-xwin supplies the MSVC CRT and Windows SDK that core's C dependencies need.

`x86_64-pc-windows-gnu` is not a support target and nothing gates on it, but
`cargo check -p hyprlayer-core --target x86_64-pc-windows-gnu` is a cheap way to
compile the Windows `cfg` arms from a Linux or macOS box.

### Linux packaging

The Linux leg cross-compiles against a Debian **bookworm** sysroot, so the ELF
needs glibc 2.34 at most and links WebKitGTK 4.1 and GTK 3 by soname. One binary
is wrapped as `.deb` and `.rpm` and repackaged for the AUR. The packages install
on Ubuntu 24.04, Debian 13, Fedora and Arch, which is what `--smoke` checks, and
the standalone server also starts on Ubuntu 22.04, Debian 12 and RHEL 9 remote
hosts.

Runtime dependencies are declared in three places, and they must stay in step:

| Package | Declared in | Names |
| --- | --- | --- |
| AUR | `packaging/aur/PKGBUILD.in` | `webkit2gtk-4.1`, `gtk3`, `git`, `xdg-utils`, `hicolor-icon-theme` |
| deb | `src-tauri/tauri.linux.conf.json` | `libwebkit2gtk-4.1-0`, `libgtk-3-0`, `git`, `xdg-utils` |
| rpm | `src-tauri/tauri.linux.conf.json` | `libwebkit2gtk-4.1.so.0()(64bit)`, `libgtk-3.so.0()(64bit)`, `git`, `xdg-utils` |

WebKitGTK pulls glib2/cairo/pango/gdk-pixbuf/libsoup3 transitively; GTK 3 is
named because the binary links it directly. `git` is spawned for every network
and branch operation, and `xdg-utils` backs `tauri-plugin-opener`. There's no
appindicator dependency — the app has no tray icon. The RPM side uses soname
`Requires` rather than package names because Fedora (`webkit2gtk4.1`) and
openSUSE (`libwebkit2gtk-4_1-0`) disagree on the latter.

The AUR package is `hyprlayer-desktop-bin` — a **binary** package, because the
source repo is private and there is nothing for an AUR user to build from. Its
`package()` unpacks the published `.deb`, which is transport only, with no Debian
runtime involved: the ELF was built against bookworm's libraries but links them by
soname and runs on Arch's newer ones. `--smoke` installs the AUR package on Arch and
launches it.

### How Linux updates work

They don't, in-app. Every Linux install now comes from a distro package that
pacman/apt/dnf owns, so nothing may write over `/usr/bin` — and the app can't
even tell which package manager it came from, since the AUR package is unpacked
from the `.deb` and inherits its `Deb` bundle stamp. `tauri-plugin-updater`
would happily try `pkexec dpkg -i` on an Arch box.

So `latest.json` deliberately carries **no `linux-x86_64` key**, and
`src/lib/updater.ts` takes a different path there: it compares the running
version against the newest entry in `changelog.json` and shows a notify-only
toast pointing at the package manager. That's discovery only — the install goes
through the distro's own verified channel, so the missing signature costs
nothing.

Consequence for the 0.5.6–0.5.7 AppImage users: they'll stop being offered
updates and need to reinstall from a package once. That's the whole migration.

## Local development

`check()` can't verify against a real signed release in `tauri dev`, so the
updater no-ops there. To exercise the Update pill UI without a backend, run dev
with `VITE_MOCK_UPDATE=1`.
