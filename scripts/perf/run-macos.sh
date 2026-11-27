#!/usr/bin/env bash
# Tauri baseline on macOS: the same runs as run-linux.sh. Keep the window visible and the display awake.
source "$(dirname "$0")/common.sh"

caffeinate -dimsu -w $$ &

print_environment() {
  node -e '
const { execSync } = require("child_process");
const sh = (c) => { try { return execSync(c, { encoding: "utf8" }).trim(); } catch { return "unknown"; } };
console.log(JSON.stringify({
  machine: sh("sysctl -n hw.model") + " " + sh("sysctl -n machdep.cpu.brand_string"), kernel: sh("uname -r"), os: sh("sw_vers -productVersion"),
  session: "aqua", compositor: "WindowServer", gpu: sh("system_profiler SPDisplaysDataType | grep -E \"Chipset Model|Resolution|Refresh\" | tr -s \" \" | tr \"\\n\" \";\""),
  webkit: sh("defaults read /System/Library/Frameworks/WebKit.framework/Resources/version_info CFBundleShortVersionString || defaults read /System/Cryptexes/OS/System/Library/Frameworks/WebKit.framework/Resources/Info CFBundleVersion"),
  load: sh("sysctl -n vm.loadavg"), cores: sh("sysctl -n hw.ncpu"),
}, null, 2));'
}

run_all
