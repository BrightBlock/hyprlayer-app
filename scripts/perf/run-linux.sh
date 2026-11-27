#!/usr/bin/env bash
# Tauri baseline on Linux: 7 runs of the baseline scenario against a kept fixture server.
# Env: PERF_SESSION=wayland|x11 (default wayland), PERF_RUNS, PERF_SCENARIO, PERF_FIRST.
# Needs a graphical session; over ssh it borrows the logged-in one.
# Idle off for the whole run: a blanked display stalls the webview's frame clock.
if [ -z "${PERF_INHIBITED:-}" ] && command -v systemd-inhibit >/dev/null; then
  PERF_INHIBITED=1 exec systemd-inhibit --what=idle:sleep --why="hyprlayer perf run" "$0" "$@"
fi
source "$(dirname "$0")/common.sh"

SESSION="${PERF_SESSION:-wayland}"
UIDN="$(id -u)"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$UIDN}"
export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=$XDG_RUNTIME_DIR/bus}"
if [ -z "${WAYLAND_DISPLAY:-}" ]; then
  sock="$(cd "$XDG_RUNTIME_DIR" && ls wayland-* 2>/dev/null | grep -v '\.lock$' | head -1 || true)"
  [ -n "$sock" ] && export WAYLAND_DISPLAY="$sock"
fi
if [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] && [ -d "$XDG_RUNTIME_DIR/hypr" ]; then
  export HYPRLAND_INSTANCE_SIGNATURE="$(ls -t "$XDG_RUNTIME_DIR/hypr" | head -1)"
fi
if [ "$SESSION" = x11 ]; then
  export GDK_BACKEND=x11
  [ -n "${DISPLAY:-}" ] || export DISPLAY=:0
fi

hypr() { command -v hyprctl >/dev/null && [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; }
ANIMATIONS=""

# Compositor animations off for the run, restored after; a tiled window would also ignore the resize drag.
if hypr; then
  ANIMATIONS="$(hyprctl getoption animations:enabled -j | node -e 'console.log(JSON.parse(require("fs").readFileSync(0,"utf8")).int)')"
  hyprctl keyword animations:enabled 0 >/dev/null
fi
restore_session() { if hypr && [ -n "$ANIMATIONS" ]; then hyprctl keyword animations:enabled "$ANIMATIONS" >/dev/null; fi; }

# Floats the app's window at a fixed size so the resize drag is the client's, not the tiler's.
post_launch() {
  hypr || return 0
  hyprctl dispatch dpms on >/dev/null
  local addr=""
  for _ in $(seq 1 120); do
    addr="$(hyprctl clients -j | node -e 'const c=JSON.parse(require("fs").readFileSync(0,"utf8")).find(c=>c.pid==process.argv[1]);if(c)console.log(c.address+" "+c.floating)' "$APP_PID" 2>/dev/null || true)"
    [ -n "$addr" ] && break
    sleep 0.25
  done
  [ -n "$addr" ] || { log "no window found for pid $APP_PID"; return 0; }
  set -- $addr
  [ "$2" = true ] || hyprctl dispatch togglefloating "address:$1" >/dev/null
  hyprctl dispatch resizewindowpixel "exact ${PERF_WIDTH:-1500} ${PERF_HEIGHT:-900},address:$1" >/dev/null
  hyprctl dispatch movewindowpixel "exact 50 50,address:$1" >/dev/null
}

print_environment() {
  local gpu webkit monitors
  gpu="$(lspci 2>/dev/null | grep -iE 'vga|3d|display' | sed 's/^[^ ]* //' | tr '\n' ';')"
  webkit="$(pkg-config --modversion webkit2gtk-4.1 2>/dev/null || echo unknown)"
  monitors="[]"
  hypr && monitors="$(hyprctl monitors -j)"
  GPU="$gpu" WEBKIT="$webkit" MONITORS="$monitors" SESSION="$SESSION" node -e '
const { execSync } = require("child_process");
const sh = (c) => { try { return execSync(c, { encoding: "utf8" }).trim(); } catch { return "unknown"; } };
const nvidia = sh("test -d /sys/module/nvidia && echo yes || echo no");
console.log(JSON.stringify({
  machine: sh("hostname"), kernel: sh("uname -r"), os: sh("grep PRETTY_NAME /etc/os-release | cut -d= -f2"),
  session: process.env.SESSION, compositor: process.env.HYPRLAND_INSTANCE_SIGNATURE ? "Hyprland " + sh("hyprctl version -j | node -e \"console.log(JSON.parse(require(\\\"fs\\\").readFileSync(0,\\\"utf8\\\")).tag)\"") : (process.env.XDG_CURRENT_DESKTOP || "unknown"),
  waylandDisplay: process.env.WAYLAND_DISPLAY || null, gpu: process.env.GPU,
  nvidiaDriver: nvidia === "yes" ? sh("nvidia-smi --query-gpu=driver_version --format=csv,noheader") : null,
  webkitgtk: process.env.WEBKIT,
  dmabufRendererOff: nvidia === "yes" && process.env.WEBKIT !== "unknown" && process.env.WEBKIT.split(".").map(Number).join(",") < "2,54,2",
  governor: sh("cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"), cores: sh("nproc"),
  load: sh("cut -d\" \" -f1-3 /proc/loadavg"), monitors: JSON.parse(process.env.MONITORS).map((m) => ({ name: m.name, width: m.width, height: m.height, refreshRate: m.refreshRate, scale: m.scale })),
  animationsDisabledForRun: true,
}, null, 2));'
}

run_all
