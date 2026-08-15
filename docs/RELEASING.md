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

./scripts/release.sh patch        # bump version + tag, locally (no push)
        ▼
./scripts/release-local.sh        # build + sign + publish, all from the Mac
  ├─ mac    (here)      → Hyprlayer_<v>_aarch64.dmg + Hyprlayer.app.tar.gz(.sig)
  │                       Developer ID–signed + notarized during the build
  ├─ win    (nexus-win) → Hyprlayer_<v>_x64-setup.exe   ─┐ built unsigned,
  ├─ linux  (nexus)     → Hyprlayer_<v>_amd64.AppImage  ─┘ scp'd back,
  │                                                        ed25519-signed here
  ├─ release      → GitHub Release on hyprlayer-releases (public) w/ assets
  ├─ manifest     → latest.json + changelog.json → jtsiros/hyprlayer.dev
  │                 (darwin-aarch64 + windows-x86_64 + linux-x86_64) → Vercel
  └─ cask         → BrightBlock/homebrew-tap version + dmg sha256 bump
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
| *(none)* | macOS arm64 + Windows x64 + Linux x64 |
| `--no-linux` | macOS + Windows |
| `--no-windows` | macOS + Linux |
| `--mac-only` | macOS only |

A platform left out is simply absent from `latest.json`, so its installs see
"no update" until the next all-platform release — they are never served a
mismatched build.

Update packages are **ed25519-signed**; the public key is baked into the app
(`src-tauri/tauri.conf.json` → `plugins.updater.pubkey`) and verifies every
download before install. The app also reads `changelog.json` from the same
host to show release notes — non-fatal if it's missing.

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

AppImages carry no OS-level code signature, so there's nothing to add here — the
ed25519 updater signature is the only one, and it's applied on the Mac like the
Windows installer's.

## Remote builders

Both are headless compile boxes reached over SSH. They need no secrets, no Git
credentials, and no signing key.

| Builder | Host (override) | Helper | Produces |
| --- | --- | --- | --- |
| Windows | `nexus-win` (`HYPRLAYER_WIN_SSH_HOST`) | `scripts/release-win-remote.ps1` | `*-setup.exe` |
| Linux | `nexus` (`HYPRLAYER_LINUX_SSH_HOST`) | `scripts/release-linux-remote.sh` | `*.AppImage` |

Windows also has a CI fallback: `HYPRLAYER_WIN_BUILDER=actions` runs
`.github/workflows/windows-build.yml` on a `windows-latest` runner and downloads
the artifact instead of using SSH. Linux has no such fallback yet — build hosted
if you need one, or run with `--no-linux`.

**Linux builder prerequisites** (Arch package names; the helper resolves
toolchains through a login shell, so mise/nvm/volta shims are fine):

```sh
# toolchains
rustup default stable
# system deps for Tauri 2 + the AppImage bundler
sudo pacman -S --needed webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg \
                        base-devel curl openssl fuse2
```

The AppImage bundler downloads `linuxdeploy` on first use; the helper exports
`APPIMAGE_EXTRACT_AND_RUN=1` so it works without a mountable FUSE device.

### How Linux updates install

Unlike macOS (a `.app.tar.gz` unpacked over the bundle) and Windows (an NSIS
installer re-run), the Linux updater payload **is the AppImage itself**:
`tauri-plugin-updater` writes the downloaded bytes straight over the running
file. Two consequences worth knowing:

- `latest.json`'s `linux-x86_64` URL points at the raw `.AppImage`, not a
  tarball. That's why the release uploads `*.AppImage` + `*.AppImage.sig`.
- The AppImage must sit on a **writable** filesystem, and the updater's temp dir
  must be on the **same mount point** (it renames the old file aside as a
  backup). An AppImage on a read-only mount or a different device than `/tmp`
  fails the install with `TempDirNotOnSameMountPoint`.

## Local development

`check()` can't verify against a real signed release in `tauri dev`, so the
updater no-ops there. To exercise the Update pill UI without a backend, run dev
with `VITE_MOCK_UPDATE=1`.
