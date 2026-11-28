//! Stand-ins for PTY claude and codex that run the hooks their launch injected, and drivers for
//! the agent-side faces (`hook`, `ctl`, `mcp`) under an agent's identity.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use hyprlayer_core::models::{AgentControlLevel, PtyLaunchSession, PtyLaunched};
use hyprlayer_core::settings::AppSettings;
use hyprlayer_server::client::{RpcClient, TerminalStream};
use hyprlayer_server::rpc::Request;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::{server_binary, Journey, Sandbox};

const FACE_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_REPLY: Duration = Duration::from_secs(30);

/// PTY claude. Logs to `agent-control/<$HYPRLAYER_TERMINAL_ID, else session-<id>>/`: `argv` (one
/// per line), `env` (`HYPRLAYER_*`), `size` (`stty size` at launch), copies of the
/// `--settings`/`--mcp-config` files, `hooks.log`
/// (`<Event> exit=<code>`) and `stdin.log` (each line as read). Each hook runs the command
/// `--settings` names for its event, so a launch without injected hooks fires none. `SessionStart`
/// fires at launch; stdin lines: `work` (prompt, Bash tool, `Stop` replying `done: work`), `hold`
/// (`UserPromptSubmit` alone, so it stays working), `ask` (`PermissionRequest` for `rm -rf build`),
/// `question` (`AskUserQuestion`), `tools` (a parallel Read's `PreToolUse` and `PostToolUse` alone),
/// `approve` (the asked tool's `PostToolUse`, then `Stop`), `clear` (`SessionEnd`, then
/// `SessionStart` as `<id>-cleared`), `bad` (malformed, non-object and empty payloads), `fail`
/// (`StopFailure` for an expired login, in place of `Stop`), `exit` (`SessionEnd`, then exit),
/// `write <path>` (writes it, then a `Write`'s `PostToolUse` with the relative path and `cwd`),
/// `shell <path> <branch> [date]` (a plan naming `branch`, through a redirect, so no hook fires),
/// `cd <dir>`, and `worktree <name>...` (one `WorktreeCreate` per name, all at once, each cut off
/// at its configured timeout and logged as `WorktreeCreate <name> exit=<code> path=<last stdout
/// line>`; for a single name the stub then moves into that path and logs `cwd <dir>`),
/// `worktree-remove` (back to the launch dir, then `WorktreeRemove` for the worktree it left) and
/// `subagent-stop <id> [dirty]` (`SubagentStop` for `agent-<id>`'s worktree, after writing
/// `changed.txt` there when `dirty`).
/// `--stub-trust` first shows claude's folder-trust prompt, firing no hook, and reads one line:
/// `trust` goes on to the launch; anything else exits, as claude's default "No, exit" does.
/// Without `--stub-tui` it paints claude's empty input box at launch and after each line, a
/// permission or question prompt after `ask` or `question` (a lone Esc cancels it with no hook, as
/// claude 2.1.332 does), the `/model` picker after `picker`, and
/// nothing after `tools`. The statusLine
/// `--settings` names runs on `status` (its stdout to `statusline.out`, its exit to
/// `statusline.log`), on shift+tab (`ESC [ Z`: the next mode of the manual → accept edits → plan →
/// auto ring, whose footer the box shows from then on, painted only after the statusLine as claude's
/// launch paints it) and after `switch` (`PostModelSwitch` to sonnet). Its context reads null until
/// the first `work`. Footers place their words with column moves, as claude 2.1.326 paints them.
///
/// `--stub-tui` makes it a raw-mode TUI painted as claude 2.1.322 is: an input box (its placeholder
/// dim while empty), a footer, and
/// `/model`, `/effort` and `/plan`. The `/model` picker's five rows and "More models" wrap on ↑/↓,
/// and its footer reads "Enter to expand" while "More models" is highlighted (logged to `tui.log`).
/// `s` fires `PreModelSwitch` (its stdout decides `--stub-dialog`'s "Switch model?", which
/// `--stub-ignore-allow` shows anyway), then `PostModelSwitch`, the `❯ /model` note and the
/// statusLine; in plan mode a switch to Haiku is followed by claude's own `auto` switch to Sonnet.
/// `--stub-silent-switch` closes it with neither; `--stub-narrow` wraps the pickers' footers as
/// claude does in a narrow terminal. `--stub-330` paints the pickers as logged-out claude 2.1.330
/// does: family-label rows that name the model in their hint, an effort row, `Enter to save`
/// footers, and under `--stub-narrow` each hint's price on a row of its own. The
/// `/effort` slider stops at the ends of ←/→ and `s` reruns the statusLine. Every byte it reads is
/// logged as `<screen> <hex>` to `keys.log`; a digit, an Enter in a picker or a Tab in the slider
/// goes to `traps.log` too, as it would save a claude default. `tui.log` records the switch path.
pub const PTY_CLAUDE: &str = r#"#!/bin/sh
base="$(dirname "$0")/../agent-control"
sid=""; settings=""; mcp=""; prev=""
tui=""; dialog=""; ignore_allow=""; silent=""; narrow=""; v330=""; trust=""
for a in "$@"; do
  case "$prev" in
    --session-id|--resume) sid="$a" ;;
    --settings) settings="$a" ;;
    --mcp-config) mcp="$a" ;;
  esac
  case "$a" in
    --stub-tui) tui=1 ;;
    --stub-dialog) dialog=1 ;;
    --stub-ignore-allow) ignore_allow=1 ;;
    --stub-silent-switch) silent=1 ;;
    --stub-narrow) narrow=1 ;;
    --stub-330) v330=1 ;;
    --stub-trust) trust=1 ;;
  esac
  prev="$a"
