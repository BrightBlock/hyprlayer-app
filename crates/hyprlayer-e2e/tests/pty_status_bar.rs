//! PTY tab titles and the status bar: claude's chained statusLine reporting model, effort and
//! context while the user's own statusLine still prints, a typed shift+tab read from the footer, a
//! model switch, the title a first prompt earns once, titles off, codex's status from its hooks and
//! rollout, and the bar changing a claude's model, effort and mode in its running TUI: a route
//! round the `/model` picker that never lands on "More models", footers a narrow terminal wraps,
//! claude 2.1.330's pickers, a switch plan mode follows with its own, and a relaunch that keeps the
//! session's mode and effort.
//!
//! The harnesses are `agent_stub`'s PTY claude (as a raw-mode TUI for the changes) and codex; the
//! title model is a claude stand-in that logs each prompt it names. Nothing here covers the real
//! binaries.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::{PtyAgentState, PtyAgentStatus, PtyLaunchSession};
use hyprlayer_e2e::agent_stub::{
    agent_env, appears, install, launch_log, launch_pty, lines, run_face, set_engine, type_line,
    update_settings, PTY_CLAUDE, PTY_CODEX,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, keystrokes, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use tokio::sync::mpsc::Receiver;

const IO: Duration = Duration::from_secs(30);
/// Long enough for a second title call, were one made, to land.
const SETTLE: Duration = Duration::from_secs(1);

/// claude for one-shots only: each title prompt is appended to `title.log`; the first two calls
/// fail, as a logged-out claude's do, and later ones answer a fixed title.
const TITLE_CLAUDE: &str = r#"#!/bin/sh
[ "$1" = --version ] && { echo '2.1.322 (Claude Code)'; exit 0; }
system=""
while [ $# -gt 0 ]; do
  case "$1" in
    --system-prompt) system="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$system" in
  "You name coding requests"*)
    log="$(dirname "$0")/../title.log"
    { cat; printf '\n---\n'; } >> "$log"
    [ "$(grep -c '^---$' "$log")" -le 2 ] && exit 1
    printf '{"type":"result","subtype":"success","is_error":false,"result":"Status bar E2E"}\n' ;;
  *)
    cat >/dev/null
    exit 1 ;;
esac
"#;

/// The user's own statusLine: it proves it read the same input by echoing its first 13 bytes.
const USER_STATUS_LINE: &str =
    r#"{"statusLine":{"type":"command","command":"printf 'user:'; head -c 13"}}"#;

#[derive(Debug, Clone)]
struct Seen {
    state: PtyAgentState,
    title: Option<String>,
    status: PtyAgentStatus,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_status_bar() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", TITLE_CLAUDE);
    let codex = sandbox.stage_tool("codex", PTY_CODEX);
    let pty_claude = install(&sandbox, "pty-claude", PTY_CLAUDE);
    let title_log = sandbox.root.join("title.log");
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("pty_status_bar", &sandbox);
    let ctl = std::fs::canonicalize(hyprlayer_e2e::server_binary()).expect("the server binary");
    j.alias(ctl.display().to_string(), "$CTL");
    j.alias(hyprlayer_e2e::server_binary().display().to_string(), "$CTL");

    let server = Server::start(&sandbox).await;
    let c = server.client();
    let mut rx = c.take_events().expect("the desktop's runtime events");
    j.call(
        &c,
        Request::AnnounceClient {
            client_id: "desktop-a".into(),
            device: "device-a".into(),
            capabilities: TARGETED_EVENTS.iter().map(|k| k.to_string()).collect(),
        },
    )
    .await
    .expect("announce_client");
    let project = onboard_and_import(&mut j, &c, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &c, &project, "Status").await;
    let ws = text(&view, "/item/id");
    let worktree = text(&view, "/item/worktreePath");
    set_engine(&mut j, &c, "claude-code", &pty_claude, &[]).await;
    set_engine(&mut j, &c, "codex", &codex, &[]).await;
    let user_settings = Path::new(&worktree).join(".claude");
    std::fs::create_dir_all(&user_settings).expect("create the project's .claude");
    std::fs::write(user_settings.join("settings.local.json"), USER_STATUS_LINE)
        .expect("write the user's statusLine");

