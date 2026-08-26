#!/usr/bin/env node
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