done
dir="$base/${HYPRLAYER_TERMINAL_ID:-session-$sid}"
mkdir -p "$dir"
home="$PWD"
for a in "$@"; do printf '%s\n' "$a"; done > "$dir/argv"
env | grep -E '^(HYPRLAYER_|CLAUDE)' > "$dir/env"
printf '%s\n' "$$" > "$dir/pid"
stty size > "$dir/size" 2>/dev/null
# The exit watcher deletes the launch dir, so keep what it held.
[ -f "$settings" ] && cp "$settings" "$dir/settings.json"
[ -f "$mcp" ] && cp "$mcp" "$dir/mcp.json"
hook_command() {
  [ -f "$settings" ] && sed -n "s/.*\"command\":\"\([^\"]*hook claude $1\)\".*/\1/p" "$settings"
}
hook() {
  cmd=$(hook_command "$1")
  [ -n "$cmd" ] || return 0
  printf '%s' "$2" | sh -c "$cmd" >/dev/null 2>&1
  printf '%s exit=%s\n' "$1" "$?" >> "$dir/hooks.log"
}
ev() { printf '{"session_id":"%s","hook_event_name":"%s"%s}' "$sid" "$1" "$2"; }
worktree_create() {
  cmd=$(hook_command WorktreeCreate)
  [ -n "$cmd" ] || return 0
  secs=$(sed -n 's/.*"command":"[^"]*hook claude WorktreeCreate","timeout":\([0-9]*\).*/\1/p' "$settings")
  out="$dir/worktree-$1.out"
  ev WorktreeCreate ",\"cwd\":\"$PWD\",\"name\":\"$1\"" | sh -c "$cmd" > "$out" 2>/dev/null &
  pid=$!
  # Off the PTY, so a sleeping watchdog never holds the terminal open.
  ( sleep "${secs:-600}"; kill "$pid" ) </dev/null >/dev/null 2>&1 &
  dog=$!
  wait "$pid"
  code=$?
  kill "$dog" 2>/dev/null
  printf 'WorktreeCreate %s exit=%s path=%s\n' "$1" "$code" "$(tail -n 1 "$out")" >> "$dir/hooks.log"
}
model="claude-haiku-5-5"; effort="low"; mode=0; moded=""; used=""; esc=$(printf '\033')
footer() {
  case $mode in
    0) printf '\033[3G⏸\033[5Gmanual\033[12Gmode\033[17Gon (shift+tab to cycle)' ;;
    1) printf '\033[3G⏵⏵\033[6Gaccept\033[13Gedits\033[19Gon (shift+tab to cycle)' ;;
    2) printf '\033[3G⏸\033[5Gplan\033[10Gmode\033[15Gon (shift+tab to cycle)' ;;
    3) printf '\033[3G⏵⏵\033[6Gauto\033[11Gmode\033[16Gon (shift+tab to cycle)' ;;
  esac
}
statusline() {
  cmd=""
  [ -f "$settings" ] && cmd=$(sed -n 's/.*"command":"\([^"]*statusline claude\)".*/\1/p' "$settings")
  [ -n "$cmd" ] || return 0
  if [ -n "$used" ]; then
    ctx='{"total_input_tokens":53568,"total_output_tokens":7,"context_window_size":1000000,"current_usage":{"input_tokens":2,"output_tokens":7,"cache_creation_input_tokens":25033,"cache_read_input_tokens":28533},"used_percentage":5,"remaining_percentage":95}'
  else
    ctx='{"total_input_tokens":0,"total_output_tokens":0,"context_window_size":1000000,"current_usage":null,"used_percentage":null,"remaining_percentage":null}'
  fi
  printf '{"session_id":"%s","cwd":"%s","model":{"id":"%s","display_name":"Stub"},"workspace":{"current_dir":"%s","project_dir":"%s","added_dirs":[]},"version":"2.1.322","effort":{"level":"%s"},"context_window":%s,"exceeds_200k_tokens":false}' \
    "$sid" "$PWD" "$model" "$PWD" "$PWD" "$effort" "$ctx" | sh -c "$cmd" > "$dir/statusline.out" 2>/dev/null
  printf 'StatusLine exit=%s\n' "$?" >> "$dir/statusline.log"
}
run_line() {
  case "$1" in
    work)
      hook UserPromptSubmit "$(ev UserPromptSubmit ',"prompt":"work"')"
      hook PreToolUse "$(ev PreToolUse ',"tool_name":"Bash","tool_input":{"command":"ls -la"}')"
      hook PostToolUse "$(ev PostToolUse ',"tool_name":"Bash","tool_input":{"command":"ls -la"},"tool_response":{"stdout":"ok"}')"
      hook Stop "$(ev Stop ',"last_assistant_message":"done: work"')"
      used=1 ;;
    hold) hook UserPromptSubmit "$(ev UserPromptSubmit '')" ;;
    fail) hook StopFailure "$(ev StopFailure ',"error":"authentication_failed","error_details":"Login expired · Please run /login"')" ;;
    status) statusline ;;
    write\ *)
      path=${1#write }
      mkdir -p "$(dirname "$path")"
      printf '# Written by a PTY agent\n' > "$path"
      hook PostToolUse "$(ev PostToolUse ",\"cwd\":\"$PWD\",\"tool_name\":\"Write\",\"tool_input\":{\"file_path\":\"$path\"},\"tool_response\":{}")" ;;
    shell\ *)
      set -- ${1#shell }
      mkdir -p "$(dirname "$1")"
      dated=""; [ -n "$3" ] && dated="date: $3\n"
      printf -- "---\ntype: plan\nstatus: draft\n${dated}branch: %s\n---\n\n# Written through a shell redirect\n" "$2" > "$1" ;;
    cd\ *) cd "${1#cd }" ;;
    worktree-remove)
      wt="$PWD"
      cd "$home"
      hook WorktreeRemove "$(ev WorktreeRemove ",\"cwd\":\"$PWD\",\"worktree_path\":\"$wt\"")" ;;
    subagent-stop\ *)
      set -- ${1#subagent-stop }
      wt=$(tail -n 1 "$dir/worktree-agent-$1.out" 2>/dev/null)
      [ "$2" = dirty ] && [ -n "$wt" ] && printf 'changed\n' > "$wt/changed.txt"
      hook SubagentStop "$(ev SubagentStop ",\"agent_id\":\"$1\",\"agent_type\":\"general-purpose\",\"cwd\":\"$wt\"")" ;;
    worktree\ *)
      set -- ${1#worktree }
      for name in "$@"; do worktree_create "$name" & done
      wait
      if [ $# -eq 1 ] && cd "$(tail -n 1 "$dir/worktree-$1.out")" 2>/dev/null; then
        printf 'cwd %s\n' "$PWD" >> "$dir/hooks.log"
      fi ;;
    "$esc[Z") mode=$(( (mode + 1) % 4 )); moded=1; statusline ;;
    switch)
      hook PostModelSwitch "$(ev PostModelSwitch ',"from_model":"claude-haiku-5-5","to_model":"claude-sonnet-5-5","source":"picker"')"
      model="claude-sonnet-5-5"
      statusline ;;
    ask) hook PermissionRequest "$(ev PermissionRequest ',"tool_name":"Bash","tool_input":{"command":"rm -rf build"}')" ;;
    tools)
      hook PreToolUse "$(ev PreToolUse ',"tool_name":"Read","tool_input":{"file_path":"README.md"}')"
      hook PostToolUse "$(ev PostToolUse ',"tool_name":"Read","tool_input":{"file_path":"README.md"},"tool_response":{}')" ;;
    approve)
      hook PostToolUse "$(ev PostToolUse ',"tool_name":"Bash","tool_input":{"command":"rm -rf build"},"tool_response":{"stdout":""}')"
      hook Stop "$(ev Stop ',"last_assistant_message":"done: approve"')" ;;
    question) hook PreToolUse "$(ev PreToolUse ',"tool_name":"AskUserQuestion","tool_input":{"questions":[]}')" ;;
    clear)
      hook SessionEnd "$(ev SessionEnd ',"reason":"clear"')"
      sid="$sid-cleared"
      hook SessionStart "$(ev SessionStart ',"source":"clear"')" ;;
    bad)
      hook Stop '{"session_id":'
      hook Stop '"not an object"'
      hook Stop '' ;;
    exit)
      hook SessionEnd "$(ev SessionEnd ',"reason":"prompt_input_exit"')"
      exit 0 ;;
  esac
}
rule="────────────────────────────────────────"
if [ -n "$trust" ]; then
  printf '\033[2J\033[H Accessing workspace:\n\n %s\n\n \342\235\257 No, exit\n   Yes, I trust this folder\n\n Enter to confirm \302\267 Esc to cancel\n' "$PWD"
  IFS= read -r line
  printf '%s\n' "$line" >> "$dir/stdin.log"
  [ "$line" = trust ] || exit 0