    j.step("statusLine values reach the agent's status");
    let (launched, _claude_events) = launch_pty(&c, &ws, "claude-code").await;
    let term = launched.terminal_id.clone();
    j.alias(&term, "$CLAUDE_TERMINAL");
    if let Some(sid) = &launched.provider_session_id {
        j.alias(sid, "$CLAUDE_SESSION");
    }
    let log = launch_log(&sandbox, &term);
    appears(&log.join("hooks.log")).await;
    until(&mut rx, &term, |s| s.state == PtyAgentState::Idle).await;
    let settings = sandbox.read_json(&log.join("settings.json"));
    let statusline_command = text(&settings, "/statusLine/command");
    let post_model_switch = text(&settings, "/hooks/PostModelSwitch/0/hooks/0/command");
    let quoted = format!("'{}'", ctl.display());
    j.guard(
        "the launch's settings run the statusline face and a PostModelSwitch hook",
        statusline_command == format!("{quoted} statusline claude")
            && post_model_switch == format!("{quoted} hook claude PostModelSwitch"),
        json!({ "statusLine": statusline_command, "postModelSwitch": post_model_switch }),
    );
    type_line(&c, &term, "status").await;
    let reported = until(&mut rx, &term, |s| s.status.model.is_some()).await;
    j.guard(
        "model and effort arrive as claude reported them; a null used_percentage is no context; a hooked claude is controllable",
        reported.as_ref().is_some_and(|s| {
            s.status.model.as_deref() == Some("claude-haiku-5-5")
                && s.status.effort.as_deref() == Some("low")
                && s.status.context.is_none()
                && s.status.permission_mode.is_none()
                && s.status.controllable
        }),
        json!({ "status": reported.map(|s| s.status) }),
    );

    j.step("the user's statusLine output passes through");
    appears(&log.join("statusline.log")).await;
    let printed = sandbox
        .read(&log.join("statusline.out"))
        .unwrap_or_default();
    j.guard(
        "the face prints the project's own statusLine output, fed the same input, and exits 0",
        printed == r#"user:{"session_id""#
            && lines(&log.join("statusline.log")) == ["StatusLine exit=0"],
        json!({ "printed": printed, "runs": lines(&log.join("statusline.log")) }),
    );
    let bare = sandbox.root.join("no-statusline");
    std::fs::create_dir_all(&bare).expect("create a dir with no settings");
    let alone = run_face(
        &["statusline", "claude"],
        &[
            ("HOME".into(), sandbox.home.display().to_string()),
            (
                "CLAUDE_CONFIG_DIR".into(),
                bare.join("claude").display().to_string(),
            ),
        ],
        format!(
            r#"{{"model":{{"id":"claude-haiku-5-5"}},"workspace":{{"project_dir":"{}"}}}}"#,
            bare.display()
        )
        .as_bytes(),
    )
    .await;
    j.guard(
        "with no statusLine of the user's and no agent token, the face prints nothing and exits 0 at once",
        alone.code == Some(0) && alone.stdout.is_empty() && alone.elapsed < Duration::from_secs(1),
        json!({
            "exit": alone.code,
            "stdout": alone.stdout,
            "underOneSecond": alone.elapsed < Duration::from_secs(1),
        }),
    );

    j.step("a failed title leaves the tab to the next prompt, which earns one, and none after");
    type_line(&c, &term, "work").await;
    let mut seen = turn(&mut rx, &term).await;
    title_calls(&title_log, 2).await;
    // The claim is released only after the second call's error comes back.
    tokio::time::sleep(SETTLE).await;
    seen.extend(changes_within(&mut rx, &term, Duration::ZERO).await);
    let untitled = seen.iter().all(|s| s.title.is_none());
    type_line(&c, &term, "work").await;
    let mut seen = turn(&mut rx, &term).await;
    if !seen.iter().any(|s| s.title.is_some()) {
        seen.extend(until(&mut rx, &term, |s| s.title.is_some()).await);
    }
    let title = seen.iter().find_map(|s| s.title.clone());
    type_line(&c, &term, "work").await;
    let again = turn(&mut rx, &term).await;
    tokio::time::sleep(SETTLE).await;
    let prompts = title_prompts(&title_log);
    let listed = pty_agent(&mut j, &c, &ws, &term).await;
    j.guard(
        "the first prompt's title calls both fail and leave no title; the second prompt earns one, and a third asks for none",
        untitled
            && title.as_deref() == Some("Status bar E2E")
            && again
                .iter()
                .all(|s| s.title.as_deref() == Some("Status bar E2E"))
            && prompts.len() == 3
            && prompts.iter().all(|p| p.contains("work"))
            && listed["title"] == json!("Status bar E2E"),
        json!({
            "untitledAfterFailures": untitled,
            "title": title,
            "titleCalls": prompts.len(),
            "promptsNameTheRequest": prompts.iter().all(|p| p.contains("work")),
            "listed": listed["title"],
        }),
    );

