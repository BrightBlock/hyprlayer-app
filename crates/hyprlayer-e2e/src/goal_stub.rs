//! The shared claude stand-in for goal scenarios, steered by control files.

use std::path::{Path, PathBuf};

use crate::Sandbox;

/// Steered by `goal-control/[<session id>/]<name>`, the session's own file first: empty applies
/// to every turn, a number to that turn only. Names: `hold` (until `release` exists), `slow`
/// (+ `deaf`, `linger`: on stdin EOF sleep 120 s, a CLI outliving its server), `stop-hook`,
/// `bg-result`, `needs-human`, `idle-turn`, `commit`, `fail-turn`, `ask` / `ask-question` (wait
/// for the reply; `ask` streams a parallel call's second `tool_use` and a subagent's text and
/// tool result while pending), `crash` / `crash-once` (`exit 1` mid-turn; `-once` deletes itself),
/// `crash-after` (`exit 1` after the turn's result).
/// `slow-exit` is read at launch; `cost` holds cents per turn (40); `tick-plan` holds a plan's path
/// and, on an optional second line, the turn that ticks every box in it. Launches log to `argv.log`
/// and `DISPLAY` to `env.log`. One-shots answer from `evaluator` (its prompts append to
/// `evaluator.log`) and `validation-summary`; `hold-summary` holds the summary until `release`,
/// touching `summary-held`. A session opened by `/validate_plan` reads its
/// controls from `goal-control/validate/` instead and replies with `validation-report`.
pub const GOAL_CLAUDE: &str = r#"#!/bin/sh
argv="$*"
base="$(dirname "$0")/../goal-control"
sid=""; budget=""; system=""; oneshot=""; validate=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id|--resume) sid="$2"; shift 2 ;;
    --max-budget-usd) budget="$2"; shift 2 ;;
    --system-prompt) system="$2"; shift 2 ;;
    --output-format) [ "$2" = json ] && oneshot=1; shift 2 ;;
    *) shift ;;
  esac
done
if [ -n "$oneshot" ]; then
  answer() {
    escaped=$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr -d '\n')
    printf '{"type":"result","subtype":"success","is_error":false,"result":"%s","total_cost_usd":%s}\n' "$escaped" "$2"
  }
  case "$system" in
    "You name coding requests"*)
      cat >/dev/null
      printf '{"type":"result","subtype":"success","is_error":false,"result":"Goal E2E"}\n' ;;
    "You judge whether a coding goal is met"*)
      mkdir -p "$base"
      { cat; printf '\n---\n'; } >> "$base/evaluator.log"
      verdict='{"met":true,"reason":"looks done"}'
      [ -f "$base/evaluator" ] && verdict=$(cat "$base/evaluator")
      answer "$verdict" 0.01 ;;
    "You condense plan-validation reports"*)
      cat >/dev/null
      if [ -f "$base/hold-summary" ]; then
        touch "$base/summary-held"
        while [ ! -e "$base/release" ]; do sleep 0.1; done
        rm -f "$base/release"
      fi
      [ -f "$base/validation-summary" ] || { printf '{"type":"result","subtype":"error_during_execution","is_error":true}\n'; exit 1; }
      answer "$(cat "$base/validation-summary")" 0.01 ;;
    *)
      cat >/dev/null
      printf '{"type":"result","subtype":"error_during_execution","is_error":true}\n'
      exit 1 ;;
  esac
  exit 0