fi
if [ -z "$tui" ]; then
  # One write each, so a screen read never lands between the clear and the paint.
  box() {
    f=""; [ -n "$moded" ] && f=$(footer)
    printf '\033[2J\033[H\n%s\n\342\235\257\302\240\033[2mTry "work"\033[22m\n%s\n%s' "$rule" "$rule" "$f"
  }
  # Non-canonical, as claude reads: macOS drops a cooked line past MAX_CANON (1024 bytes) unread.
  stty -icanon min 1 time 0
  box
  hook SessionStart "$(ev SessionStart ',"source":"startup"')"
  open=""; nl=$(printf '\nx'); nl=${nl%x}
  while :; do
    if [ -n "$open" ]; then
      # Byte by byte, so a lone Esc cancels the prompt as claude's does: no hook, just the box.
      c=$(dd bs=1 count=1 2>/dev/null; printf x); c=${c%x}
      [ -n "$c" ] || exit 0
      open=""
      if [ "$c" = "$esc" ]; then box; continue; fi
      line=""
      [ "$c" = "$nl" ] || { IFS= read -r line || exit 0; line="$c$line"; }
    else
      IFS= read -r line || exit 0
    fi
    printf '%s\n' "$line" >> "$dir/stdin.log"
    run_line "$line"
    case "$line" in
      ask) open=1; printf '\033[2J\033[H Bash command\n   rm -rf build\n\n Do you want to proceed?\n \342\235\257 1. Yes\n   2. No\n\n Esc to cancel \302\267 Tab to amend\n' ;;
      question) open=1; printf '\033[2J\033[H Which one?\n \342\235\257 1. This\n   2. That\n\n Enter to select \302\267 Esc to cancel\n' ;;
      picker) printf '\033[2J\033[H   Select model\n   \342\235\257 1.  Default (recommended)   Sonnet 5.5\n\n   Enter to set as default \302\267 s to use this session only \302\267 Esc to cancel\n' ;;
      tools) ;;
      *) box ;;
    esac
  done
  exit 0
fi