    j.step("context follows the first response");
    type_line(&c, &term, "status").await;
    let reported = until(&mut rx, &term, |s| s.status.context.is_some()).await;
    let context = reported.and_then(|s| s.status.context);
    j.guard(
        "tokens and window are claude's own, with its rounded percentage",
        context
            .as_ref()
            .is_some_and(|c| c.tokens == 53_568 && c.window == 1_000_000 && c.percent == Some(5)),
        json!({ "context": context }),
    );

    j.step("a typed shift+tab updates the mode via the footer");
    c.terminal_write(term.clone(), keystrokes("\x1b[Z\r"))
        .await
        .expect("terminal_write");
    let first = until(&mut rx, &term, |s| s.status.permission_mode.is_some()).await;
    c.terminal_write(term.clone(), keystrokes("\x1b[Z\r"))
        .await
        .expect("terminal_write");
    let second = until(&mut rx, &term, |s| {
        s.status.permission_mode.as_deref() == Some("plan")
    })
    .await;
    let modes =
        [&first, &second].map(|s| s.as_ref().and_then(|s| s.status.permission_mode.clone()));
    j.guard(
        "each shift+tab's footer, painted after its statusLine report, moves the mode along the ring",
        modes == [Some("acceptEdits".into()), Some("plan".into())],
        json!({ "modes": modes }),
    );

    j.step("a model switch reports the new model");
    type_line(&c, &term, "switch").await;
    let switched = until(&mut rx, &term, |s| {
        s.status.model.as_deref() == Some("claude-sonnet-5-5")
    })
    .await;
    j.guard(
        "PostModelSwitch and the next statusLine both name the new model; the mode stays",
        switched.as_ref().is_some_and(|s| {
            s.status.permission_mode.as_deref() == Some("plan")
                && s.status.effort.as_deref() == Some("low")
        }),
        json!({ "status": switched.map(|s| s.status) }),
    );

    j.step("a turn an API error ends settles to idle");
    type_line(&c, &term, "hold").await;
    until(&mut rx, &term, |s| s.state == PtyAgentState::Working).await;
    type_line(&c, &term, "fail").await;
    let settled = until(&mut rx, &term, |s| s.state == PtyAgentState::Idle).await;
    let listed = pty_agent(&mut j, &c, &ws, &term).await;
    j.guard(
        "StopFailure, which claude fires instead of Stop, reads idle with the error as its activity and the last reply kept",
        settled.is_some()
            && listed["activity"] == json!("authentication_failed")
            && listed["lastReply"] == json!("done: work"),
        json!({
            "idle": settled.is_some(),
            "activity": listed["activity"],
            "lastReply": listed["lastReply"],
        }),
    );

    j.step("titles off");
    update_settings(&mut j, &c, |s| s.tab_titles.enabled = false).await;
    let (quiet, _quiet_events) = launch_pty(&c, &ws, "claude-code").await;
    let quiet_term = quiet.terminal_id.clone();
    j.alias(&quiet_term, "$UNTITLED_TERMINAL");
    if let Some(sid) = &quiet.provider_session_id {
        j.alias(sid, "$UNTITLED_SESSION");
    }
    until(&mut rx, &quiet_term, |s| s.state == PtyAgentState::Idle).await;
    type_line(&c, &quiet_term, "work").await;
    let mut seen = turn(&mut rx, &quiet_term).await;
    seen.extend(changes_within(&mut rx, &quiet_term, SETTLE).await);
    let listed = pty_agent(&mut j, &c, &ws, &quiet_term).await;
    j.guard(
        "with titles off a first prompt asks for no title",
        seen.iter().all(|s| s.title.is_none())
            && title_prompts(&title_log).len() == 3
            && listed["title"].is_null(),
        json!({
            "titled": seen.iter().any(|s| s.title.is_some()),
            "titleCalls": title_prompts(&title_log).len(),
            "listed": listed["title"],
        }),
    );
    update_settings(&mut j, &c, |s| s.tab_titles.enabled = true).await;
    type_line(&c, &quiet_term, "work").await;
    let mut seen = turn(&mut rx, &quiet_term).await;
    if !seen.iter().any(|s| s.title.is_some()) {
        seen.extend(until(&mut rx, &quiet_term, |s| s.title.is_some()).await);
    }
    let late = seen.iter().find_map(|s| s.title.clone());
    j.guard(
        "titles turned back on, the untitled tab's next prompt earns its title",
        late.as_deref() == Some("Status bar E2E") && title_prompts(&title_log).len() == 4,
        json!({ "title": late, "titleCalls": title_prompts(&title_log).len() }),
    );

