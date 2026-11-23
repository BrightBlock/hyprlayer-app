//! Agents spawning, waiting on and messaging composer sessions: lineage and attribution, the
//! background tab every announced desktop opens, the wait outcomes, the hop, depth, children and
//! escalation limits, descendants-only control, project scope, a failed launch and a restart.
//!
//! Every agent is `goal_stub`'s claude stand-in, driven through `hyprlayer-server mcp` under its
//! own launch env; nothing here covers the real binary.
#![cfg(unix)]

use std::time::{Duration, Instant};

use hyprlayer_core::models::goal::{
    GoalCheck, GoalDetail, GoalLimits, GoalSource, GoalSpec, GoalStatus,
};
use hyprlayer_core::models::{AgentControlLevel, PermissionMode, UiRequest};
use hyprlayer_core::runtime::goals::{GoalTarget, StartGoalRequest};
use hyprlayer_core::{Harness, SessionEvent, Stage, WorktreeMode};
use hyprlayer_e2e::agent_stub::{
    agent_env, env_var, set_level, tool_failed, tool_text, update_settings, McpDriver,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::goal_stub::{clear, control, control_for, GOAL_CLAUDE};
use hyprlayer_e2e::{
    events_until, eventually, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use hyprlayer_server::streams::StreamStartOpts;
use tokio::sync::mpsc::Receiver;

const IO: Duration = Duration::from_secs(30);
const GOAL_WAIT: Duration = Duration::from_secs(90);
/// Under `McpDriver`'s 30 s reply wait, so a slow wait never wedges the driver.
const WAIT_SECS: u64 = 20;
const CHECK: &str = "test -f counter.txt";
const WATCH_ARMS: Duration = Duration::from_secs(1);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_spawn_and_message() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", GOAL_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let other_repo = sandbox.fixture_repo("other", &[("README.md", "# other\n")]);
    let mut j = Journey::new("agent_spawn_and_message", &sandbox);
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
    let view = create_workstream(&mut j, &c, &project, "Agents").await;
    let ws = text(&view, "/item/id");
    let other_ws = other_workstream(&mut j, &c, &sandbox, &other_repo).await;
    set_level(&mut j, &c, AgentControlLevel::Full).await;

    let lead = start_session(&mut j, &c, &ws, "Lead", "$LEAD").await;
    let mut lead_mcp = driver(&mut j, &sandbox, &lead, "$LEAD_TOKEN").await;

    j.step("spawn starts a background child");
    let spawned = lead_mcp
        .call(
            "spawn_agent",
            json!({ "prompt": "E2E_REVIEW", "title": "Reviewer" }),
        )
        .await;
    let child = text(&parsed(&spawned), "/sessionId");
    j.alias(&child, "$CHILD");
    let tab = open_tab(&mut rx, &child).await;
    let row = session_row(&mut j, &c, &ws, &child).await;
    j.guard(
        "the child is an agent-origin row whose parent is the caller",
        !tool_failed(&spawned) && row["origin"] == json!("agent") && row["parentSessionId"] == json!(lead),
        json!({ "spawned": !tool_failed(&spawned), "origin": row["origin"], "parentSessionId": row["parentSessionId"] }),
    );
    j.guard(
        "announced desktops get an unfocused tab carrying the mode the child launched in",
        tab.as_ref()
            .is_some_and(|t| t["focus"] == json!(false) && t["permissionMode"] == json!("default")),
        json!({ "tab": tab }),
    );

    j.step("the first turn is attributed and headed");
    let header = format!("[from agent \"Lead\" · {lead}]");
    let turns = user_turns(&c, &child).await;
    let first = turns.first().cloned().unwrap_or_default();
    eventually(IO, || !stub_user_lines(&sandbox, &child).is_empty()).await;
    let wire = stub_user_lines(&sandbox, &child);
    j.guard(
        "its author is agent:<caller>, on the caller's title",
        first.0 == format!("agent:{lead}") && first.1 == "Lead",
        json!({ "clientId": first.0, "device": first.1 }),
    );
    j.guard(
        "the logged turn and the harness's stdin both open with the header",
        first.2.starts_with(&header) && wire.first().is_some_and(|w| w.starts_with(&header)),
        json!({
            "logged": first.2.lines().next(),
            "wire": wire.first().and_then(|w| w.lines().next()),
        }),
    );

    j.step("wait returns the last reply");
    let waited = outcome(
        &lead_mcp
            .call(
                "wait_agent",
                json!({ "session_id": child, "timeout_sec": WAIT_SECS }),
            )
            .await,
    );
    j.guard(
        "idle, with the child's reply to its first turn",
        waited["state"] == json!("idle")
            && waited["lastReply"] == json!("counter is 2")
            && waited["stalled"] == json!(false)
            && waited["timedOut"] == json!(false),
        json!({ "outcome": waited }),
    );

    j.step("prompt_and_wait sends then waits");
    let answered = outcome(
        &lead_mcp
            .call(
                "prompt_and_wait",
                json!({ "session_id": child, "text": "E2E_AGAIN", "timeout_sec": WAIT_SECS }),
            )
            .await,
    );
    let turns = user_turns(&c, &child).await;
    let second = turns.get(1).cloned().unwrap_or_default();
    j.guard(
        "idle with the new turn's reply",
        answered["state"] == json!("idle") && answered["lastReply"] == json!("counter is 3"),
        json!({ "outcome": answered }),
    );
    j.guard(
        "the message is attributed and headed too",
        second.0 == format!("agent:{lead}") && second.2 == format!("{header}\n\nE2E_AGAIN"),
        json!({ "clientId": second.0, "text": second.2 }),
    );

    j.step("wait on an idle, unchanged session stalls");
    let started = Instant::now();
    let stalled = outcome(
        &lead_mcp
            .call(
                "wait_agent",
                json!({ "session_id": child, "until": "exited", "timeout_sec": WAIT_SECS }),
            )
            .await,
    );
    let elapsed = started.elapsed();
    j.guard(
        "waiting for an exit that never comes returns stalled after about 5 s, not at the deadline",
        stalled["stalled"] == json!(true)
            && stalled["timedOut"] == json!(false)
            && stalled["state"] == json!("idle")
            && stalled["lastReply"] == json!("counter is 3")
            && elapsed >= Duration::from_millis(4500)
            && elapsed < Duration::from_secs(15),
        json!({
            "outcome": stalled,
            "afterFiveSeconds": elapsed >= Duration::from_millis(4500),
            "beforeTheDeadline": elapsed < Duration::from_secs(15),
        }),
    );
    control_for(&sandbox, &child, "hold", "3");
    let sent = lead_mcp
        .call(
            "message_agent",
            json!({ "session_id": child, "text": "E2E_HELD" }),
        )
        .await;
    let held = outcome(
        &lead_mcp
            .call(
                "wait_agent",
                json!({ "session_id": child, "timeout_sec": 2 }),
            )
            .await,
    );
    j.guard(
        "a working child is never stalled; the wait times out at its deadline",
        !tool_failed(&sent)
            && held["state"] == json!("working")
            && held["timedOut"] == json!(true)
            && held["stalled"] == json!(false),
        json!({ "sent": tool_text(&sent), "outcome": held }),
    );
    // A caller that exits mid-wait leaves its request running to the deadline; nothing else may notice.
    let mut leaving = driver(&mut j, &sandbox, &lead, "$LEAD_TOKEN").await;
    let gone_child = child.clone();
    let wait = tokio::spawn(async move {
        leaving
            .call(
                "wait_agent",
                json!({ "session_id": gone_child, "timeout_sec": WAIT_SECS }),
            )
            .await
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    wait.abort();
    let _ = wait.await;
    control(&sandbox, "release", "");
    let released = outcome(
        &lead_mcp
            .call(
                "wait_agent",
                json!({ "session_id": child, "timeout_sec": WAIT_SECS }),
            )
            .await,
    );
    clear(&sandbox, "release");
    j.guard(
        "after a waiting caller is killed mid-wait, the child finishes and another wait still answers",
        released["state"] == json!("idle") && released["lastReply"] == json!("counter is 4"),
        json!({ "outcome": released }),
    );

    j.step("the hop limit stops a ping-pong");
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 2).await;
    let spawned = lead_mcp
        .call(
            "spawn_agent",
            json!({ "prompt": "E2E_PARTNER", "title": "Partner" }),
        )
        .await;
    let partner = text(&parsed(&spawned), "/sessionId");
    j.alias(&partner, "$PARTNER");
    open_tab(&mut rx, &partner).await;
    let mut partner_mcp = driver(&mut j, &sandbox, &partner, "$PARTNER_TOKEN").await;
    wait_idle(&mut lead_mcp, &partner).await;
    let mut volleys = Vec::new();
    for _ in 0..2 {
        volleys.push(ping(&mut lead_mcp, &partner, "E2E_PING").await);
        volleys.push(ping(&mut partner_mcp, &lead, "E2E_PONG").await);
    }
    let to_partner = ping(&mut lead_mcp, &partner, "E2E_PING").await;
    let to_lead = ping(&mut partner_mcp, &lead, "E2E_PONG").await;
    let notes = notifies(&mut rx, Duration::from_secs(2)).await;
    let again = ping(&mut lead_mcp, &partner, "E2E_PING").await;
    let more = notifies(&mut rx, Duration::from_secs(2)).await;
    j.guard(
        "two messages each way pass: the spawn's own first turn was not counted",
        volleys.iter().all(|v| v == "idle"),
        json!({ "volleys": volleys }),
    );
    j.guard(
        "the third message into each target is agent_limit_reached, naming that target",
        to_partner.starts_with("agent_limit_reached: ")
            && to_partner.contains("Partner got 2 agent messages with no human turn")
            && to_lead.contains("Lead got 2 agent messages with no human turn")
            && again.starts_with("agent_limit_reached"),
        json!({ "toPartner": to_partner, "toLead": to_lead, "again": again }),
    );
    j.guard(
        "each crossing raises one session-less Notify; a further refusal raises none",
        notes.len() == 2
            && notes
                .iter()
                .all(|n| n["title"] == json!("An agent needs you") && n["sessionId"].is_null())
            && more.is_empty(),
        json!({ "notifies": notes, "afterAnotherRefusal": more.len() }),
    );

    j.step("a human turn resets the hops");
    j.call(
        &c,
        Request::SendUserMessage {
            session_id: partner.clone(),
            text: "E2E_HUMAN".into(),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    wait_idle(&mut lead_mcp, &partner).await;
    let after_human = ping(&mut lead_mcp, &partner, "E2E_PING").await;
    let still_capped = ping(&mut partner_mcp, &lead, "E2E_PONG").await;
    j.guard(
        "the desktop's turn into the partner lets agents message it again; the lead stays capped",
        after_human == "idle" && still_capped.starts_with("agent_limit_reached"),
        json!({ "toPartner": after_human, "toLead": still_capped }),
    );
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 6).await;

    j.step("depth and children limits");
    let mut child_mcp = driver(&mut j, &sandbox, &child, "$CHILD_TOKEN").await;
    let spawned = child_mcp
        .call(
            "spawn_agent",
            json!({ "prompt": "E2E_GRANDCHILD", "title": "Grandchild" }),
        )
        .await;
    let grandchild = text(&parsed(&spawned), "/sessionId");
    j.alias(&grandchild, "$GRANDCHILD");
    open_tab(&mut rx, &grandchild).await;
    let mut grand_mcp = driver(&mut j, &sandbox, &grandchild, "$GRANDCHILD_TOKEN").await;
    let child_who = whoami(&mut child_mcp).await;
    let grand_who = whoami(&mut grand_mcp).await;
    j.guard(
        "the child is depth 1 under the lead, the grandchild depth 2 under the child",
        child_who["depth"] == json!(1)
            && child_who["parent"]["sessionId"] == json!(lead)
            && grand_who["depth"] == json!(2)
            && grand_who["parent"]["sessionId"] == json!(child),
        json!({
            "child": { "depth": child_who["depth"], "parent": child_who["parent"] },
            "grandchild": { "depth": grand_who["depth"], "parent": grand_who["parent"] },
        }),
    );
    let rows_before = session_count(&c, &ws).await;
    let deep = refusal(
        &grand_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_TOO_DEEP", "title": "Too deep" }),
            )
            .await,
    );
    j.guard(
        "a great-grandchild is refused at Max depth 2, and no row is written",
        deep.starts_with("agent_limit_reached: ")
            && deep.contains("a child would be 3 agents deep, past the Max depth of 2")
            && session_count(&c, &ws).await == rows_before,
        json!({ "refusal": deep, "rowsUnchanged": session_count(&c, &ws).await == rows_before }),
    );
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 2).await;
    let crowded = refusal(
        &lead_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_CROWDED", "title": "Crowded" }),
            )
            .await,
    );
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 4).await;
    j.guard(
        "with the child and the partner live, Max live children 2 refuses a third",
        crowded.starts_with("agent_limit_reached: ")
            && crowded.contains("already has 2 live children"),
        json!({ "refusal": crowded }),
    );

    j.step("only descendants can be stopped");
    let sibling = refusal(
        &partner_mcp
            .call("kill_agent", json!({ "session_id": child }))
            .await,
    );
    let cousin = refusal(
        &partner_mcp
            .call("stop_agent", json!({ "session_id": grandchild }))
            .await,
    );
    let own = lead_mcp
        .call("kill_agent", json!({ "session_id": partner }))
        .await;
    let partner_gone = gone(&c, &partner).await;
    let child_live = live(&c, &child).await;
    j.guard(
        "the partner can neither kill its sibling nor interrupt the sibling's child",
        sibling.starts_with("agent_control_denied: ")
            && sibling.contains("not spawned by this one")
            && cousin.starts_with("agent_control_denied: ")
            && cousin.contains("not spawned by this one")
            && child_live,
        json!({ "killSibling": sibling, "stopSiblingsChild": cousin, "siblingStillLive": child_live }),
    );
    j.guard(
        "the lead kills its own child",
        !tool_failed(&own) && partner_gone,
        json!({ "kill": tool_text(&own), "gone": partner_gone }),
    );

    j.step("a message revives a dead session");
    let launches = || {
        std::fs::read_to_string(
            sandbox
                .root
                .join("goal-control")
                .join(&partner)
                .join("argv.log"),
        )
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>()
    };
    let before = launches().len();
    let revived = outcome(
        &lead_mcp
            .call(
                "prompt_and_wait",
                json!({ "session_id": partner, "text": "E2E_REVIVE", "timeout_sec": WAIT_SECS }),
            )
            .await,
    );
    let after = launches();
    let resumed = after.len() == before + 1 && after.last().is_some_and(|l| l.contains("--resume"));
    let delivered = stub_user_lines(&sandbox, &partner)
        .last()
        .is_some_and(|w| w.starts_with(&header) && w.ends_with("E2E_REVIVE"));
    let revived_live = live(&c, &partner).await;
    j.guard(
        "prompt_and_wait into the partner the lead killed resumes its harness, delivers the message and returns the reply",
        revived["state"] == json!("idle")
            && revived["lastReply"] == json!("counter is 13")
            && resumed
            && delivered
            && revived_live,
        json!({
            "outcome": revived,
            "oneResumedLaunch": resumed,
            "delivered": delivered,
            "live": revived_live,
        }),
    );
    j.call(
        &c,
        Request::SetSessionPermissionMode {
            session_id: partner.clone(),
            mode: Some(PermissionMode::BypassPermissions),
        },
    )
    .await
    .expect("raise the partner to bypass");
    let killed = lead_mcp
        .call("kill_agent", json!({ "session_id": partner }))
        .await;
    let killed = !tool_failed(&killed) && gone(&c, &partner).await;
    let before = launches().len();
    let to_sibling = refusal(
        &child_mcp
            .call(
                "message_agent",
                json!({ "session_id": partner, "text": "E2E_SIBLING" }),
            )
            .await,
    );
    let relaunched = launches().len() != before || live(&c, &partner).await;
    j.guard(
        "the child cannot revive its dead sibling recorded in bypass, above its own default",
        killed
            && to_sibling.starts_with("agent_control_denied: ")
            && to_sibling.contains("runs in bypass mode, which outranks this agent's default")
            && !relaunched,
        json!({ "killedAgain": killed, "childToBypassSibling": to_sibling, "relaunched": relaunched }),
    );

    j.step(
        "a revive is refused past the cap, into an archived workstream, or with no conversation",
    );
    let launched = |id: &str| {
        std::fs::read_to_string(sandbox.root.join("goal-control").join(id).join("argv.log"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 1).await;
    let before = launched(&partner);
    let crowded = refusal(
        &lead_mcp
            .call(
                "message_agent",
                json!({ "session_id": partner, "text": "E2E_CROWDED_REVIVE" }),
            )
            .await,
    );
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 4).await;
    let relaunched = launched(&partner) != before || live(&c, &partner).await;
    j.guard(
        "with the child live, Max live children 1 refuses the revive of the lead's dead partner",
        crowded.starts_with("agent_limit_reached: ")
            && crowded.contains("this agent already has 1 live children, the Max live children")
            && !relaunched,
        json!({ "refusal": crowded, "relaunched": relaunched }),
    );
    let shelf = create_workstream(&mut j, &c, &project, "Shelved").await;
    let shelf = text(&shelf, "/item/id");
    let shelved = start_session(&mut j, &c, &shelf, "Shelved", "$SHELVED").await;
    j.call(
        &c,
        Request::ArchiveWorkstream {
            id: shelf.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let stopped = gone(&c, &shelved).await;
    let before = launched(&shelved);
    let archived = refusal(
        &lead_mcp
            .call(
                "message_agent",
                json!({ "session_id": shelved, "text": "E2E_ARCHIVED_REVIVE" }),
            )
            .await,
    );
    let relaunched = launched(&shelved) != before || live(&c, &shelved).await;
    j.guard(
        "a session in an archived workstream is not revived",
        stopped
            && archived.starts_with("agent_control_denied: ")
            && archived.contains("that session's workstream is archived")
            && !relaunched,
        json!({ "stopped": stopped, "refusal": archived, "relaunched": relaunched }),
    );
    control(&sandbox, "noinit", "");
    let spawned = lead_mcp
        .call(
            "spawn_agent",
            json!({ "prompt": "E2E_UNSTARTED", "title": "Unstarted" }),
        )
        .await;
    clear(&sandbox, "noinit");
    let unstarted = text(&parsed(&spawned), "/sessionId");
    j.alias(&unstarted, "$UNSTARTED");
    open_tab(&mut rx, &unstarted).await;
    let killed = lead_mcp
        .call("kill_agent", json!({ "session_id": unstarted }))
        .await;
    let killed = !tool_failed(&killed) && gone(&c, &unstarted).await;
    let before = launched(&unstarted);
    let unresumable = refusal(
        &lead_mcp
            .call(
                "message_agent",
                json!({ "session_id": unstarted, "text": "E2E_UNSTARTED_REVIVE" }),
            )
            .await,
    );
    let relaunched = launched(&unstarted) != before || live(&c, &unstarted).await;
    j.guard(
        "a dead session whose harness never started is not revived as a new conversation",
        killed
            && unresumable.starts_with("agent_control_denied: ")
            && unresumable.contains("never started a conversation to resume")
            && !relaunched,
        json!({ "killed": killed, "refusal": unresumable, "relaunched": relaunched }),
    );
    drain(&mut rx);

    j.step("no escalation");
    let rows_before = session_count(&c, &ws).await;
    let spawn_up = refusal(
        &lead_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_UP", "title": "Up", "permission_mode": "bypassPermissions" }),
            )
            .await,
    );
    let goal_up = refusal(
        &lead_mcp
            .call(
                "start_goal",
                json!({
                    "condition": "E2E_GOAL_UP",
                    "work_prompt": "Raise the counter.",
                    "checks": [{ "kind": "evaluator" }],
                    "new_session": { "stage": "general", "permissionMode": "bypassPermissions" },
                }),
            )
            .await,
    );
    j.guard(
        "a Default caller asking for bypass is refused through spawn_agent and start_goal alike",
        spawn_up.starts_with("agent_control_denied: ")
            && spawn_up.contains("cannot run in bypass mode, above this agent's own default")
            && goal_up == spawn_up
            && session_count(&c, &ws).await == rows_before,
        json!({
            "spawnAgent": spawn_up,
            "startGoal": goal_up,
            "rowsUnchanged": session_count(&c, &ws).await == rows_before,
        }),
    );
    let ship_up = refusal(
        &lead_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_SHIP", "title": "Ship", "stage": "ship" }),
            )
            .await,
    );
    let ship_goal = refusal(
        &lead_mcp
            .call(
                "start_goal",
                json!({
                    "condition": "E2E_SHIP_GOAL",
                    "work_prompt": "Push it.",
                    "checks": [{ "kind": "evaluator" }],
                    "new_session": { "stage": "ship" },
                }),
            )
            .await,
    );
    j.guard(
        "a Default General caller's ship child is refused through spawn_agent and start_goal, though its mode is Default too",
        ship_up.starts_with("agent_control_denied: ")
            && ship_up.contains("cannot run the ship stage")
            && ship_goal == ship_up
            && session_count(&c, &ws).await == rows_before,
        json!({
            "spawnAgent": ship_up,
            "startGoal": ship_goal,
            "rowsUnchanged": session_count(&c, &ws).await == rows_before,
        }),
    );
    j.call(
        &c,
        Request::SetSessionPermissionMode {
            session_id: lead.clone(),
            mode: Some(PermissionMode::BypassPermissions),
        },
    )
    .await
    .expect("raise the lead to bypass");
    let to_bypass = refusal(
        &child_mcp
            .call(
                "message_agent",
                json!({ "session_id": lead, "text": "E2E_UPWARD" }),
            )
            .await,
    );
    let down = lead_mcp
        .call(
            "message_agent",
            json!({ "session_id": child, "text": "E2E_DOWNWARD" }),
        )
        .await;
    wait_idle(&mut lead_mcp, &child).await;
    j.call(
        &c,
        Request::SetSessionPermissionMode {
            session_id: lead.clone(),
            mode: Some(PermissionMode::Default),
        },
    )
    .await
    .expect("lower the lead to default");
    let upward_logged = user_turns(&c, &lead)
        .await
        .iter()
        .any(|t| t.2.contains("E2E_UPWARD"));
    j.guard(
        "with the lead in bypass, its Default child's message to it is refused; the lead still messages its own child",
        to_bypass.starts_with("agent_control_denied: ")
            && to_bypass.contains("outranks this agent's default")
            && !upward_logged
            && !tool_failed(&down),
        json!({
            "childToBypassLead": to_bypass,
            "reachedTheLead": upward_logged,
            "leadToChild": tool_text(&down),
        }),
    );

    j.step("other projects are out of scope");
    let other = start_session(&mut j, &c, &other_ws, "Elsewhere", "$OTHER_SESSION").await;
    let spawn_there = refusal(
        &lead_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_THERE", "title": "There", "workstream_id": other_ws }),
            )
            .await,
    );
    let message_there = refusal(
        &lead_mcp
            .call(
                "message_agent",
                json!({ "session_id": other, "text": "E2E_THERE" }),
            )
            .await,
    );
    let wait_there = refusal(
        &lead_mcp
            .call(
                "wait_agent",
                json!({ "session_id": other, "timeout_sec": 1 }),
            )
            .await,
    );
    let other_rows = session_count(&c, &other_ws).await;
    let foreign = |r: &str| {
        r.starts_with("agent_control_denied: ") && r.contains("belongs to another project")
    };
    j.guard(
        "spawning into, messaging and waiting on another project's workstream are refused",
        foreign(&spawn_there) && foreign(&message_there) && foreign(&wait_there) && other_rows == 1,
        json!({
            "spawn": spawn_there,
            "message": message_there,
            "wait": wait_there,
            "otherProjectRows": other_rows,
        }),
    );

    j.step("a failed launch leaves no row");
    drain(&mut rx);
    let rows_before = session_count(&c, &ws).await;
    let failed = refusal(
        &lead_mcp
            .call(
                "spawn_agent",
                json!({ "prompt": "E2E_NO_CODEX", "title": "No codex", "harness": "codex" }),
            )
            .await,
    );
    let tabs = open_tabs(&mut rx, Duration::from_secs(2)).await;
    let rows_after = session_count(&c, &ws).await;
    let listed: Vec<String> = c
        .list_sessions(ws.clone())
        .await
        .expect("list_sessions")
        .into_iter()
        .map(|s| s.id)
        .collect();
    // The launch names the row it inserted; finding it gone proves the discard, not an earlier refusal.
    let launched_then_discarded = sandbox.server_log().lines().any(|line| {
        line.contains("Codex General")
            && line.contains("failed to start")
            && line
                .split_once("session ")
                .and_then(|(_, rest)| rest.split_once(':'))
                .is_some_and(|(id, _)| !listed.iter().any(|l| l == id))
    });
    j.guard(
        "a codex child with no codex installed fails its launch, leaves no row and opens no tab",
        failed.starts_with("binary_missing: ")
            && launched_then_discarded
            && rows_after == rows_before
            && tabs == 0,
        json!({
            "error": failed,
            "launchedThenDiscarded": launched_then_discarded,
            "rowsUnchanged": rows_after == rows_before,
            "openSessionTabs": tabs,
        }),
    );

    j.step("goal sessions are origin goal");
    let agent_goal = parsed(
        &lead_mcp
            .call(
                "start_goal",
                json!({
                    "condition": "E2E_AGENT_GOAL",
                    "work_prompt": "Raise the counter.",
                    "checks": [{ "kind": "evaluator" }],
                    "new_session": { "stage": "general" },
                }),
            )
            .await,
    );
    let agent_goal_id = text(&agent_goal, "/goalId");
    let agent_goal_session = text(&agent_goal, "/sessionId");
    j.alias(&agent_goal_id, "$AGENT_GOAL");
    j.alias(&agent_goal_session, "$AGENT_GOAL_SESSION");
    let agent_goal_end = wait_goal(&c, &agent_goal_id).await;
    let desktop_goal = j
        .call(
            &c,
            Request::StartGoal {
                request: StartGoalRequest {
                    target: GoalTarget::NewSession {
                        workstream_id: ws.clone(),
                        stage: Stage::General,
                        harness: Some(Harness::Claude),
                        model: None,
                        effort: None,
                        permission_mode: None,
                        attached_docs: vec![],
                    },
                    spec: goal_spec(),
                },
            },
        )
        .await
        .expect("start_goal");
    let desktop_goal_id = text(&desktop_goal, "/id");
    let desktop_goal_session = text(&desktop_goal, "/sessionId");
    j.alias(&desktop_goal_id, "$DESKTOP_GOAL");
    j.alias(&desktop_goal_session, "$DESKTOP_GOAL_SESSION");
    let desktop_goal_end = wait_goal(&c, &desktop_goal_id).await;
    let agent_row = session_row(&mut j, &c, &ws, &agent_goal_session).await;
    let desktop_row = session_row(&mut j, &c, &ws, &desktop_goal_session).await;
    j.guard(
        "a desktop's goal session is origin goal with no parent",
        desktop_row["origin"] == json!("goal")
            && desktop_row["parentSessionId"].is_null()
            && desktop_goal_end == Some(GoalStatus::Met),
        json!({ "origin": desktop_row["origin"], "parentSessionId": desktop_row["parentSessionId"], "status": desktop_goal_end.map(GoalStatus::as_str) }),
    );
    j.guard(
        "an agent's goal session is origin agent under the caller",
        agent_row["origin"] == json!("agent")
            && agent_row["parentSessionId"] == json!(lead)
            && agent_goal_end == Some(GoalStatus::Met),
        json!({ "origin": agent_row["origin"], "parentSessionId": agent_row["parentSessionId"], "status": agent_goal_end.map(GoalStatus::as_str) }),
    );

    j.step("a validation links to its implement session without being its spawn");
    let live_count = |r: &str| {
        r.split("already has ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|n| n.parse::<u32>().ok())
    };
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 0).await;
    let before = crowd(&mut lead_mcp).await;
    drain(&mut rx);
    let validate = start_linked(
        &mut j,
        &c,
        &ws,
        Stage::Validate,
        Some(&lead),
        "Validate",
        "$VALIDATE",
    )
    .await;
    let tabs = open_tabs(&mut rx, Duration::from_secs(2)).await;
    let after = crowd(&mut lead_mcp).await;
    update_settings(&mut j, &c, |s| s.agent_control.max_live_children = 4).await;
    let row = session_row(&mut j, &c, &ws, &validate).await;
    j.guard(
        "the validation is a person's session linked to the implement one, and no agent tab opens",
        row["origin"] == json!("user") && row["parentSessionId"] == json!(lead) && tabs == 0,
        json!({ "origin": row["origin"], "parentSessionId": row["parentSessionId"], "openSessionTabs": tabs }),
    );
    j.guard(
        "the link adds nothing to the implement agent's live children",
        live_count(&before).is_some()
            && live_count(&before) == live_count(&after)
            && live(&c, &validate).await,
        json!({ "before": before, "after": after }),
    );
    let kill = refusal(
        &lead_mcp
            .call("kill_agent", json!({ "session_id": validate }))
            .await,
    );
    let archive = refusal(
        &lead_mcp
            .call("archive_agent", json!({ "session_id": validate }))
            .await,
    );
    j.guard(
        "the implement agent can neither kill nor archive the validation it didn't spawn",
        kill.starts_with("agent_control_denied: ")
            && kill.contains("not spawned by this one")
            && archive.starts_with("agent_control_denied: ")
            && archive.contains("not spawned by this one")
            && live(&c, &validate).await,
        json!({ "kill": kill, "archive": archive, "stillLive": live(&c, &validate).await }),
    );
    let mut validate_mcp = driver(&mut j, &sandbox, &validate, "$VALIDATE_TOKEN").await;
    let validate_who = whoami(&mut validate_mcp).await;
    j.guard(
        "the validation's own agent is depth 0 with no parent, so its spawn budget is whole",
        validate_who["depth"] == json!(0) && validate_who["parent"].is_null(),
        json!({ "depth": validate_who["depth"], "parent": validate_who["parent"] }),
    );
    let foreign = start_linked(
        &mut j,
        &c,
        &ws,
        Stage::Validate,
        Some(&other),
        "",
        "$FOREIGN_LINK",
    )
    .await;
    let missing = start_linked(
        &mut j,
        &c,
        &ws,
        Stage::Validate,
        Some("00000000-0000-0000-0000-000000000000"),
        "",
        "$MISSING_LINK",
    )
    .await;
    let foreign_row = session_row(&mut j, &c, &ws, &foreign).await;
    let missing_row = session_row(&mut j, &c, &ws, &missing).await;
    j.guard(
        "a link to another workstream's session or to no session is dropped, and the session still starts",
        foreign_row["parentSessionId"].is_null()
            && foreign_row["origin"] == json!("user")
            && missing_row["parentSessionId"].is_null()
            && missing_row["origin"] == json!("user"),
        json!({
            "foreign": { "origin": foreign_row["origin"], "parentSessionId": foreign_row["parentSessionId"] },
            "missing": { "origin": missing_row["origin"], "parentSessionId": missing_row["parentSessionId"] },
        }),
    );

    j.step("a message its harness held when it crashed goes back to its sender");
    let holder = start_session(&mut j, &c, &ws, "Holder", "$HOLDER").await;
    control_for(&sandbox, &holder, "queue-crash", "2");
    let held_crash = hold_a_message(&c, &sandbox, &mut lead_mcp, &holder, "E2E_HELD_CRASH").await;
    let exited = gone(&c, &holder).await;
    let report = {
        let deadline = tokio::time::Instant::now() + IO;
        let mut found = None;
        while found.is_none() && tokio::time::Instant::now() < deadline {
            found = user_turns(&c, &lead).await.into_iter().find(|t| {
                t.2.contains("was not delivered: the session exited before reading it")
                    && t.2.contains("E2E_HELD_CRASH")
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        found.is_some()
    };
    let logged = user_turns(&c, &holder)
        .await
        .iter()
        .any(|t| t.2.ends_with("E2E_HELD_CRASH"));
    j.guard(
        "message_agent returns ok, the harness exits before reading it, and the sender gets a turn saying it was not delivered",
        held_crash && exited && report && !logged,
        json!({ "sent": held_crash, "exited": exited, "senderTold": report, "loggedAsRead": logged }),
    );

    j.step("a message its harness held when the server stopped is set up");
    let keeper = start_session(&mut j, &c, &ws, "Keeper", "$KEEPER").await;
    control_for(&sandbox, &keeper, "queue", "2");
    let held_restart =
        hold_a_message(&c, &sandbox, &mut lead_mcp, &keeper, "E2E_HELD_RESTART").await;
    j.guard(
        "the keeper's harness holds the message mid-turn",
        held_restart,
        json!({ "held": held_restart }),
    );

    j.step("lineage survives a restart");
    drop((lead_mcp, partner_mcp, child_mcp, grand_mcp, validate_mcp));
    let stopped = server.shutdown(&c).await;
    let server = Server::start(&sandbox).await;
    let c = server.client();
    let rows = rows(&mut j, &c, &ws).await;
    let read = |id: &str| {
        rows.iter()
            .find(|r| r["id"] == json!(id))
            .map(|r| json!({ "origin": r["origin"], "parentSessionId": r["parentSessionId"] }))
            .unwrap_or(Value::Null)
    };
    let after: Vec<Value> = [
        &child,
        &grandchild,
        &agent_goal_session,
        &desktop_goal_session,
        &validate,
    ]
    .into_iter()
    .map(|id| json!({ "session": id, "row": read(id) }))
    .collect();
    j.guard(
        "after a restart every row keeps its origin and parent",
        stopped.status.success()
            && read(&child) == json!({ "origin": "agent", "parentSessionId": lead })
            && read(&grandchild) == json!({ "origin": "agent", "parentSessionId": child })
            && read(&agent_goal_session) == json!({ "origin": "agent", "parentSessionId": lead })
            && read(&desktop_goal_session) == json!({ "origin": "goal", "parentSessionId": null })
            && read(&validate) == json!({ "origin": "user", "parentSessionId": lead })
            && read(&lead) == json!({ "origin": "user", "parentSessionId": null }),
        json!({ "restarted": stopped.status.success(), "rows": after, "lead": read(&lead) }),
    );

    j.step("the restart resends the held message once");
    let resent = |reads: usize| {
        stub_user_lines(&sandbox, &keeper)
            .iter()
            .filter(|w| w.ends_with("E2E_HELD_RESTART"))
            .count()
            == reads
    };
    let reread = eventually(IO + IO, || resent(2)).await;
    let mut stored = 0;
    let deadline = tokio::time::Instant::now() + IO;
    while stored == 0 && tokio::time::Instant::now() < deadline {
        stored = user_turns(&c, &keeper)
            .await
            .iter()
            .filter(|t| t.0 == format!("agent:{lead}") && t.2.ends_with("E2E_HELD_RESTART"))
            .count();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    j.guard(
        "the relaunched harness reads it, and the log holds it once, from its sender",
        reread && stored == 1,
        json!({ "readByTheNewProcess": reread, "storedTimes": stored }),
    );

    j.step("wake_when wakes its caller when the timer is up");
    let waker = start_session(&mut j, &c, &ws, "Waker", "$WAKER").await;
    let mut waker_mcp = driver(&mut j, &sandbox, &waker, "$WAKER_TOKEN").await;
    let started = Instant::now();
    let timer = wake_when(
        &mut j,
        &mut waker_mcp,
        json!({ "condition": { "kind": "after", "secs": 2 } }),
        "$TIMER_WAKE",
    )
    .await;
    let turn = woken(&c, &waker, &timer).await;
    let elapsed = started.elapsed();
    let header = format!("[from hyprlayer · wake {timer}]");
    eventually(IO, || {
        stub_user_lines(&sandbox, &waker)
            .iter()
            .any(|w| w.starts_with(&header))
    })
    .await;
    let wire = stub_user_lines(&sandbox, &waker);
    let pending = parsed(&waker_mcp.call("wake_when", json!({})).await)["pending"].clone();
    j.guard(
        "after 2 s the caller's harness reads a turn headed by the wake, authored agent:hyprlayer, and nothing stays pending",
        turn.0 == "agent:hyprlayer"
            && turn.1 == format!("{header}\n\ntimer up: 2 s passed")
            && wire.iter().any(|w| w.starts_with(&header))
            && elapsed >= Duration::from_secs(2)
            && pending == json!([]),
        json!({
            "clientId": turn.0,
            "text": turn.1,
            "wire": wire.iter().find(|w| w.starts_with(&header)).and_then(|w| w.lines().next()),
            "afterTwoSeconds": elapsed >= Duration::from_secs(2),
            "pending": pending,
        }),
    );

    j.step("a wake whose condition never holds times out");
    let spawned = waker_mcp
        .call(
            "spawn_agent",
            json!({ "prompt": "E2E_SLEEPER", "title": "Sleeper" }),
        )
        .await;
    let sleeper = text(&parsed(&spawned), "/sessionId");
    j.alias(&sleeper, "$SLEEPER");
    wait_idle(&mut waker_mcp, &sleeper).await;
    let started = Instant::now();
    let exit = wake_when(
        &mut j,
        &mut waker_mcp,
        json!({
            "condition": { "kind": "session", "sessionId": sleeper, "until": "exited" },
            "timeout_sec": 2,
        }),
        "$EXIT_WAKE",
    )
    .await;
    let turn = woken(&c, &waker, &exit).await;
    let elapsed = started.elapsed();
    j.guard(
        "waiting 2 s for an idle child to exit wakes the caller as timed out, at the deadline",
        turn.1
            == format!(
                "[from hyprlayer · wake {exit}]\n\ntimed out after 2 s waiting for session {sleeper} to exit"
            )
            && elapsed >= Duration::from_secs(2),
        json!({ "text": turn.1, "atTheDeadline": elapsed >= Duration::from_secs(2) }),
    );

    j.step("agent-state wakes");
    let started = Instant::now();
    let idle = wake_when(
        &mut j,
        &mut waker_mcp,
        json!({ "condition": { "kind": "session", "sessionId": sleeper, "until": "idle" } }),
        "$IDLE_WAKE",
    )
    .await;
    let turn = woken(&c, &waker, &idle).await;
    let elapsed = started.elapsed();
    j.guard(
        "a session wake on a child already idle fires at once",
        turn.1
            == format!("[from hyprlayer · wake {idle}]\n\nsession {sleeper} \"Sleeper\" is idle")
            && elapsed < Duration::from_secs(5),
        json!({ "text": turn.1, "atOnce": elapsed < Duration::from_secs(5) }),
    );
    control_for(&sandbox, &sleeper, "hold", "2");
    let sent = waker_mcp
        .call(
            "message_agent",
            json!({ "session_id": sleeper, "text": "E2E_HOLD" }),
        )
        .await;
    let any = wake_when(
        &mut j,
        &mut waker_mcp,
        json!({ "condition": { "kind": "any_agent", "until": "idle" } }),
        "$ANY_WAKE",
    )
    .await;
    let held = outcome(
        &waker_mcp
            .call(
                "wait_agent",
                json!({ "session_id": sleeper, "timeout_sec": 2 }),
            )
            .await,
    );
    let any_header = format!("[from hyprlayer · wake {any}]");
    let early = user_turns(&c, &waker)
        .await
        .iter()
        .any(|t| t.2.starts_with(&any_header));
    control(&sandbox, "release", "");
    let turn = woken(&c, &waker, &any).await;
    clear(&sandbox, "release");
    j.guard(
        "an any_agent wake holds while the child works, then names it once it goes idle",
        !tool_failed(&sent)
            && held["state"] == json!("working")
            && !early
            && turn.1 == format!("{any_header}\n\nsession {sleeper} \"Sleeper\" is idle"),
        json!({ "whileHeld": held["state"], "wokeEarly": early, "text": turn.1 }),
    );

    j.step("a ref wake rides the worktree watch");
    sandbox.git(
        &sandbox.root,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch",
            "main",
            "origin.git",
        ],
    );
    let origin = sandbox.root.join("origin.git").display().to_string();
    sandbox.git(&repo, &["remote", "add", "origin", &origin]);
    let pusher = create_workstream(&mut j, &c, &project, "Pusher").await;
    let tree = std::path::PathBuf::from(text(&pusher, "/item/worktreePath"));
    let branch = text(&pusher, "/item/branch");
    j.alias(&branch, "$PUSHER_BRANCH");
    sandbox.git(
        &tree,
        &["commit", "--quiet", "--allow-empty", "--message", "first"],
    );
    sandbox.git(&tree, &["push", "--quiet", "-u", "origin", &branch]);
    let was = sandbox.git(&tree, &["rev-parse", "--short=7", "HEAD"]);
    sandbox.git(
        &tree,
        &["commit", "--quiet", "--allow-empty", "--message", "second"],
    );
    let now = sandbox.git(&tree, &["rev-parse", "--short=7", "HEAD"]);
    j.alias(&was, "$TIP_WAS");
    j.alias(&now, "$TIP_NOW");
    j.call(
        &c,
        Request::WatchWorktree {
            workstream_id: Some(ws.clone()),
        },
    )
    .await
    .expect("watch_worktree");
    tokio::time::sleep(WATCH_ARMS).await;
    let moved = wake_when(
        &mut j,
        &mut waker_mcp,
        json!({ "condition": { "kind": "ref", "branch": branch } }),
        "$REF_WAKE",
    )
    .await;
    let started = Instant::now();
    sandbox.git(&tree, &["push", "--quiet", "origin", &branch]);
    let turn = woken(&c, &waker, &moved).await;
    let elapsed = started.elapsed();
    j.guard(
        "a push from another workstream's worktree wakes the caller within seconds, naming the move",
        turn.1
            == format!("[from hyprlayer · wake {moved}]\n\norigin/{branch} moved {was} → {now}")
            && elapsed < Duration::from_secs(15),
        json!({ "text": turn.1, "withinSeconds": elapsed < Duration::from_secs(15) }),
    );
    j.step("a wake past the hop limit is dropped");
    let mut rx = c
        .take_events()
        .expect("the restarted desktop's runtime events");
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
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 5).await;
    let capped = wake_when(
        &mut j,
        &mut waker_mcp,
        json!({ "condition": { "kind": "after", "secs": 1 } }),
        "$CAPPED_WAKE",
    )
    .await;
    let notes = notifies(&mut rx, Duration::from_secs(4)).await;
    let capped_header = format!("[from hyprlayer · wake {capped}]");
    let delivered = user_turns(&c, &waker)
        .await
        .iter()
        .any(|t| t.2.starts_with(&capped_header));
    let pending = parsed(&waker_mcp.call("wake_when", json!({})).await)["pending"].clone();
    update_settings(&mut j, &c, |s| s.agent_control.max_agent_hops = 6).await;
    j.guard(
        "after five wakes with no human turn, a sixth at a limit of 5 is dropped and the user is told once",
        !delivered
            && pending == json!([])
            && notes.len() == 1
            && notes[0]["body"]
                .as_str()
                .is_some_and(|b| b.contains("Waker got 5 agent messages in a row")),
        json!({ "delivered": delivered, "pending": pending, "notifies": notes }),
    );
    drop(waker_mcp);

    j.step("shutdown");
    let stopped = server.shutdown(&c).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

/// A second project and its workstream, imported without re-running onboarding.
async fn other_workstream(
    j: &mut Journey,
    c: &RpcClient,
    sandbox: &Sandbox,
    repo: &std::path::Path,
) -> String {
    let project = j
        .call(
            c,
            Request::ImportProject {
                path: sandbox.as_given(repo).display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            },
        )
        .await
        .expect("import_project");
    let view = j
        .call(
            c,
            Request::CreateWorkstream {
                project_path: text(&project, "/path"),
                title: "Elsewhere".into(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let id = text(&view, "/item/id");
    j.alias(&id, "$OTHER_WS");
    let slug = text(&view, "/item/slug");
    if let Some((stem, _)) = slug.rsplit_once('-') {
        j.alias(&slug, format!("{stem}-$OTHER_SUFFIX"));
    }
    id
}

/// A desktop-started Default claude session, titled, launched, and through its first turn.
async fn start_session(
    j: &mut Journey,
    c: &RpcClient,
    ws: &str,
    title: &str,
    alias: &str,
) -> String {
    start_linked(j, c, ws, Stage::General, None, title, alias).await
}

/// As `start_session`, at `stage` and carrying a person's link to `parent`; an empty title is left to the titler.
async fn start_linked(
    j: &mut Journey,
    c: &RpcClient,
    ws: &str,
    stage: Stage,
    parent: Option<&str>,
    title: &str,
    alias: &str,
) -> String {
    let (session, _prompt, _stream_id, _events) = c
        .start_stage_session_stream(
            ws.to_string(),
            stage,
            "E2E_START".into(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                permission_mode: Some(PermissionMode::Default),
                parent_session_id: parent.map(str::to_string),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, alias);
    // Set before the titler can, so the header naming this caller is fixed.
    if !title.is_empty() {
        j.call(
            c,
            Request::SetSessionTitle {
                session_id: session.id.clone(),
                title: Some(title.into()),
            },
        )
        .await
        .expect("set_session_title");
    }
    j.call(
        c,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        let log = c
            .get_session_log(session.id.clone())
            .await
            .expect("get_session_log");
        if log
            .iter()
            .any(|e| matches!(e.event, SessionEvent::TurnDone { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    session.id
}

/// Starts a turn in `target` whose harness then holds the next message, and sends it from `sender`.
async fn hold_a_message(
    c: &RpcClient,
    sandbox: &Sandbox,
    sender: &mut McpDriver,
    target: &str,
    text: &str,
) -> bool {
    c.send_user_message(
        target.to_string(),
        "E2E_BUSY".into(),
        vec![],
        None,
        None,
        None,
    )
    .await
    .expect("send_user_message");
    let busy = eventually(IO, || {
        stub_user_lines(sandbox, target)
            .iter()
            .any(|w| w == "E2E_BUSY")
    })
    .await;
    // The stub writes its lifecycle frame before the tool; only once the driver has read that
    // frame does it hold a mid-turn message instead of echoing it at once.
    let deadline = tokio::time::Instant::now() + IO;
    while !c
        .get_session_log(target.to_string())
        .await
        .expect("get_session_log")
        .iter()
        .any(|e| matches!(&e.event, SessionEvent::ToolUse { id, .. } if id == "toolu-long"))
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let sent = sender
        .call(
            "message_agent",
            json!({ "session_id": target, "text": text }),
        )
        .await;
    let read = eventually(IO, || {
        stub_user_lines(sandbox, target)
            .iter()
            .any(|w| w.ends_with(text))
    })
    .await;
    busy && !tool_failed(&sent) && read
}

/// The MCP face under a composer session's launch env, initialized.
async fn driver(j: &mut Journey, sandbox: &Sandbox, session_id: &str, alias: &str) -> McpDriver {
    let path = sandbox
        .root
        .join("goal-control")
        .join(session_id)
        .join("agent-env");
    eventually(IO, || {
        !env_var(&agent_env(sandbox, session_id), "HYPRLAYER_AGENT_TOKEN").is_empty()
    })
    .await;
    let env = agent_env(sandbox, session_id);
    j.alias(env_var(&env, "HYPRLAYER_AGENT_TOKEN"), alias);
    assert!(
        path.exists(),
        "the agent env of {session_id} was never written"
    );
    let mut mcp = McpDriver::start(&env, false);
    mcp.initialize("2025-06-18").await;
    mcp
}

fn parsed(result: &Value) -> Value {
    serde_json::from_str(&tool_text(result)).unwrap_or(Value::Null)
}

/// A wait tool's outcome, or the refusal text when it failed.
fn outcome(result: &Value) -> Value {
    if tool_failed(result) {
        json!({ "error": tool_text(result) })
    } else {
        parsed(result)
    }
}

/// A refused tool's first line, `<code>: <message>`; `ok` when it was not refused.
fn refusal(result: &Value) -> String {
    if !tool_failed(result) {
        return "ok".into();
    }
    tool_text(result)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// A spawn refused at Max live children 0, whose message names the caller's live children.
async fn crowd(mcp: &mut McpDriver) -> String {
    refusal(
        &mcp.call(
            "spawn_agent",
            json!({ "prompt": "E2E_COUNT", "title": "Count" }),
        )
        .await,
    )
}

async fn wake_when(j: &mut Journey, mcp: &mut McpDriver, args: Value, alias: &str) -> String {
    let result = mcp.call("wake_when", args).await;
    assert!(
        !tool_failed(&result),
        "wake_when refused: {}",
        tool_text(&result)
    );
    let id = text(&parsed(&result), "/wake/id");
    j.alias(&id, alias);
    id
}

async fn woken(c: &RpcClient, session_id: &str, wake_id: &str) -> (String, String) {
    let header = format!("[from hyprlayer · wake {wake_id}]");
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        if let Some((author, _, text)) = user_turns(c, session_id)
            .await
            .into_iter()
            .find(|t| t.2.starts_with(&header))
        {
            return (author, text);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Default::default()
}

async fn whoami(mcp: &mut McpDriver) -> Value {
    parsed(&mcp.call("whoami", json!({})).await)
}

/// `prompt_and_wait` into `target`: its outcome's state, or the refusal.
async fn ping(mcp: &mut McpDriver, target: &str, message: &str) -> String {
    let result = mcp
        .call(
            "prompt_and_wait",
            json!({ "session_id": target, "text": message, "timeout_sec": WAIT_SECS }),
        )
        .await;
    if tool_failed(&result) {
        refusal(&result)
    } else {
        text(&parsed(&result), "/state")
    }
}

async fn wait_idle(mcp: &mut McpDriver, target: &str) {
    mcp.call(
        "wait_agent",
        json!({ "session_id": target, "timeout_sec": WAIT_SECS }),
    )
    .await;
}

/// The `OpenSessionTab` for `session_id`, as its wire JSON.
async fn open_tab(rx: &mut Receiver<RuntimeEvent>, session_id: &str) -> Option<Value> {
    let is_tab = |e: &RuntimeEvent| {
        matches!(
            e,
            RuntimeEvent::UiRequested {
                request: UiRequest::OpenSessionTab { session_id: s, .. }
            } if s == session_id
        )
    };
    events_until(rx, IO, is_tab)
        .await
        .into_iter()
        .find(|e| is_tab(e))
        .and_then(|e| match e {
            RuntimeEvent::UiRequested { request } => serde_json::to_value(request).ok(),
            _ => None,
        })
}

async fn open_tabs(rx: &mut Receiver<RuntimeEvent>, within: Duration) -> usize {
    events_until(rx, within, |_| false)
        .await
        .iter()
        .filter(|e| {
            matches!(
                e,
                RuntimeEvent::UiRequested {
                    request: UiRequest::OpenSessionTab { .. }
                }
            )
        })
        .count()
}

/// `Notify` requests heard over `within`, as wire JSON with the body cut to its sender.
async fn notifies(rx: &mut Receiver<RuntimeEvent>, within: Duration) -> Vec<Value> {
    events_until(rx, within, |_| false)
        .await
        .into_iter()
        .filter_map(|e| match e {
            RuntimeEvent::UiRequested {
                request: request @ UiRequest::Notify { .. },
            } => serde_json::to_value(request).ok(),
            _ => None,
        })
        .collect()
}

fn drain(rx: &mut Receiver<RuntimeEvent>) {
    while rx.try_recv().is_ok() {}
}

/// Each user turn's author id, author device and text.
async fn user_turns(c: &RpcClient, session_id: &str) -> Vec<(String, String, String)> {
    c.get_session_log(session_id.to_string())
        .await
        .expect("get_session_log")
        .into_iter()
        .filter_map(|e| match e.event {
            SessionEvent::UserMessage { text, author, .. } => {
                let (id, device) = author.map(|a| (a.client_id, a.device)).unwrap_or_default();
                Some((id, device, text))
            }
            _ => None,
        })
        .collect()
}

/// The user messages the stub read, as their content.
fn stub_user_lines(sandbox: &Sandbox, session_id: &str) -> Vec<String> {
    std::fs::read_to_string(
        sandbox
            .root
            .join("goal-control")
            .join(session_id)
            .join("stdin.log"),
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    .filter(|line| line["type"] == "user")
    .filter_map(|line| line["message"]["content"].as_str().map(str::to_string))
    .collect()
}

async fn rows(j: &mut Journey, c: &RpcClient, ws: &str) -> Vec<Value> {
    j.call(
        c,
        Request::ListSessions {
            workstream_id: ws.to_string(),
        },
    )
    .await
    .expect("list_sessions")
    .as_array()
    .cloned()
    .unwrap_or_default()
}

async fn session_row(j: &mut Journey, c: &RpcClient, ws: &str, id: &str) -> Value {
    rows(j, c, ws)
        .await
        .into_iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or(Value::Null)
}

async fn session_count(c: &RpcClient, ws: &str) -> usize {
    c.list_sessions(ws.to_string())
        .await
        .expect("list_sessions")
        .len()
}

async fn live(c: &RpcClient, id: &str) -> bool {
    c.list_active_sessions()
        .await
        .expect("list_active_sessions")
        .iter()
        .any(|a| a.session.id == id)
}

async fn gone(c: &RpcClient, id: &str) -> bool {
    let deadline = tokio::time::Instant::now() + IO;
    while tokio::time::Instant::now() < deadline {
        if !live(c, id).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

fn goal_spec() -> GoalSpec {
    GoalSpec {
        condition: "E2E_DESKTOP_GOAL".into(),
        work_prompt: "Raise the counter.".into(),
        source: GoalSource::Composer,
        source_ref: None,
        source_phase: None,
        checks: vec![GoalCheck::Command {
            command: CHECK.into(),
            timeout_secs: 30,
            label: None,
            cwd: None,
            phases: vec![],
        }],
        limits: GoalLimits {
            max_iterations: 2,
            max_cost_usd: Some(10.0),
            max_tokens: None,
            max_active_secs: 600,
        },
    }
}

/// The goal's status once it stops running, or `None` if it never does.
async fn wait_goal(c: &RpcClient, goal_id: &str) -> Option<GoalStatus> {
    let deadline = tokio::time::Instant::now() + GOAL_WAIT;
    while tokio::time::Instant::now() < deadline {
        let detail: GoalDetail = c.get_goal(goal_id.to_string()).await.expect("get_goal");
        if detail.goal.status != GoalStatus::Running {
            return Some(detail.goal.status);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    None
}