cr=$(printf '\r'); tab=$(printf '\t'); ctrlu=$(printf '\025'); del=$(printf '\177')
state=input; buf=""; said=""; note=""; hl=0; eidx=0; target=""
label_of() {
  case $1 in
    claude-opus-5-5) echo "Opus 5.5" ;;
    claude-fable-5-2) echo "Fable 5.2" ;;
    claude-sonnet-5-5) echo "Sonnet 5.5" ;;
    *) echo "Haiku 5.5" ;;
  esac
}
row_label() {
  case $1 in
    0) echo "Default (recommended)" ;;
    1) echo "Opus 5.5" ;;
    2) echo "Fable 5.2" ;;
    3) echo "Sonnet 5.5" ;;
    4) echo "Haiku 5.5" ;;
  esac
}
row_id() {
  case $1 in
    1) echo claude-opus-5-5 ;;
    2) echo claude-fable-5-2 ;;
    4) echo claude-haiku-5-5 ;;
    *) echo claude-sonnet-5-5 ;;
  esac
}
row_hint() {
  case $1 in
    0) echo "Sonnet 5.5 · Efficient for routine tasks" ;;
    1) echo "For complex work and everyday tasks" ;;
    2) echo "For your toughest challenges" ;;
    3) echo "Most efficient for simpler tasks" ;;
    4) echo "Fastest for quick answers" ;;
  esac
}
row_family() {
  case $1 in 0) echo "Default (recommended)" ;; *) row_label $1 | cut -d' ' -f1 ;; esac
}
row_hint330() {
  case $1 in
    0) echo "Use the default model (currently Sonnet 5.5) · \$2/\$10 per Mtok" ;;
    1) echo "Opus 5.5 · Best for everyday, complex tasks · \$4/\$20 per Mtok" ;;
    2) echo "Fable 5.2 · Most capable for your hardest and longest-running tasks · \$8/\$40 per Mtok" ;;
    3) echo "Sonnet 5.5 · Efficient for routine tasks · \$2/\$10 per Mtok" ;;
    4) echo "Haiku 5.5 · Fastest for quick answers · \$0.10/\$0.50 per Mtok" ;;
  esac
}
eff_level() {
  case $1 in 0) echo low ;; 1) echo medium ;; 2) echo high ;; 3) echo xhigh ;; *) echo max ;; esac
}
# The centre column of each level on the labels row below.
eff_col() {
  case $1 in 0) echo 11 ;; 1) echo 21 ;; 2) echo 31 ;; 3) echo 40 ;; *) echo 50 ;; esac
}
cls() { printf '\033[2J\033[H'; }
paint_input() {
  cls
  printf ' stub claude · %s\n\n' "$(label_of "$model")"
  [ -n "$said" ] && printf '❯ %s\n' "$said"
  [ -n "$note" ] && printf '  ⎿  %s\n' "$note"
  if [ -n "$buf" ]; then
    printf '\n%s\n❯\302\240%s\n%s\n' "$rule" "$buf" "$rule"
  else
    printf '\n%s\n❯\302\240\033[2mTry "refactor <filepath>"\033[22m\n%s\n' "$rule" "$rule"
  fi
  footer; printf '\n'
}
# A footer and its last words, which a narrow terminal puts on a row of their own.
wrap() {
  if [ -n "$narrow" ]; then printf '%s\n   %s\n' "$1" "$2"; else printf '%s %s\n' "$1" "$2"; fi
}
paint_model() {
  cls
  if [ -n "$v330" ]; then
    printf '  Select model\n  Your pick is the default for new sessions. For other models, use --model.\n\n'
  else
    printf '   Select model\n   Switch between Claude models.\n\n'
  fi
  i=0
  while [ $i -le 4 ]; do
    mark="  "; [ $i -eq $hl ] && mark="❯ "
    tick=""; [ $i -ge 1 ] && [ "$(row_id $i)" = "$model" ] && tick=" ✔"
    if [ -z "$v330" ]; then
      printf '   %s%s.  %s%s   %s\n' "$mark" "$((i + 1))" "$(row_label $i)" "$tick" "$(row_hint $i)"
    else
      hint=$(row_hint330 $i); lead=$hint
      [ -n "$narrow" ] && lead=${hint% · \$*}
      printf '  %s%s. %-26s %s\n' "$mark" "$((i + 1))" "$(row_family $i)$tick" "$lead"
      [ "$lead" = "$hint" ] || printf '%32s%s\n' '' "${hint#"$lead"}"
    fi
    i=$((i + 1))
  done
  more="  "
  if [ $hl -eq 5 ]; then
    more="❯ "; echo "more highlighted" >> "$dir/tui.log"
  fi
  printf '   %s▸   More models\n\n' "$more"
  [ -n "$v330" ] && printf '  ◐ %s effort ←/→ to adjust\n\n' "$effort"
  if [ $hl -eq 5 ]; then
    printf '   Enter to expand · Esc to cancel\n'
  elif [ -n "$v330" ]; then
    wrap '  s to use this session only · Enter to save · Esc to' cancel
  else
    wrap '   Enter to set as default · s to use this session only · Esc to' cancel
  fi
}
paint_dialog() {
  cls
  printf '   Switch model?\n   Your next response will be slower and use more tokens\n\n'
  printf '   ❯ 1. Yes, switch to %s\n     2. No, go back\n' "$(label_of "$target")"
}
paint_effort() {
  cls
  printf '   Effort\n\n'
  printf "%$(eff_col $eidx)s▲\n" ''
  printf '          low     medium     high     xhigh      max      Tab to toggle\n\n'
  if [ -n "$v330" ]; then
    wrap '  ←/→ to adjust · s to use this session only · Enter to save · Esc' 'to cancel'
  else
    wrap '   ←/→ to adjust · Enter to confirm · s for this session only · Esc' 'to cancel'
  fi
}
hex() { printf '%s' "$1" | od -An -tx1 | tr -d ' \n'; }
# claude would save a default here: the driver must never send it.
trapped() {
  printf '%s %s\n' "$state" "$(hex "$1")" >> "$dir/traps.log"
  state=input; paint_input
}
switch_to() {
  state=input
  if [ -n "$silent" ]; then
    note=""; paint_input; return 0
  fi
  hook PostModelSwitch "$(ev PostModelSwitch ",\"from_model\":\"$model\",\"to_model\":\"$1\",\"source\":\"picker\"")"
  model="$1"; said="/model"; note="Set model to $(label_of "$1") for this session only"
  # Plan mode runs Haiku's turns on Sonnet: claude switches on its own right after.
  if [ $mode -eq 2 ] && [ "$1" = claude-haiku-5-5 ]; then
    hook PostModelSwitch "$(ev PostModelSwitch ',"from_model":"claude-haiku-5-5","to_model":"claude-sonnet-5-5","source":"auto"')"
    model=claude-sonnet-5-5
  fi
  paint_input; statusline
}
pick_model() {
  [ $hl -eq 5 ] && return 0
  target=$(row_id $hl)
  cmd=$(hook_command PreModelSwitch); out=""
  [ -n "$cmd" ] && out=$(ev PreModelSwitch ",\"from_model\":\"$model\",\"to_model\":\"$target\",\"source\":\"picker\"" | sh -c "$cmd" 2>/dev/null)
  case $out in
    *'"permissionDecision":"allow"'*) allowed=1; echo "PreModelSwitch allow" >> "$dir/tui.log" ;;
    *) allowed=""; echo "PreModelSwitch silent" >> "$dir/tui.log" ;;
  esac
  if [ -n "$dialog" ] && { [ -z "$allowed" ] || [ -n "$ignore_allow" ]; }; then
    echo "dialog shown" >> "$dir/tui.log"
    state=dialog; paint_dialog; return 0
  fi
  switch_to "$target"
}
on_key() {
  case $state in
    input)
      case $1 in
        "$cr")
          line="$buf"; buf=""; said=""; note=""
          case $line in
            /model)
              hl=4; i=1
              while [ $i -le 4 ]; do [ "$(row_id $i)" = "$model" ] && hl=$i; i=$((i + 1)); done
              state=model; paint_model ;;
            /effort)
              eidx=0
              while [ "$(eff_level $eidx)" != "$effort" ] && [ $eidx -lt 4 ]; do eidx=$((eidx + 1)); done
              state=effort; paint_effort ;;
            /plan) mode=2; note="Enabled plan mode"; paint_input; sleep 0.3; statusline ;;
            *) paint_input; run_line "$line"; paint_input ;;
          esac ;;
        "$ctrlu") buf=""; paint_input ;;
        "$del") buf=${buf%?}; paint_input ;;
        "esc[Z") mode=$(( (mode + 1) % 4 )); paint_input; sleep 0.3; statusline ;;
        esc) echo "interrupted" >> "$dir/tui.log" ;;
        esc*) ;;
        *) buf="$buf$1"; paint_input ;;
      esac ;;
    model)
      case $1 in
        "esc[A") hl=$(( (hl + 5) % 6 )); paint_model ;;
        "esc[B") hl=$(( (hl + 1) % 6 )); paint_model ;;
        s) pick_model ;;
        esc) note="Kept model as $(label_of "$model")"; state=input; paint_input ;;
        "$cr"|[0-9]|"esc[C"|"esc[D") trapped "$1" ;;
      esac ;;
    dialog)
      case $1 in
        "$cr") echo "dialog answered" >> "$dir/tui.log"; switch_to "$target" ;;
        esc) state=model; paint_model ;;
        [0-9]) trapped "$1" ;;
      esac ;;
    effort)
      case $1 in
        "esc[D") [ $eidx -gt 0 ] && eidx=$((eidx - 1)); paint_effort ;;
        "esc[C") [ $eidx -lt 4 ] && eidx=$((eidx + 1)); paint_effort ;;
        s)
          effort=$(eff_level $eidx); note="Set effort level to $effort (this session only)"
          state=input; paint_input; statusline ;;
        esc) state=input; paint_input ;;
        "$cr"|"$tab"|[0-9]) trapped "$1" ;;
      esac ;;
  esac
}
readkey() { k=$(dd bs=1 count=1 2>/dev/null; echo .); k=${k%.}; }
logkey() { printf '%s %s\n' "$state" "$(hex "$1")" >> "$dir/keys.log"; }
# Raw and painted before SessionStart: once the agent reads idle, keys arrive one by one onto an
# input box. Byte at a time with a 0.2 s read timeout, so a lone ESC reads as Esc.
stty -icanon -echo -icrnl min 0 time 2
paint_input
hook SessionStart "$(ev SessionStart ',"source":"startup"')"
while :; do
  readkey
  [ -n "$k" ] || continue
  logkey "$k"
  if [ "$k" != "$esc" ]; then
    on_key "$k"
    continue
  fi
  readkey
  if [ "$k" = "[" ]; then
    logkey "$k"; readkey; logkey "$k"
    on_key "esc[$k"
  else
    [ -n "$k" ] && logkey "$k"
    on_key esc
  fi