    j.step("codex status from its hooks and rollout");
    let (launched, _codex_events) = launch_pty(&c, &ws, "codex").await;
    let cterm = launched.terminal_id.clone();
    j.alias(&cterm, "$CODEX_TERMINAL");
    let clog = launch_log(&sandbox, &cterm);
    appears(&clog.join("hooks.log")).await;
    until(&mut rx, &cterm, |s| s.state == PtyAgentState::Idle).await;
    type_line(&c, &cterm, "rollout").await;
    let turned = until(&mut rx, &cterm, |s| {
        s.status.effort.is_some() && s.status.context.is_some()
    })
    .await;
    type_line(&c, &cterm, "plan").await;
    let planned = until(&mut rx, &cterm, |s| s.status.planning).await;
    let first = turned.map(|s| s.status);
    let second = planned.map(|s| s.status);
    j.guard(
        "the hook's model and mode, then the rollout's effort, tokens, and the used share codex's own footer implies (100 - its % left)",
        first.as_ref().is_some_and(|s| {
            s.model.as_deref() == Some("gpt-5.6-luna")
                && s.effort.as_deref() == Some("low")
                && s.permission_mode.as_deref() == Some("default")
                && !s.planning
                && s.context.as_ref().is_some_and(|c| {
                    // codex shows 98% left here: (258400-12000 - (16042-12000)) / (258400-12000).
                    c.tokens == 16_042 && c.window == 258_400 && c.percent == Some(2)
                })
        }),
        json!({ "status": first }),
    );
    j.guard(
        "a later thread_settings_applied, read from where the last read stopped, sets model, effort and Plan",
        second.as_ref().is_some_and(|s| {
            s.model.as_deref() == Some("gpt-5.6-sol")
                && s.effort.as_deref() == Some("high")
                && s.planning
                && s.context.as_ref().is_some_and(|c| c.tokens == 16_042)
        }),
        json!({ "status": second }),
    );

    j.step("the bar changes a claude's model in its running TUI");
    let tui_stub = TuiStub {
        sandbox: &sandbox,
        ws: &ws,
        stub: &pty_claude,
    };
    let tui = tui_stub
        .launch(
            &mut j,
            &c,
            &mut rx,
            &["--stub-tui", "--stub-dialog"],
            "$TUI",
        )
        .await;
    type_line(&c, &tui.term, "status").await;
    let footed = until(&mut rx, &tui.term, |s| s.status.permission_mode.is_some()).await;
    j.guard(
        "the mode reads off a footer whose words claude places with column moves",
        footed
            .as_ref()
            .is_some_and(|s| s.status.permission_mode.as_deref() == Some("default")),
        json!({ "mode": footed.and_then(|s| s.status.permission_mode) }),
    );
    let outcome = control(
        &mut j,
        &c,
        Request::PtyAgentSetModel {
            terminal_id: tui.term.clone(),
            model: "claude-sonnet-5-5".into(),
        },
    )
    .await;
    let switched = until(&mut rx, &tui.term, |s| {
        s.status.model.as_deref() == Some("claude-sonnet-5-5")
    })
    .await;
    let path = lines(&tui.log.join("tui.log"));
    j.guard(
        "/model, the Sonnet row and `s` switch it for the session; PreModelSwitch allows the driven switch, so no dialog shows",
        outcome == "applied" && switched.is_some() && path == ["PreModelSwitch allow"],
        json!({ "outcome": outcome, "switched": switched.is_some(), "switchPath": path }),
    );
    let own = run_face(
        &["hook", "claude", "PreModelSwitch"],
        &agent_env(&sandbox, &tui.term),
        br#"{"hook_event_name":"PreModelSwitch","from_model":"claude-sonnet-5-5","to_model":"claude-opus-5-5","source":"command"}"#,
    )
    .await;
    j.guard(
        "with no switch of the bar's pending, the hook prints nothing, so the user's own switch keeps claude's dialog",
        own.code == Some(0) && own.stdout.is_empty(),
        json!({ "exit": own.code, "stdout": own.stdout }),
    );

