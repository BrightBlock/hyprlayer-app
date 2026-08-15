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
`scripts/release-local.sh`, which farms the two non-Mac bundles out to headless
builders over SSH and signs their output locally, so the ed25519 signing key
never leaves the Mac.

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
  ├─ win    (nexus-win) → Hyprlayer_<v>_x64-setup.exe  built unsigned, scp'd
  │                                                    back, ed25519-signed here
  ├─ linux  (nexus)     → Hyprlayer_<v>_amd64.deb + Hyprlayer-<v>-1.x86_64.rpm
  │                       + a rendered PKGBUILD/.SRCINFO (no signature — see below)
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

Both remote builders compile a clean `git archive` of the release tag shipped to
them — never their own working clone — so what they produce matches the tag byte
for byte. Neither holds any secret.

| Flag | Builds |
| --- | --- |
| *(none)* | macOS arm64 + Windows x64 + Linux x64 + AUR |
| `--no-linux` | macOS + Windows (implies `--no-aur`) |
| `--no-windows` | macOS + Linux + AUR |
| `--no-aur` | everything, but don't push the AUR package |
| `--mac-only` | macOS only |

A platform left out is simply absent from `latest.json`, so its installs see
"no update" until the next all-platform release — they are never served a
mismatched build.

Update packages are **ed25519-signed**; the public key is baked into the app
(`src-tauri/tauri.conf.json` → `plugins.updater.pubkey`) and verifies every
download before install. The app also reads `changelog.json` from the same
host to show release notes — non-fatal if it's missing, except on Linux where
it *is* the update signal (see "How Linux updates work").

## Cutting a release

```sh
./scripts/release.sh patch     # or minor / major / an explicit version
```

The script gates on `changelog.json`: it scaffolds a stub entry for the new
version if missing and refuses to proceed until you've written a `summary`
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

Still unsigned. To drop the SmartScreen prompt, add a signing step with an EV/OV
(or Azure Trusted Signing) certificate to `scripts/release-win-remote.ps1`.

### Linux (no OS signing)

Distro packages carry no OS-level code signature of ours. The `.deb` and `.rpm`
go up unsigned; the AUR package's integrity is the `sha256sum` the PKGBUILD pins
against the published `.deb`. There is no ed25519 signature either, because
nothing on Linux self-installs — see below.

## Remote builders

Both are headless compile boxes reached over SSH. They need no secrets, no Git
credentials, and no signing key.

| Builder | Host (override) | Helper | Produces |
| --- | --- | --- | --- |
| Windows | `nexus-win` (`HYPRLAYER_WIN_SSH_HOST`) | `scripts/release-win-remote.ps1` | `*-setup.exe` |
| Linux | `nexus` (`HYPRLAYER_LINUX_SSH_HOST`) | `scripts/release-linux-remote.sh` | `*.deb`, `*.rpm`, `PKGBUILD` + `.SRCINFO` |

Windows also has a CI fallback: `HYPRLAYER_WIN_BUILDER=actions` runs
`.github/workflows/windows-build.yml` on a `windows-latest` runner and downloads
the artifact instead of using SSH. Linux has no such fallback yet — build hosted
if you need one, or run with `--no-linux`.

**Linux builder prerequisites** (Arch package names; the helper resolves
toolchains through a login shell, so mise/nvm/volta shims are fine):

```sh
# toolchains
rustup default stable
# system deps for Tauri 2
sudo pacman -S --needed webkit2gtk-4.1 gtk3 librsvg base-devel
```

Both bundlers are pure Rust — no `dpkg`, no `rpmbuild`, no FUSE. `makepkg`
(from `pacman`, always present) renders the `.SRCINFO`, which is why that file
is generated on the builder rather than on the Mac.

### Linux packaging

The builder is an **Arch** box, and Arch is the primary Linux target, so the ELF
it produces links Arch's `webkit2gtk-4.1` — exactly what the AUR package wants.
The same binary is wrapped as `.deb` and `.rpm` for everyone else, which sets a
glibc floor: those install cleanly on Ubuntu 24.04, Debian 13 and Fedora 40 and
newer, but not on older LTS releases. Targeting those properly means building in
an old-glibc container; that isn't wired up.

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
`package()` unpacks the published `.deb`; since that `.deb` was itself built on
Arch, this is transport only, with no Debian runtime involved.

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