done
"#;

/// codex. `--version` prints `codex-cli 0.164.0` (`$PTY_CODEX_VERSION` overrides it); `--help`
/// lists no `--remote`, so PTY launches stay embedded. `exec` (the
/// server's session mint) appends to `agent-control/exec.log` and starts `minted-thread-e2e`.
/// `app-server` (composer) is a generic responder: every request gets thread `codex-thread-e2e`
/// and a turn, and nothing else is ever sent; it logs `argv`, `env` and `requests.log` under
/// `agent-control/<$HYPRLAYER_SESSION_ID, else app-server>/`. Otherwise it is the PTY TUI, logging
/// as `PTY_CLAUDE` does plus `acks.log` (`ok`, or `bad:<stdout>` when a hook didn't print `{}`).
/// Each hook runs the command its `-c hooks.<Event>=…` argv names. A hooked launch reports
/// `codex-hooked-thread` at `SessionStart`; stdin lines: `work`, `ask`, `esc` (`Interrupt`), `exit`,
/// `rollout` (a turn's `turn_context` and `token_count` into `rollout.jsonl`, then a `Stop` naming
/// it) and `plan` (`thread_settings_applied` for sol, high effort and Plan, then a `Stop`).
pub const PTY_CODEX: &str = r#"#!/bin/sh
base="$(dirname "$0")/../agent-control"
mkdir -p "$base"
case "$1" in
  --version) printf 'codex-cli %s\n' "${PTY_CODEX_VERSION:-0.164.0}"; exit 0 ;;
  --help) exit 0 ;;
  exec)
    printf '%s\n' "$*" >> "$base/exec.log"
    printf '{"type":"thread.started","thread_id":"minted-thread-e2e"}\n'
    exit 0 ;;
  app-server)
    dir="$base/${HYPRLAYER_SESSION_ID:-app-server}"
    mkdir -p "$dir"
    for a in "$@"; do printf '%s\n' "$a"; done > "$dir/argv"
    env | grep '^HYPRLAYER_' > "$dir/env"
    while IFS= read -r line; do
      printf '%s\n' "$line" >> "$dir/requests.log"
      id=$(printf '%s' "$line" | sed -n 's/^{"id":\([0-9][0-9]*\),"method".*/\1/p')
      [ -n "$id" ] && printf '{"id":%s,"result":{"thread":{"id":"codex-thread-e2e"},"turn":{"id":"turn-%s"}}}\n' "$id" "$id"
    done
    exit 0 ;;