    j.step("the bar changes effort and mode");
    let effort = control(
        &mut j,
        &c,
        Request::PtyAgentSetEffort {
            terminal_id: tui.term.clone(),
            effort: "high".into(),
        },
    )
    .await;
    let raised = until(&mut rx, &tui.term, |s| {
        s.status.effort.as_deref() == Some("high")
    })
    .await;
    j.guard(
        "the /effort slider moves to high and `s` applies it; the statusLine reports it",
        effort == "ok" && raised.is_some(),
        json!({ "outcome": effort, "reported": raised.map(|s| s.status.effort) }),
    );
    let mut modes = Vec::new();
    for mode in ["acceptEdits", "plan", "bypassPermissions"] {
        let outcome = control(
            &mut j,
            &c,
            Request::PtyAgentSetPermissionMode {
                terminal_id: tui.term.clone(),
                mode: mode.into(),
            },
        )
        .await;
        let listed = pty_agent(&mut j, &c, &ws, &tui.term).await;
        modes.push(json!({
            "asked": mode,
            "outcome": outcome,
            "now": listed["status"]["permissionMode"],
        }));
    }
    j.guard(
        "shift+tab reaches accept edits and /plan reaches plan; bypass, outside this ring, is refused and the cycle stops back at plan",
        modes[0]["outcome"] == json!("ok")
            && modes[0]["now"] == json!("acceptEdits")
            && modes[1]["outcome"] == json!("ok")
            && modes[1]["now"] == json!("plan")
            && modes[2]["outcome"]
                .as_str()
                .is_some_and(|o| o.starts_with("pty_control_failed: "))
            && modes[2]["now"] == json!("plan"),
        json!({ "modes": modes }),
    );

    j.step("a change is refused over a person's draft, which survives it");
    let keys_log = tui.log.join("keys.log");
    let keys_start = lines(&keys_log).len();
    c.terminal_write(tui.term.clone(), keystrokes("draft"))
        .await
        .expect("terminal_write");
    let typed_in = hyprlayer_e2e::eventually(IO, || lines(&keys_log).len() == keys_start + 5).await;
    tokio::time::sleep(SETTLE).await;
    let drafted = control(
        &mut j,
        &c,
        Request::PtyAgentSetEffort {
            terminal_id: tui.term.clone(),
            effort: "max".into(),
        },
    )
    .await;
    let keys_refused = lines(&keys_log).len();
    // The person sends the draft; the stub reads the line its box held.
    c.terminal_write(tui.term.clone(), keystrokes("\r"))
        .await
        .expect("terminal_write");
    hyprlayer_e2e::eventually(IO, || lines(&keys_log).len() == keys_refused + 1).await;
    let sent = typed_commands(&lines(&keys_log)).last().cloned();
    j.guard(
        "with text on the input line the change is refused before a key is sent, and the draft is sent intact",
        typed_in
            && drafted == "pty_control_failed: the agent's input holds a draft; clear your draft first"
            && keys_refused == keys_start + 5
            && sent.as_deref() == Some("draft"),
        json!({
            "refusal": drafted,
            "keysWhileRefused": keys_refused.saturating_sub(keys_start + 5),
            "sent": sent,
        }),
    );

    j.step("a change is refused while the agent asks or shows a prompt");
    type_line(&c, &tui.term, "ask").await;
    until(&mut rx, &tui.term, |s| {
        s.state == PtyAgentState::AwaitingPermission
    })
    .await;
    let keys_before = lines(&tui.log.join("keys.log")).len();
    let asking = control(
        &mut j,
        &c,
        Request::PtyAgentSetEffort {
            terminal_id: tui.term.clone(),
            effort: "max".into(),
        },
    )
    .await;
    let keys_after = lines(&tui.log.join("keys.log")).len();
    type_line(&c, &tui.term, "work").await;
    until(&mut rx, &tui.term, |s| s.state == PtyAgentState::Idle).await;
    type_line(&c, &tui.term, "/model").await;
    tokio::time::sleep(SETTLE).await;
    let prompted = control(
        &mut j,
        &c,
        Request::PtyAgentSetEffort {
            terminal_id: tui.term.clone(),
            effort: "max".into(),
        },
    )
    .await;
    c.terminal_write(tui.term.clone(), keystrokes("\x1b"))
        .await
        .expect("terminal_write");
    j.guard(
        "asking, the change is refused before a key is sent; with the user's own /model picker open, it is refused as a prompt",
        asking.starts_with("pty_control_failed: ")
            && asking.contains("asking")
            && keys_after == keys_before
            && prompted == "pty_control_failed: the agent is showing a prompt",
        json!({
            "asking": asking,
            "keysWhileAsking": keys_after - keys_before,
            "prompted": prompted,
        }),
    );

