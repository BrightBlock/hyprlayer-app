#!/usr/bin/env node
// Render a GitHub Release body: "What's new" from changelog.json, then a
// compact per-platform install block derived from the release's own assets.
//
// The changelog entry already feeds the in-app Update card and the changelog on
// brightblock.ai, so the release page says exactly what the app says — one
// source of truth, no second place to keep in sync.
//
// The install block is deliberately short and links to brightblock.ai for the
// rest. It's driven by the asset filenames rather than a set of flags so that
// this one renderer produces byte-identical output whether it's called by
// release-local.sh at publish time (passing the files it just built) or by a
// backfill that passes what `gh release view` reports. Assets that aren't
// installable by hand — the updater tarball, the .sig sidecars — are ignored.
//
// Usage:
//   node scripts/release-notes.mjs 0.5.11
//   node scripts/release-notes.mjs 0.5.11 Hyprlayer_0.5.11_aarch64.dmg …
//
// With no assets, only "What's new" is printed. Exits 1 (message on stderr)
// when the version has no changelog entry or the entry is empty.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { basename, dirname, join } from "node:path";

const [rawVersion, ...assetPaths] = process.argv.slice(2);
const version = (rawVersion ?? "").replace(/^v/, "");
if (!version) {
  console.error("usage: release-notes.mjs <version> [asset …]");
  process.exit(2);
}

const SITE = "https://brightblock.ai/hyprlayer";
// Overridable so the AUR package name has one owner (release-local.sh's config
// block) rather than being spelled twice.
const AUR_PKGNAME = process.env.HYPRLAYER_AUR_PKGNAME || "hyprlayer-desktop-bin";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
let versions;
try {
  versions = JSON.parse(readFileSync(join(repoRoot, "changelog.json"), "utf8")).versions;
} catch (e) {
  console.error(`✗ could not read changelog.json: ${e.message}`);
  process.exit(1);
}

const entry = (Array.isArray(versions) ? versions : []).find((v) => v?.version === version);
if (!entry) {
  console.error(`✗ changelog.json has no entry for ${version}`);
  process.exit(1);
}

const summary = typeof entry.summary === "string" ? entry.summary.trim() : "";
const notes = (Array.isArray(entry.notes) ? entry.notes : [])
  .filter((n) => typeof n === "string" && n.trim().length > 0)
  .map((n) => n.trim());

if (!summary && notes.length === 0) {
  console.error(`✗ changelog.json entry for ${version} has no summary or notes`);
  process.exit(1);
}

const out = ["## What's new"];
if (summary) out.push("", summary);
if (notes.length) out.push("", ...notes.map((n) => `- ${n}`));

// ─── install block, keyed off what actually shipped ──────────────────
const names = assetPaths.map((p) => basename(p)).filter((n) => !n.endsWith(".sig"));
const find = (re) => names.find((n) => re.test(n));
const dmg = find(/\.dmg$/);
const exe = find(/-setup\.exe$/);
const deb = find(/\.deb$/);
const rpm = find(/\.rpm$/);
const appimage = find(/\.AppImage$/);

const install = [];
if (dmg) {
  install.push(
    `- **macOS (Apple Silicon)** — download \`${dmg}\` and drag it to /Applications. ` +
      `Signed with a Developer ID and notarized by Apple, so it opens with no Gatekeeper warning.`,
  );
}
if (exe) {
  install.push(
    `- **Windows (x64)** — run \`${exe}\`. It isn't code-signed yet, so SmartScreen shows an ` +
      `"unknown publisher" prompt — choose **More info → Run anyway**.`,
  );
}
if (deb || rpm) {
  const cmds = [`\`paru -S ${AUR_PKGNAME}\` (AUR)`];
  if (deb) cmds.push(`\`sudo apt install ./${deb}\``);
  if (rpm) cmds.push(`\`sudo dnf install ./${rpm}\``);
  install.push(
    `- **Linux (x64)** — ${cmds.join(", ")}. Pulls in WebKitGTK 4.1 and GTK 3; the ` +
      `.deb/.rpm are built on Arch, so they want a current glibc (Ubuntu 24.04, Debian 13, Fedora 40 or newer).`,
  );
} else if (appimage) {
  install.push(
    `- **Linux (x64)** — download \`${appimage}\`, \`chmod +x\` it, and run it. Needs WebKitGTK 4.1.`,
  );
}

if (install.length) {
  out.push("", "## Install", "", ...install, "", `More at ${SITE}.`);
  out.push(
    "",
    deb || rpm
      ? "Updates: macOS and Windows self-update in-app. Linux installs belong to your " +
          "package manager, so update them the usual way (`paru -Syu`, `apt`, `dnf`)."
      : "Updates: the app self-updates from the in-app update prompt.",
  );
}

process.stdout.write(out.join("\n") + "\n");