fi
dir="$base/$sid"
log="$dir/stdin.log"
mkdir -p "$dir"
echo $$ > "$dir/pid"
launch=$(printf '%s' "$argv" | tr '\n' ' ')
printf '%s\n' "$launch" >> "$base/argv.log"
printf '%s\n' "$launch" >> "$dir/argv.log"
printf 'DISPLAY=%s\n' "$DISPLAY" >> "$dir/env.log"
cents=0; turn=0; cap=""; tripped=""; cbase="$base"
[ -n "$budget" ] && cap=$(awk -v usd="$budget" 'BEGIN { printf "%d", usd * 100 + 0.5 }')
trip=',"terminal_reason":"budget_exhausted","errors":["Reached maximum budget"]'
ctl() {
  for f in "$dir/$1" "$cbase/$1"; do
    [ -f "$f" ] || continue
    want=$(cat "$f")
    [ -z "$want" ] || [ "$want" = "$turn" ]
    return
  done
  return 1
}
setting() {
  for f in "$dir/$1" "$cbase/$1"; do
    [ -f "$f" ] && { cat "$f"; return; }
  done
}
reply() {
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"%s"}]},"session_id":"%s"}\n' "$1" "$sid"
}
json_text() {
  sed 's/\\/\\\\/g; s/"/\\"/g' | awk 'NR > 1 { printf "\\n" } { printf "%s", $0 }'
}
tool_use() {
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"%s","name":"%s","input":%s}]},"session_id":"%s"}\n' "$1" "$2" "$3" "$sid"
}
result() {
  printf '{"type":"result","subtype":"%s","is_error":%s,"result":"","session_id":"%s","num_turns":%s,"total_cost_usd":%d.%02d%s}\n' \
    "$1" "$2" "$sid" "$3" $((cents / 100)) $((cents % 100)) "$4"
}
ctl slow-exit && trap 'sleep 2; exit 143' TERM
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  case "$line" in *'"type":"user"'*) ;; *) continue ;; esac
  turn=$(( $(cat "$dir/turns" 2>/dev/null || echo 0) + 1 ))
  echo "$turn" > "$dir/turns"
  if [ "$turn" = 1 ]; then
    case "$line" in *'"content":"/validate_plan'*) validate=1; cbase="$base/validate" ;; esac
  fi
  # The cap is sticky, as claude's is: a tripped process refuses every later turn at once.
  if [ -n "$tripped" ]; then
    result error_max_budget_usd true 1 "$trip"
    continue
  fi
  if ctl hold; then
    while [ ! -e "$base/release" ]; do sleep 0.1; done
    rm -f "$base/release"
  fi
  per=$(setting cost)
  cents=$((cents + ${per:-40}))
  if [ -n "$cap" ] && [ "$cents" -gt "$cap" ]; then
    tripped=1
    result error_max_budget_usd true 1 "$trip"
    continue
  fi
  if ctl slow; then
    reply "working"
    got=""
    while IFS= read -r next; do
      printf '%s\n' "$next" >> "$log"
      case "$next" in *'"subtype":"interrupt"'*) ctl deaf && continue; got=1; break ;; esac
    done
    [ -n "$got" ] || { ctl linger && sleep 120; exit 0; }
    printf '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"session_id":"%s"}\n' "$sid"
    result success false 1 ',"terminal_reason":"aborted_streaming"'
    continue
  fi
  if ctl fail-turn; then
    result error_during_execution true 1 ""
    continue
  fi
  if ctl crash-once || ctl crash; then
    printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}},"session_id":"%s","parent_tool_use_id":null}\n' "$sid"
    ctl crash-once && rm -f "$dir/crash-once" "$base/crash-once"
    exit 1
  fi
  asked=""
  if ctl ask; then
    asked=tool
    tool_use toolu-bash Bash '{"command":"echo hi"}'
    printf '{"type":"control_request","request_id":"perm-%s","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"echo hi"}}}\n' "$turn"
    # A parallel call streams its next tool_use while the ask is still pending, and so does a subagent.
    tool_use toolu-edit Edit '{"file_path":"counter.txt"}'
    tool_use toolu-task Task '{"prompt":"look around"}'
    printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"subagent looking"}]},"parent_tool_use_id":"toolu-task","session_id":"%s"}\n' "$sid"
    printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu-sub","content":"ok"}]},"parent_tool_use_id":"toolu-task","session_id":"%s"}\n' "$sid"
  elif ctl ask-question; then
    asked=1
    printf '{"type":"control_request","request_id":"q-%s","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Ship it?","header":"Ship","multiSelect":false,"options":[{"label":"Yes","description":"ship now"},{"label":"No","description":"hold"}]}]}}}\n' "$turn"
  fi
  if [ -n "$asked" ]; then
    got=""
    while IFS= read -r next; do
      printf '%s\n' "$next" >> "$log"
      case "$next" in *control_response*) got=1; break ;; esac
    done
    [ -n "$got" ] || exit 0
    [ "$asked" = tool ] && printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu-bash","content":"hi"}]},"session_id":"%s"}\n' "$sid"
  fi
  if [ -n "$validate" ]; then
    report=$(cat "$base/validation-report" 2>/dev/null)
    [ -n "$report" ] || report="The validation could not finish."
    reply "$(printf '%s' "$report" | json_text)"
  elif ctl needs-human; then
    reply "NEEDS-HUMAN: which database?"
  elif ctl idle-turn; then
    reply "nothing to do"
  elif ctl commit; then
    echo "turn $turn" >> notes.txt
    git add notes.txt && git -c core.hooksPath=/dev/null commit -qm "turn $turn"
    reply "committed"
  else
    n=$(( $(cat counter.txt 2>/dev/null || echo 0) + 1 ))
    echo "$n" > counter.txt
    reply "counter is $n"
  fi
  plan=$(setting tick-plan | sed -n 1p)
  at=$(setting tick-plan | sed -n 2p)
  if [ -n "$plan" ] && { [ -z "$at" ] || [ "$at" = "$turn" ]; }; then
    awk '/^[ \t]*[-*+][ \t]+\[ \]/ { sub(/\[ \]/, "[x]") } { print }' "$plan" > "$plan.tick" && mv "$plan.tick" "$plan"
  fi
  turns=1
  if ctl stop-hook; then
    printf '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Stop hook feedback:\\nnot at target yet"}]},"isSynthetic":true,"session_id":"%s"}\n' "$sid"
    reply "still working"
    turns=2
  fi
  result success false "$turns" ""
  ctl crash-after && exit 1
  if ctl bg-result; then
    sleep 0.5
    result success false 0 ""
  fi
done
"#;

fn control_dir(sandbox: &Sandbox) -> PathBuf {
    sandbox.root.join("goal-control")
}

fn write(dir: &Path, name: &str, contents: &str) {
    std::fs::create_dir_all(dir).expect("create the control dir");
    std::fs::write(dir.join(name), contents).expect("write a control");
}

pub fn control(sandbox: &Sandbox, name: &str, contents: &str) {
    write(&control_dir(sandbox), name, contents);
}

pub fn control_for(sandbox: &Sandbox, session_id: &str, name: &str, contents: &str) {
    write(&control_dir(sandbox).join(session_id), name, contents);
}

pub fn clear(sandbox: &Sandbox, name: &str) {
    match std::fs::remove_file(control_dir(sandbox).join(name)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("clear a control: {e}"),
    }
}