    j.step("a model route never lands on More models");
    let route = tui_stub
        .launch(
            &mut j,
            &c,
            &mut rx,
            &["--stub-tui", "--stub-narrow"],
            "$ROUTE",
        )
        .await;
    type_line(&c, &route.term, "status").await;
    until(&mut rx, &route.term, |s| s.status.model.is_some()).await;
    let outcome = control(
        &mut j,
        &c,
        Request::PtyAgentSetModel {
            terminal_id: route.term.clone(),
            model: "claude-opus-5-5".into(),
        },
    )
    .await;
    let opus = until(&mut rx, &route.term, |s| {
        s.status.model.as_deref() == Some("claude-opus-5-5")
    })
    .await;
    let path = lines(&route.log.join("tui.log"));
    j.guard(
        "Haiku to Opus, three rows either way round, goes up: down would land on More models, whose Enter expands it",
        outcome == "applied"
            && opus.is_some()
            && !path.iter().any(|line| line == "more highlighted"),
        json!({ "outcome": outcome, "switched": opus.is_some(), "switchPath": path }),
    );

    j.step("a narrow terminal's wrapped picker footers still drive the change");
    let narrow = control(
        &mut j,
        &c,
        Request::PtyAgentSetEffort {
            terminal_id: route.term.clone(),
            effort: "max".into(),
        },
    )
    .await;
    let maxed = until(&mut rx, &route.term, |s| {
        s.status.effort.as_deref() == Some("max")
    })
    .await;
    j.guard(
        "with /model's and /effort's footers wrapped onto a second row, the switch above and an effort change both apply",
        narrow == "ok" && maxed.is_some(),
        json!({ "outcome": narrow, "reported": maxed.and_then(|s| s.status.effort) }),
    );

    j.step("claude 2.1.330's pickers, whose Enter saves a default, still change it session-only");
    let v330 = tui_stub
        .launch(
            &mut j,
            &c,
            &mut rx,
            &["--stub-tui", "--stub-330", "--stub-narrow"],
            "$V330",
        )
        .await;
    type_line(&c, &v330.term, "status").await;
    until(&mut rx, &v330.term, |s| s.status.model.is_some()).await;
    let model = control(
        &mut j,
        &c,
        Request::PtyAgentSetModel {
            terminal_id: v330.term.clone(),
            model: "claude-fable-5-2".into(),
        },
    )
    .await;
    let fable = until(&mut rx, &v330.term, |s| {
        s.status.model.as_deref() == Some("claude-fable-5-2")
    })
    .await;
    let effort = control(
        &mut j,
        &c,
        Request::PtyAgentSetEffort {
            terminal_id: v330.term.clone(),
            effort: "high".into(),
        },
    )
    .await;
    let high = until(&mut rx, &v330.term, |s| {
        s.status.effort.as_deref() == Some("high")
    })
    .await;
    j.guard(
        "rows naming the model only in their hint, and footers offering `s` before Enter's save, still switch to Fable and high effort",
        model == "applied" && fable.is_some() && effort == "ok" && high.is_some(),
        json!({
            "model": model,
            "switched": fable.is_some(),
            "effort": effort,
            "reported": high.and_then(|s| s.status.effort),
        }),
    );

    j.step("a switch in plan mode confirms though claude's own switch follows it");
    let plan = control(
        &mut j,
        &c,
        Request::PtyAgentSetPermissionMode {
            terminal_id: route.term.clone(),
            mode: "plan".into(),
        },
    )
    .await;
    let outcome = control(
        &mut j,
        &c,
        Request::PtyAgentSetModel {
            terminal_id: route.term.clone(),
            model: "claude-haiku-5-5".into(),
        },
    )
    .await;
    let planned = until(&mut rx, &route.term, |s| {
        s.status.model.as_deref() == Some("claude-sonnet-5-5")
    })
    .await;
    j.guard(
        "the picker's PostModelSwitch to Haiku confirms it, and the bar then shows the Sonnet claude's plan mode runs",
        plan == "ok" && outcome == "applied" && planned.is_some(),
        json!({ "plan": plan, "outcome": outcome, "planModeModel": planned.and_then(|s| s.status.model) }),
    );

    j.step("the dialog fallback answers only claude's own dialog");
    let warm = tui_stub
        .launch(
            &mut j,
            &c,
            &mut rx,
            &["--stub-tui", "--stub-dialog", "--stub-ignore-allow"],
            "$WARM",
        )
        .await;
    let outcome = control(
        &mut j,
        &c,
        Request::PtyAgentSetModel {
            terminal_id: warm.term.clone(),
            model: "claude-sonnet-5-5".into(),
        },
    )
    .await;
    let path = lines(&warm.log.join("tui.log"));
    j.guard(
        "a \"Switch model?\" dialog shown despite the hook gets one Enter on its Yes row, and the switch confirms",
        outcome == "applied"
            && path == ["PreModelSwitch allow", "dialog shown", "dialog answered"],
        json!({ "outcome": outcome, "switchPath": path }),
    );

