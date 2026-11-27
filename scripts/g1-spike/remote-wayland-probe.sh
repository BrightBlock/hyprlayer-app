#!/usr/bin/env bash
# Wayland-only probe: protocol log for text-input-v3 (IME) and a compositor-sent Ctrl+C for clipboard.
set -u
OUT="$1"; mkdir -p "$OUT"
export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-1
export HYPRLAND_INSTANCE_SIGNATURE=$(ls "$XDG_RUNTIME_DIR/hypr" | head -1)
cd ~/g1-spike
WAYLAND_DEBUG=1 HYPRLAYER_SPIKE_HOLD=40 ./gpui-spike --host --selftest "$OUT" > "$OUT/stderr.log" 2> "$OUT/wayland-debug.log" &
PID=$!
for _ in $(seq 1 120); do [[ -f "$OUT/report.json" ]] && break; sleep 1; done
hyprctl dispatch "hl.dsp.focus({ window = \"pid:$PID\" })" > "$OUT/focus.log" 2>&1
for form in \
  "hl.dsp.send_shortcut({ mods = \"CTRL\", key = \"C\", window = \"pid:$PID\" })" \
  "hl.dsp.send_shortcut(\"CTRL\", \"C\", \"pid:$PID\")" \
  "hl.dsp.send_shortcut(\"CTRL, C, pid:$PID\")"; do
  echo "== $form" >> "$OUT/sendshortcut.log"
  hyprctl dispatch "$form" >> "$OUT/sendshortcut.log" 2>&1
  sleep 1
  echo "paste=[$(wl-paste -n 2>&1)]" >> "$OUT/sendshortcut.log"
done
wait $PID
grep -oE "zwp_text_input_manager_v3[^ ]*|zwp_text_input_v3@[0-9]+\.(enable|commit|set_content_type|set_cursor_rectangle)|wl_data_device@[0-9]+\.set_selection|zwp_primary_selection_device_v1@[0-9]+\.set_selection" "$OUT/wayland-debug.log" | sort | uniq -c > "$OUT/protocol-summary.txt"