esac
sid=""; prev=""
for a in "$@"; do
  [ "$prev" = resume ] && sid="$a"
  prev="$a"
done
dir="$base/${HYPRLAYER_TERMINAL_ID:-session-$sid}"
mkdir -p "$dir"
for a in "$@"; do printf '%s\n' "$a"; done > "$dir/argv"
env | grep '^HYPRLAYER_' > "$dir/env"
[ -n "$sid" ] || sid="codex-hooked-thread"
hook() {
  cmd=$(sed -n "s/^hooks\.$1=.*command='\([^']*\)'.*/\1/p" "$dir/argv")
  [ -n "$cmd" ] || return 0
  out=$(printf '%s' "$2" | sh -c "$cmd" 2>/dev/null)
  printf '%s exit=%s\n' "$1" "$?" >> "$dir/hooks.log"
  if [ "$out" = "{}" ]; then echo ok >> "$dir/acks.log"; else printf 'bad:%s\n' "$out" >> "$dir/acks.log"; fi
}
ev() { printf '{"session_id":"%s","turn_id":"turn-1","hook_event_name":"%s"%s}' "$sid" "$1" "$2"; }
stty -icanon min 1 time 0
hook SessionStart "$(ev SessionStart ',"source":"startup"')"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$dir/stdin.log"
  case "$line" in
    work)
      hook UserPromptSubmit "$(ev UserPromptSubmit ',"prompt":"work"')"
      hook PreToolUse "$(ev PreToolUse ',"tool_name":"Bash","tool_input":{"command":"ls -la"}')"
      hook PostToolUse "$(ev PostToolUse ',"tool_name":"Bash","tool_input":{"command":"ls -la"}')"
      hook Stop "$(ev Stop ',"last_assistant_message":"done: work"')" ;;
    ask) hook PermissionRequest "$(ev PermissionRequest ',"tool_name":"Bash","tool_input":{"command":"rm -rf build"}')" ;;
    esc) hook Interrupt "$(ev Interrupt '')" ;;
    rollout)
      printf '%s\n' \
        '{"timestamp":"2026-11-12T00:00:00.000Z","ordinal":0,"type":"session_meta","payload":{"id":"codex-hooked-thread"}}' \
        '{"timestamp":"2026-11-12T00:00:01.000Z","ordinal":1,"type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-5.6-luna","effort":"low","approval_policy":"on-request","collaboration_mode":{"mode":"default","settings":{"model":"gpt-5.6-luna","reasoning_effort":"low"}}}}' \
        '{"timestamp":"2026-11-12T00:00:02.000Z","ordinal":2,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":16042},"last_token_usage":{"input_tokens":15862,"output_tokens":180,"total_tokens":16042},"model_context_window":258400},"rate_limits":null}}' \
        >> "$dir/rollout.jsonl"
      hook Stop "$(ev Stop ',"model":"gpt-5.6-luna","permission_mode":"default","transcript_path":"'"$dir"'/rollout.jsonl","last_assistant_message":"done: rollout"')" ;;
    plan)
      printf '%s\n' \
        '{"timestamp":"2026-11-12T00:00:03.000Z","ordinal":3,"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"codex-hooked-thread","thread_settings":{"model":"gpt-5.6-sol","reasoning_effort":"high","approval_policy":"on-request","collaboration_mode":{"mode":"plan","settings":{"model":"gpt-5.6-sol","reasoning_effort":"high"}}}}}' \
        >> "$dir/rollout.jsonl"
      hook Stop "$(ev Stop ',"model":"gpt-5.6-sol","permission_mode":"default","transcript_path":"'"$dir"'/rollout.jsonl","last_assistant_message":"done: plan"')" ;;
    exit)
      hook SessionEnd "$(ev SessionEnd ',"reason":"exit"')"
      exit 0 ;;
  esac
done
"#;

