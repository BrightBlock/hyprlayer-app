#!/usr/bin/env bash
# Runs on a Linux box with a live Hyprland session: $1 = wayland|x11, $2 = out dir. Starts the spike in --host --selftest and grims its window.
set -u
MODE="$1"; OUT="$2"; mkdir -p "$OUT"
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export HYPRLAND_INSTANCE_SIGNATURE=$(ls "$XDG_RUNTIME_DIR/hypr" | head -1)
if [[ "$MODE" == x11 ]]; then
  export DISPLAY=:0; unset WAYLAND_DISPLAY; export WAYLAND_DISPLAY=
else
  export WAYLAND_DISPLAY=wayland-1; unset DISPLAY
fi
cd ~/g1-spike
./gpui-spike --host --selftest "$OUT" > "$OUT/stderr.log" 2>&1 &
PID=$!
sleep 14
GEOM=$(hyprctl clients -j | python3 -c "
import json,sys
for c in json.load(sys.stdin):
    if c.get('pid')==$PID or c.get('class','').lower().startswith('gpui'):
        print('%d,%d %dx%d'%(c['at'][0],c['at'][1],c['size'][0],c['size'][1])); break")
echo "geom=$GEOM xwayland=$(hyprctl clients -j | python3 -c "
import json,sys
print([c.get('xwayland') for c in json.load(sys.stdin) if c.get('pid')==$PID])")" > "$OUT/window.txt"
[[ -n "$GEOM" ]] && grim -g "$GEOM" "$OUT/window.png"
wait $PID
