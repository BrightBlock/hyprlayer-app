#!/usr/bin/env node
// Builds hyprlayer-server and stages it as a Tauri sidecar, so every bundle
// carries it next to the app binary — which is exactly where
// supervisor.rs::server_binary() looks. Without this the desktop ships with no
// server and cannot start (ADR-0009, plan Phase 6).

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const run = (cmd, args, opts = {}) =>
  execFileSync(cmd, args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"], ...opts });

// Tauri requires the sidecar filename to carry the target triple; it strips it
// when bundling.
const triple =
  process.env.HYPRLAYER_SIDECAR_TARGET ||
  run("rustc", ["-vV"]).match(/^host:\s*(\S+)$/m)?.[1];
if (!triple) {
  console.error("build-server-sidecar: could not determine the host triple from `rustc -vV`");
  process.exit(1);
}

// dev stages a debug build so `tauri dev` (which also resolves externalBin)
// doesn't pay for a release compile.
const debug = process.argv.includes("--debug");
const cross = Boolean(process.env.HYPRLAYER_SIDECAR_TARGET);
const args = ["build", "-p", "hyprlayer-server"];
if (!debug) args.push("--release");
if (cross) args.push("--target", triple);

console.log(`build-server-sidecar: cargo ${args.join(" ")}`);
run("cargo", args, { stdio: "inherit" });

const exe = triple.includes("windows") ? ".exe" : "";
const targetDir = process.env.CARGO_TARGET_DIR || join(root, "target");
const built = join(targetDir, ...(cross ? [triple] : []), debug ? "debug" : "release", `hyprlayer-server${exe}`);
const staged = join(root, "src-tauri", "binaries", `hyprlayer-server-${triple}${exe}`);

mkdirSync(dirname(staged), { recursive: true });
copyFileSync(built, staged);
console.log(`build-server-sidecar: staged ${staged}`);