    j.step("an unconfirmed change falls back to a relaunch, or errors mid-turn");
    let mute = tui_stub
        .launch(
            &mut j,
            &c,
            &mut rx,
            &["--stub-tui", "--stub-silent-switch"],
            "$MUTE",
        )
        .await;
    let idle = control(
        &mut j,
        &c,
        Request::PtyAgentSetModel {
            terminal_id: mute.term.clone(),
            model: "claude-sonnet-5-5".into(),
        },
    )
    .await;
    type_line(&c, &mute.term, "hold").await;
    until(&mut rx, &mute.term, |s| s.state == PtyAgentState::Working).await;
    let busy = control(
        &mut j,
        &c,
        Request::PtyAgentSetModel {
            terminal_id: mute.term.clone(),
            model: "claude-opus-5-5".into(),
        },
    )
    .await;
    let (relaunched, _stream, _relaunch_events) = c
        .pty_launch_stream(
            ws.clone(),
            "claude-code".into(),
            PtyLaunchSession {
                session_id: Some(mute.session.clone()),
                resume: true,
                bypass: false,
                model: Some("claude-sonnet-5-5".into()),
                permission_mode: Some("plan".into()),
                effort: Some("high".into()),
                first_prompt: None,
            },
            80,
            24,
        )
        .await
        .expect("pty_launch");
    j.alias(&relaunched.terminal_id, "$RELAUNCH");
    let relaunch_argv = launch_log(&sandbox, &relaunched.terminal_id).join("argv");
    appears(&relaunch_argv).await;
    let argv = lines(&relaunch_argv);
    let resumes = argv.windows(4).any(|w| {
        w == [
            "--resume",
            mute.session.as_str(),
            "--model",
            "claude-sonnet-5-5",
        ]
    });
    let keeps = argv
        .windows(4)
        .any(|w| w == ["--permission-mode", "plan", "--effort", "high"]);
    j.guard(
        "idle and unconfirmed, the desktop is told to relaunch, which resumes the session on the model in its mode and effort; working, it is an error",
        idle == "relaunch"
            && busy == "pty_control_failed: claude didn't confirm the switch to Opus 5.5"
            && resumes
            && keeps,
        json!({
            "idle": idle,
            "busy": busy,
            "relaunchResumesOnTheModel": resumes,
            "relaunchKeepsModeAndEffort": keeps,
        }),
    );