pub const SCREEN_TUI: &str = r#"#!/bin/sh
stty -icanon -echo -icrnl min 0 time 2
cat "$1"
printf '\033[?2031h'
cr=$(printf '\r')
n=0; hl=0; winched=""; floods=0; ruled=""; full=""
trap 'winched=1' WINCH
# Each SIGWINCH's size goes to the log the second argument names. sh runs a trap only between
# commands, so two signals 30 ms apart would land as one; a sleeping perl catches each.
[ -z "$2" ] || perl -e '$SIG{WINCH} = sub { open my $f, ">>", $ARGV[0]; print $f `stty size </dev/tty` }; sleep 1 while 1' "$2" &
label() { case $1 in 0) l=low ;; 1) l=medium ;; *) l=high ;; esac; }
# Anchored to the bottom row, as claude's fullscreen renderer is, so a flood can't move it.
paint() {
  set -- $(stty size); rows=$1; cols=$2
  [ "$cols" = "$ruled" ] || { rule=$(printf "%$((cols - 1))s" '' | sed 's/ /─/g'); ruled=$cols; }
  n=$((n + 1))
  printf '\033[%s;1H\033[J Effort · %sx%s\r\n' $((rows - 5)) "$cols" "$rows"
  for i in 0 1 2; do
    mark="  "; [ $i -eq $hl ] && mark="❯ "
    label $i; printf ' %s%s. %s\r\n' "$mark" $((i + 1)) "$l"
  done
  printf '%s\r\n Enter to select · j to move · (paint %s)' "$rule" "$n"
}
listen() {
  stty min 0 time 15
  got=""
  while c=$(dd bs=64 count=1 2>/dev/null | od -An -tx1); [ -n "$c" ]; do got="$got $c"; done
  got=$(printf '%s' "$got" | tr -s ' \n' '  ')
  stty min 0 time 2
}
count() { printf '%s' "$got" | grep -o "$1" | wc -l | tr -d ' '; }
theme() {
  printf '\033]11;?\033\\\033[?996n'
  listen
  dark=' 72 67 62 3a 30 30 30 30'; light=' 72 67 62 3a 66 66 66 66'
  darks=$(count "$dark"); lights=$(count "$light")
  before_dark=${got%%"$dark"*}; before_light=${got%%"$light"*}
  first=none; sgr=0
  if [ "$darks" -gt 0 ] && [ ${#before_dark} -le ${#before_light} ]; then first=dark; sgr='97;40'
  elif [ "$lights" -gt 0 ]; then first=light; sgr='30;107'; fi
  printf '\033[%s;1H\033[2K ANSWERS osc11=%s scheme=%s dark=%s light=%s' $((rows - 7)) \
    "$(count ' 5d 31 31 3b 72 67 62 3a')" "$(count ' 5b 3f 39 39 37 3b')" "$darks" "$lights"
  printf '\033[%s;1H\033[2K\033[%sm THEME %-5s \033[0m' $((rows - 6)) "$sgr" "$first"
  paint
}
heard() {
  paint
  listen
  printf '\033[%s;1H\033[2K HEARD ctrl_f3=%s batched_key=%s' $((rows - 8)) \
    "$(count ' 1b 5b 31 3b 35 52')" "$(count ' 78 1b 5d 31 31 3b')"
  paint
}
burst() {
  i=0; while [ $i -lt "$1" ]; do yes "$line" | head -n 400; sleep 0.005; i=$((i + 1)); done
}
flood() {
  floods=$((floods + 1))
  line=$(printf '\033[1;1H0123456789 0123456789 0123456789 0123456789 0123456789 0123456789\033[K')
  burst 64
  printf '\033[3;1H\033[2K STATUS %s' "$floods"
  burst 32
  paint
}
scroll() {
  printf '\033[%s;1H\033[J' $((rows - 6))
  i=1; while [ $i -le 60 ]; do printf 'scrolled line %02d · the quick brown fox jumps over the lazy dog\r\n' $i; i=$((i + 1)); done
  paint
}
# The load scrolls above the picker, which each key repaints in one write, so echo can be timed.
load() {
  printf '\033[1;%sr\033[%s;1H' $((rows - 6)) $((rows - 6))
  yes 'yes-like load 0123456789 abcdefghijklmnopqrstuvwxyz 0123456789' & p=$!
  end=$(($(date +%s) + 5))
  while [ "$(date +%s)" -lt "$end" ]; do
    k=$(dd bs=1 count=1 2>/dev/null)
    [ -z "$k" ] || { drawn=$(paint); n=$((n + 1)); printf '\0337%s\0338' "$drawn"; }
  done
  kill $p; wait $p 2>/dev/null
  printf '\033[r'
  paint
}
repaints() { i=0; while [ $i -lt 580 ]; do paint; sleep 0.016; i=$((i + 1)); done; }
emoji() {
  printf '\033[%s;1H\033[2K\033[44m \342\235\257 \360\237\216\211 Option one \033[0m \346\227\245\346\234\254\350\252\236 \360\237\207\272\360\237\207\270 \342\234\205 \303\251' $((rows - 10))
  paint
}
url() {
  printf '\033[%s;1H\033[J' $((rows - 6))
  printf 'see https://example.com/%s end\r\n' "$(printf '%0150d' 0 | tr 0 a)"
  paint
}
# On the main screen, where scrolled lines go into history; the fixture left the alternate one.
lines() {
  printf '\033[?1049l\033[%s;1H\033[J' $((rows - 6))
  printf 'progress 10%%'; printf '\rprogress 100%% done\r\n'
  i=1; while [ $i -le 300 ]; do printf 'history line %03d \033[3%sm%s\033[0m\r\n' $i $((i % 7 + 1)) "$(printf "%$((i % 90))s" '' | tr ' ' =)"; i=$((i + 1)); done
  paint
}
copy() { printf '\033]52;c;%s\a' "$(printf 'copied by the program' | base64)"; paint; }
# A resize redraws everything, as claude does: scrollback cleared, the transcript printed again.
redraw() {
  printf '\033[2J\033[3J\033[H'
  i=1; while [ $i -le 60 ]; do printf 'transcript %02d %s\r\n' $i "$(printf "%$((i * 3))s" '' | tr ' ' '~')"; i=$((i + 1)); done
  paint
}
paint
while :; do
  k=$(dd bs=1 count=1 2>/dev/null; echo .); k=${k%.}
  [ -z "$winched" ] || { winched=""; if [ -n "$full" ]; then redraw; else paint; fi; }
  case $k in
    j) hl=$(( (hl + 1) % 3 )); paint ;;
    "$cr") label $hl; printf '\033[%s;1H\033[2K ⎿  Set effort level to %s' $((rows - 9)) "$l"; paint ;;
    t) theme ;;
    h) heard ;;
    F) flood ;;
    S) scroll ;;
    Y) load ;;
    R) repaints ;;
    E) emoji ;;
    U) url ;;
    L) lines ;;
    C) copy ;;
    W) full=1; redraw ;;
    w) full=""; paint ;;
  esac
done
"#;

/// Where both stubs log.
pub fn stub_dir(sandbox: &Sandbox) -> PathBuf {
    sandbox.root.join("agent-control")
}

/// One launch's logs: its terminal id, `session-<id>` for an identity-less PTY, or a composer session id.
pub fn launch_log(sandbox: &Sandbox, key: &str) -> PathBuf {
    stub_dir(sandbox).join(key)
}

/// Writes an executable outside the sandbox's `bin`, for an engine command no binary pin names.
pub fn install(sandbox: &Sandbox, name: &str, script: &str) -> PathBuf {
    let dir = sandbox.root.join("engines");
    std::fs::create_dir_all(&dir).expect("create the engines dir");
    let path = dir.join(name);
    std::fs::write(&path, script).expect("write an engine stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make an engine stub executable");
    }
    path
}

/// Reads, edits and stores the app settings; returns what the server saved.
pub async fn update_settings(
    j: &mut Journey,
    client: &RpcClient,
    edit: impl FnOnce(&mut AppSettings),
) -> Value {
    let current = j
        .call(client, Request::GetAppSettings {})
        .await
        .expect("get_app_settings");
    let mut settings: AppSettings =
        serde_json::from_value(current).expect("decode the app settings");
    edit(&mut settings);
    j.call(client, Request::SetAppSettings { settings })
        .await
        .expect("set_app_settings")
}

