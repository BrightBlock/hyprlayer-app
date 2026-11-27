#!/usr/bin/env bash
# Runs on a Linux box with a live Hyprland session: $1 = wayland|x11, $2 = out dir.
# Starts the spike in --host --selftest, waits for its report, then grims the window and reads the Wayland clipboard.
set -u
MODE="$1"; OUT="$2"; mkdir -p "$OUT"
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export HYPRLAND_INSTANCE_SIGNATURE=$(ls "$XDG_RUNTIME_DIR/hypr" | head -1)
if [[ "$MODE" == x11 ]]; then export DISPLAY=:0; unset WAYLAND_DISPLAY; else export WAYLAND_DISPLAY=wayland-1; unset DISPLAY; fi
cd ~/g1-spike
HYPRLAYER_SPIKE_HOLD=25 ./gpui-spike --host --selftest "$OUT" > "$OUT/stderr.log" 2>&1 &
PID=$!
for _ in $(seq 1 120); do [[ -f "$OUT/report.json" ]] && break; sleep 1; done
GEOM=$(hyprctl clients -j | python3 -c "
import json,sys
for c in json.load(sys.stdin):
    if c.get('pid')==$PID:
        print('%d,%d %dx%d'%(c['at'][0],c['at'][1],c['size'][0],c['size'][1])); break")
echo "geom=$GEOM xwayland=$(hyprctl clients -j | python3 -c "
import json,sys
print([c.get('xwayland') for c in json.load(sys.stdin) if c.get('pid')==$PID])")" > "$OUT/window.txt"
# grim and wl-paste are Wayland clients even when the spike runs under X11.
[[ -n "$GEOM" ]] && WAYLAND_DISPLAY=wayland-1 grim -g "$GEOM" "$OUT/window.png" 2> "$OUT/grim.log"
WAYLAND_DISPLAY=wayland-1 wl-paste -n > "$OUT/wl-paste.txt" 2> "$OUT/wl-paste.err"
# A compositor-delivered Ctrl+C carries a real input serial, unlike the in-app synthetic keystroke.
hyprctl dispatch focuswindow "pid:$PID" > /dev/null
hyprctl dispatch sendshortcut "CTRL, C, pid:$PID" > "$OUT/sendshortcut.log" 2>&1
sleep 1
WAYLAND_DISPLAY=wayland-1 wl-paste -n > "$OUT/wl-paste-after-shortcut.txt" 2>> "$OUT/wl-paste.err"
wait $PID