    j.step("the TUI never got a digit, an Enter in a picker, or a Tab in the slider");
    let mut keys = Vec::new();
    for launch in [&tui, &route, &v330, &warm, &mute] {
        keys.extend(lines(&launch.log.join("keys.log")));
    }
    let traps: Vec<String> = [&tui, &route, &v330, &warm, &mute]
        .iter()
        .flat_map(|l| lines(&l.log.join("traps.log")))
        .collect();
    let unsafe_keys: Vec<&String> = keys
        .iter()
        .filter(|line| match line.split_once(' ') {
            Some((screen, byte)) => {
                let digit = byte.len() == 2 && ("30"..="39").contains(&byte);
                let enter = byte == "0d" && matches!(screen, "model" | "effort");
                let tab = byte == "09" && screen == "effort";
                (digit && screen != "input") || enter || tab
            }
            None => true,
        })
        .collect();
    let typed = typed_commands(&keys);
    // A driver Esc outside a picker would interrupt the turn; the busy switch above is where it would land.
    let interrupts = [&tui, &route, &v330, &warm, &mute]
        .iter()
        .flat_map(|l| lines(&l.log.join("tui.log")))
        .filter(|line| line == "interrupted")
        .count();
    j.guard(
        "every picker key is an arrow, `s` or Esc, the only Enter outside the input is the dialog's, no Esc reached the input, and commands are typed bare",
        unsafe_keys.is_empty()
            && traps.is_empty()
            && interrupts == 0
            && keys.iter().filter(|k| *k == "dialog 0d").count() == 1
            && typed.iter().all(|t| matches!(t.as_str(), "/model" | "/effort" | "/plan" | "status" | "ask" | "work" | "hold" | "draft")),
        json!({ "unsafe": unsafe_keys, "traps": traps, "interrupts": interrupts, "typed": typed }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&c).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

fn seen_of(event: &RuntimeEvent, terminal: &str) -> Option<Seen> {
    match event {
        RuntimeEvent::PtyAgentChanged {
            terminal_id,
            state,
            title,
            status,
            ..
        } if terminal_id == terminal => Some(Seen {
            state: *state,
            title: title.clone(),
            status: status.clone(),
        }),
        _ => None,
    }
}

/// The first change for `terminal` that `stop` matches, within `IO`.
async fn until(
    rx: &mut Receiver<RuntimeEvent>,
    terminal: &str,
    stop: impl Fn(&Seen) -> bool,
) -> Option<Seen> {
    events_until(rx, IO, |e| seen_of(e, terminal).is_some_and(|s| stop(&s)))
        .await
        .iter()
        .filter_map(|e| seen_of(e, terminal))
        .rfind(|s| stop(s))
}

/// Every change for `terminal` through one turn: up to working, then up to idle.
async fn turn(rx: &mut Receiver<RuntimeEvent>, terminal: &str) -> Vec<Seen> {
    let mut seen = Vec::new();
    for state in [PtyAgentState::Working, PtyAgentState::Idle] {
        seen.extend(
            events_until(rx, IO, |e| {
                seen_of(e, terminal).is_some_and(|s| s.state == state)
            })
            .await
            .iter()
            .filter_map(|e| seen_of(e, terminal)),
        );
    }
    seen
}

async fn changes_within(
    rx: &mut Receiver<RuntimeEvent>,
    terminal: &str,
    within: Duration,
) -> Vec<Seen> {
    events_until(rx, within, |_| false)
        .await
        .iter()
        .filter_map(|e| seen_of(e, terminal))
        .collect()
}

/// Where the raw-mode TUI claude launches: the stub and the workstream it runs in.
struct TuiStub<'a> {
    sandbox: &'a Sandbox,
    ws: &'a str,
    stub: &'a Path,
}

/// One TUI launch, idle and logging.
struct TuiLaunch {
    term: String,
    session: String,
    log: PathBuf,
}

impl TuiStub<'_> {
    /// Launches with `args` as the engine's own, and waits for it to read idle.
    async fn launch(
        &self,
        j: &mut Journey,
        c: &RpcClient,
        rx: &mut Receiver<RuntimeEvent>,
        args: &[&str],
        alias: &str,
    ) -> TuiLaunch {
        set_engine(j, c, "claude-code", self.stub, args).await;
        let (launched, _events) = launch_pty(c, self.ws, "claude-code").await;
        let term = launched.terminal_id.clone();
        j.alias(&term, alias);
        let session = launched.provider_session_id.unwrap_or_default();
        j.alias(&session, format!("{alias}_SESSION"));
        let log = launch_log(self.sandbox, &term);
        appears(&log.join("hooks.log")).await;
        until(rx, &term, |s| s.state == PtyAgentState::Idle).await;
        TuiLaunch { term, session, log }
    }
}

/// A bar change as a guard records it: its result, `ok` for none, or `<code>: <message>`.
async fn control(j: &mut Journey, c: &RpcClient, request: Request) -> String {
    match j.call(c, request).await {
        Ok(Value::Null) => "ok".into(),
        Ok(value) => value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_string),
        Err(e) => format!("{}: {}", e.code, e.message),
    }
}

/// Each line typed into the input box, from `keys.log`'s `input <hex>` bytes up to each Enter.
fn typed_commands(keys: &[String]) -> Vec<String> {
    let mut typed = Vec::new();
    let mut line = Vec::new();
    let mut in_sequence = 0;
    for key in keys {
        let Some(byte) = key
            .strip_prefix("input ")
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        else {
            continue;
        };
        if in_sequence > 0 {
            in_sequence -= 1;
            continue;
        }
        match byte {
            b'\r' => typed.push(String::from_utf8_lossy(&std::mem::take(&mut line)).into_owned()),
            0x15 => line.clear(),
            0x7f => {
                line.pop();
            }
            // shift+tab: `ESC [ Z`.
            0x1b => in_sequence = 2,
            _ => line.push(byte),
        }
    }
    typed
}

/// Waits up to `IO` for the title model to have been called `n` times.
async fn title_calls(path: &Path, n: usize) {
    let deadline = tokio::time::Instant::now() + IO;
    while title_prompts(path).len() < n && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Each prompt the title model was asked to name.
fn title_prompts(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .split("\n---\n")
        .filter(|p| !p.trim().is_empty())
        .map(str::to_string)
        .collect()
}

async fn pty_agent(j: &mut Journey, c: &RpcClient, ws: &str, terminal: &str) -> Value {
    j.call(
        c,
        Request::ListPtyAgents {
            workstream_id: Some(ws.to_string()),
        },
    )
    .await
    .expect("list_pty_agents")
    .as_array()
    .and_then(|a| a.iter().find(|a| a["terminalId"] == json!(terminal)))
    .cloned()
    .unwrap_or(Value::Null)
}