pub async fn set_level(j: &mut Journey, client: &RpcClient, level: AgentControlLevel) {
    update_settings(j, client, |s| s.agent_control.level = level).await;
}

/// Points a builtin engine at a stub, with `args` ahead of the session args.
pub async fn set_engine(
    j: &mut Journey,
    client: &RpcClient,
    engine_id: &str,
    command: &Path,
    args: &[&str],
) {
    update_settings(j, client, |s| {
        let engine = s
            .multiplexer
            .engines
            .iter_mut()
            .find(|e| e.id == engine_id)
            .expect("a builtin engine");
        engine.command = command.display().to_string();
        engine.args = args.iter().map(|a| a.to_string()).collect();
    })
    .await;
}

/// A fresh desktop launch: no session id, no resume, no bypass, at 80×24.
pub async fn launch_pty(
    client: &RpcClient,
    workstream_id: &str,
    engine_id: &str,
) -> (PtyLaunched, TerminalStream) {
    let (launched, _stream_id, events) = client
        .pty_launch_stream(
            workstream_id.to_string(),
            engine_id.to_string(),
            PtyLaunchSession {
                session_id: None,
                resume: false,
                bypass: false,
                model: None,
                permission_mode: None,
                effort: None,
                first_prompt: None,
            },
            80,
            24,
        )
        .await
        .expect("pty_launch");
    (launched, events)
}

/// Types `line` and Enter into a terminal.
pub async fn type_line(client: &RpcClient, terminal_id: &str, line: &str) {
    client
        .terminal_write(
            terminal_id.to_string(),
            crate::keystrokes(&format!("{line}\r")),
        )
        .await
        .expect("terminal_write");
}

/// Waits for a file a stub writes, so a check never races the launch.
pub async fn appears(path: &Path) -> bool {
    crate::eventually(Duration::from_secs(30), || path.exists()).await
}

/// A file's lines, or none when it doesn't exist yet.
pub fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// An agent's launch `HYPRLAYER_*` env, by terminal or composer session id (`GOAL_CLAUDE`'s too).
pub fn agent_env(sandbox: &Sandbox, id: &str) -> Vec<(String, String)> {
    let stub = launch_log(sandbox, id).join("env");
    let path = if stub.exists() {
        stub
    } else {
        sandbox.root.join("goal-control").join(id).join("agent-env")
    };
    lines(&path)
        .iter()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// One variable from an agent env, or `""`.
pub fn env_var<'a>(env: &'a [(String, String)], key: &str) -> &'a str {
    env.iter()
        .find(|(k, _)| k == key)
        .map_or("", |(_, v)| v.as_str())
}

/// How one run of a `hyprlayer-server` face ended.
#[derive(Debug)]
pub struct FaceRun {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

/// Runs `hyprlayer-server <args>` with only `env`: the shell may export the release profile's dirs.
pub async fn run_face(args: &[&str], env: &[(String, String)], stdin: &[u8]) -> FaceRun {
    let started = Instant::now();
    let mut child = Command::new(server_binary())
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn a hyprlayer-server face");
    let mut input = child.stdin.take().expect("stdin piped");
    let _ = input.write_all(stdin).await;
    drop(input);
    let output = tokio::time::timeout(FACE_TIMEOUT, child.wait_with_output())
        .await
        .expect("the face exits")
        .expect("wait for the face");
    FaceRun {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        elapsed: started.elapsed(),
    }
}

/// `hyprlayer-server mcp [--read-only]` under an agent's env, spoken to as an MCP client would.
pub struct McpDriver {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl McpDriver {
    pub fn start(env: &[(String, String)], read_only: bool) -> Self {
        let mut command = Command::new(server_binary());
        command.arg("mcp");
        if read_only {
            command.arg("--read-only");
        }
        let mut child = command
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn hyprlayer-server mcp");
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
        Self {
            _child: child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    /// The whole JSON-RPC reply to one request; `Null` if none comes.
    pub async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        let read = async {
            while let Ok(Some(line)) = self.stdout.next_line().await {
                let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if reply["id"] == json!(id) {
                    return reply;
                }
            }
            Value::Null
        };
        tokio::time::timeout(MCP_REPLY, read)
            .await
            .unwrap_or(Value::Null)
    }

    pub async fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await;
    }

    /// `initialize` then `notifications/initialized`; returns the `initialize` reply.
    pub async fn initialize(&mut self, protocol_version: &str) -> Value {
        let reply = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": protocol_version,
                    "capabilities": {},
                    "clientInfo": { "name": "hyprlayer-e2e", "version": "0" },
                }),
            )
            .await;
        self.notify("notifications/initialized", json!({})).await;
        reply
    }

    /// A `tools/call`'s `result`: `{content: [{type, text}], isError}`.
    pub async fn call(&mut self, tool: &str, args: Value) -> Value {
        self.request("tools/call", json!({ "name": tool, "arguments": args }))
            .await["result"]
            .take()
    }

    async fn send(&mut self, message: Value) {
        let line = format!("{message}\n");
        let _ = self.stdin.write_all(line.as_bytes()).await;
        let _ = self.stdin.flush().await;
    }
}

/// A tool result's text, where a refusal reads `<code>: <message>`.
pub fn tool_text(result: &Value) -> String {
    crate::text(result, "/content/0/text")
}

pub fn tool_failed(result: &Value) -> bool {
    result["isError"] == json!(true)
}

/// What the stub reads of a prompt sent to claude, line by line, as it logs them: a leading slash
/// command typed, and the rest as one bracketed paste after it.
pub fn delivered(prompt: &str) -> Vec<String> {
    match hyprlayer_core::pipeline::leading_command(prompt) {
        Some((command, rest)) if rest.trim().is_empty() => command.to_string(),
        Some((command, rest)) => format!("{command} \x1b[200~{rest}\x1b[201~"),
        None => format!("\x1b[200~{prompt}\x1b[201~"),
    }
    .split('\n')
    .map(str::to_string)
    .collect()
}
