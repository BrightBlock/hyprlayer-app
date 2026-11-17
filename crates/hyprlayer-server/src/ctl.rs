//! `hyprlayer-server ctl <verb>`: the agent control plane from a shell, over the same tools as `mcp`.

use serde_json::{json, Map, Value};

use crate::mcp::{call_tool, describe, not_an_agent, AgentEnv};

const HELP: &str = "usage: hyprlayer-server ctl <verb> [--json] [args]

Run from inside an agent hyprlayer launched; it reads HYPRLAYER_SOCKET and HYPRLAYER_AGENT_TOKEN.

verbs:
  whoami                                  this agent's session or terminal, workstream and level
  workstreams                             open workstreams in this project
  agents                                  live sessions and PTY agents in this project
  turns <session> [--limit n]             a session's latest user and assistant turns
  read-terminal <terminal> [--lines n]    a PTY agent's screen as plain text
  docs [--workstream id]                  a workstream's attached, written and branch documents
  doc <id>                                a document's content
  pr [--workstream id]                    the workstream's pull request status
  git [--workstream id]                   the workstream's git status
  script-output <kind> [--workstream id]  the last setup, run or archive script output
  notify <title> <body>                   a desktop notification
  attention <reason>                      tell the user this agent needs them

At agent control Full:
  spawn-agent <prompt> [--workstream id] [--stage s] [--harness h] [--model m] [--effort e]
              [--mode m] [--doc path]... [--title t] [--engine id] [--bypass]
                                          start a background agent; --engine starts a PTY agent
  wait-agent <session> [--until s] [--timeout secs]
                                          wait for idle, needs_you or exited; prints its last reply
  message-agent <session> <text>          send an agent a message from this agent
  prompt-and-wait <session> <text> [--until s] [--timeout secs]
                                          message-agent, then wait-agent
  stop-agent <session>                    interrupt an agent this agent spawned
  kill-agent <session>                    end an agent this agent spawned
  archive-agent <session>                 archive a session this agent spawned
  answer <session> <request> <answers>    answer its question; answers is a JSON object
  title <title> [--session id]            rename this agent's session or one it spawned
  create-workstream <title> [--base b] [--branch b] [--worktree-mode isolated|existing]
                                          a new workstream; its setup script runs
  approve-plan <plan> [--workstream id] [--harness h] [--model m] [--effort e] [--mode m] [--title t]
                                          approve a plan and build it in a background session
  validate-plan <plan> [--workstream id] [--harness h] [--model m] [--effort e] [--title t]
                                          validate a plan in a background session
  start-goal <condition> <work-prompt> --checks <json> [--limits json] [--session id]
             [--new-session json]          repeat work until the checks pass
  stop-goal <goal>                        stop a goal on this agent or one it spawned
  run-script <kind> [--workstream id]     run the setup or run script
  stop-script [--workstream id]           stop the run script
  comment <path> <line> <body> [--side s] [--anchor text] [--reply-to id] [--workstream id]
                                          add a review comment
  resolve-comment <comment> [--workstream id]
                                          resolve a review comment
  reveal <doc|file|diff> <path>           open it in a background tab on the user's desktops

--terminal <id> in place of <session> targets a PTY agent: turns, wait-agent, message-agent,
prompt-and-wait, stop-agent, kill-agent.

--json prints the raw result. Exit codes: 0 ok, 1 refused or no server, 2 usage.";

#[derive(Clone, Copy)]
enum FlagValue {
    Text,
    Number,
    Json,
    /// Repeatable; collects into an array.
    List,
    /// Takes no value; present means true.
    Switch,
}

const FLAGS: [(&str, &str, FlagValue); 25] = [
    ("--limit", "limit", FlagValue::Number),
    ("--workstream", "workstream_id", FlagValue::Text),
    ("--stage", "stage", FlagValue::Text),
    ("--harness", "harness", FlagValue::Text),
    ("--model", "model", FlagValue::Text),
    ("--effort", "effort", FlagValue::Text),
    ("--mode", "permission_mode", FlagValue::Text),
    ("--doc", "attached_docs", FlagValue::List),
    ("--title", "title", FlagValue::Text),
    ("--until", "until", FlagValue::Text),
    ("--timeout", "timeout_sec", FlagValue::Number),
    ("--session", "session_id", FlagValue::Text),
    ("--engine", "terminal_engine", FlagValue::Text),
    ("--bypass", "bypass", FlagValue::Switch),
    ("--terminal", "terminal_id", FlagValue::Text),
    ("--lines", "lines", FlagValue::Number),
    ("--base", "base", FlagValue::Text),
    ("--branch", "branch", FlagValue::Text),
    ("--worktree-mode", "worktree_mode", FlagValue::Text),
    ("--checks", "checks", FlagValue::Json),
    ("--limits", "limits", FlagValue::Json),
    ("--new-session", "new_session", FlagValue::Json),
    ("--side", "side", FlagValue::Text),
    ("--anchor", "anchor_text", FlagValue::Text),
    ("--reply-to", "parent_id", FlagValue::Text),
];

const TEXT: FlagValue = FlagValue::Text;
const SESSION: (&str, FlagValue) = ("session_id", FlagValue::Text);

struct Parsed {
    tool: &'static str,
    args: Value,
    json: bool,
}

pub async fn run(argv: Vec<String>) -> i32 {
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return 0;
    }
    let parsed = match parse(argv) {
        Ok(parsed) => parsed,
        Err(usage) => {
            eprintln!("hyprlayer-server ctl: {usage}\n\n{HELP}");
            return 2;
        }
    };
    let Some(env) = AgentEnv::from_env() else {
        eprintln!("hyprlayer-server ctl: {}", not_an_agent().message);
        return 2;
    };
    let result = match env.connect().await {
        Ok(agent) => call_tool(&agent, parsed.tool, &parsed.args).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(value) => {
            if parsed.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            } else {
                println!("{}", human(parsed.tool, &value));
            }
            0
        }
        Err(e) => {
            eprintln!("{}", describe(&e));
            1
        }
    }
}

fn parse(argv: Vec<String>) -> Result<Parsed, String> {
    let mut json = false;
    let mut flags = Map::new();
    let mut positional = Vec::new();
    let mut rest = argv.into_iter();
    while let Some(arg) = rest.next() {
        if arg == "--json" {
            json = true;
            continue;
        }
        let Some(&(_, key, kind)) = FLAGS.iter().find(|(flag, ..)| *flag == arg) else {
            if arg.starts_with("--") {
                return Err(format!("unknown flag `{arg}`"));
            }
            positional.push(arg);
            continue;
        };
        if let FlagValue::Switch = kind {
            flags.insert(key.into(), json!(true));
            continue;
        }
        let value = rest.next().ok_or(format!("{arg} needs a value"))?;
        match kind {
            FlagValue::Switch => {}
            FlagValue::Text => {
                flags.insert(key.into(), json!(value));
            }
            FlagValue::Number => {
                let n = value
                    .parse::<u64>()
                    .map_err(|_| format!("{arg} takes a number"))?;
                flags.insert(key.into(), json!(n));
            }
            FlagValue::Json => {
                let parsed = serde_json::from_str::<Value>(&value)
                    .map_err(|_| format!("{arg} takes JSON"))?;
                flags.insert(key.into(), parsed);
            }
            FlagValue::List => {
                let list = flags.entry(key).or_insert_with(|| json!([]));
                if let Value::Array(items) = list {
                    items.push(json!(value));
                }
            }
        }
    }
    let (verb, operands) = positional.split_first().ok_or("missing verb")?;
    let (tool, names): (&'static str, &[(&str, FlagValue)]) = match verb.as_str() {
        "whoami" => ("whoami", &[]),
        "workstreams" => ("list_workstreams", &[]),
        "agents" => ("list_agents", &[]),
        "turns" => ("read_agent", &[SESSION]),
        "docs" => ("list_docs", &[]),
        "doc" => ("read_doc", &[("id", TEXT)]),
        "pr" => ("pr_status", &[]),
        "git" => ("git_status", &[]),
        "script-output" => ("script_output", &[("kind", TEXT)]),
        "notify" => ("notify_user", &[("title", TEXT), ("body", TEXT)]),
        "attention" => ("request_attention", &[("reason", TEXT)]),
        "spawn-agent" => ("spawn_agent", &[("prompt", TEXT)]),
        "wait-agent" => ("wait_agent", &[SESSION]),
        "message-agent" => ("message_agent", &[SESSION, ("text", TEXT)]),
        "prompt-and-wait" => ("prompt_and_wait", &[SESSION, ("text", TEXT)]),
        "stop-agent" => ("stop_agent", &[SESSION]),
        "kill-agent" => ("kill_agent", &[SESSION]),
        "archive-agent" => ("archive_agent", &[SESSION]),
        "answer" => (
            "answer_question",
            &[SESSION, ("request_id", TEXT), ("answers", FlagValue::Json)],
        ),
        "title" => ("set_agent_title", &[("title", TEXT)]),
        "read-terminal" => ("read_terminal", &[("terminal_id", TEXT)]),
        "create-workstream" => ("create_workstream", &[("title", TEXT)]),
        "approve-plan" => ("approve_plan", &[("plan", TEXT)]),
        "validate-plan" => ("validate_plan", &[("plan", TEXT)]),
        "start-goal" => ("start_goal", &[("condition", TEXT), ("work_prompt", TEXT)]),
        "stop-goal" => ("stop_goal", &[("goal_id", TEXT)]),
        "run-script" => ("run_script", &[("kind", TEXT)]),
        "stop-script" => ("stop_script", &[]),
        "comment" => (
            "comment",
            &[("path", TEXT), ("line", FlagValue::Number), ("body", TEXT)],
        ),
        "resolve-comment" => ("resolve_comment", &[("comment_id", TEXT)]),
        "reveal" => ("reveal", &[("kind", TEXT), ("path", TEXT)]),
        other => return Err(format!("unknown verb `{other}`")),
    };
    // `--terminal` stands in for a positional session.
    let names: Vec<(&str, FlagValue)> = names
        .iter()
        .copied()
        .filter(|(name, _)| !(*name == SESSION.0 && flags.contains_key("terminal_id")))
        .collect();
    if operands.len() != names.len() {
        return Err(format!(
            "{verb} takes {} argument(s), got {}",
            names.len(),
            operands.len()
        ));
    }
    for ((name, kind), value) in names.into_iter().zip(operands) {
        let value = match kind {
            FlagValue::Json => {
                serde_json::from_str::<Value>(value).map_err(|_| format!("{name} must be JSON"))?
            }
            FlagValue::Number => json!(value
                .parse::<u64>()
                .map_err(|_| format!("{name} must be a number"))?),
            _ => json!(value),
        };
        flags.insert(name.into(), value);
    }
    Ok(Parsed {
        tool,
        args: Value::Object(flags),
        json,
    })
}

/// Lists one row per item and prose as itself; anything else stays JSON.
fn human(tool: &str, value: &Value) -> String {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let rows = |items: Option<&Vec<Value>>, cols: &[&str]| -> Vec<String> {
        items
            .into_iter()
            .flatten()
            .map(|item| {
                cols.iter()
                    .map(|c| text(item, c))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect()
    };
    match tool {
        "list_workstreams" => {
            rows(value.as_array(), &["id", "stage", "status", "title"]).join("\n")
        }
        "list_agents" => {
            let mut out = rows(
                value["sessions"].as_array(),
                &["sessionId", "harness", "state", "title"],
            );
            out.extend(rows(
                value["ptyAgents"].as_array(),
                &["terminalId", "engineId", "state", "lastReply"],
            ));
            out.join("\n")
        }
        "read_agent" if value.is_array() => rows(value.as_array(), &["role", "text"]).join("\n\n"),
        "spawn_agent" | "approve_plan" | "validate_plan" => match value.get("terminalId") {
            Some(_) => text(value, "terminalId"),
            None => text(value, "sessionId"),
        },
        "create_workstream" => text(value, "id"),
        "start_goal" | "stop_goal" => text(value, "goalId"),
        "wait_agent" | "prompt_and_wait" => {
            let mut state = text(value, "state");
            for flag in ["stalled", "timedOut"] {
                if value[flag].as_bool() == Some(true) {
                    state.push_str(&format!(" ({flag})"));
                }
            }
            match value["lastReply"].as_str() {
                Some(reply) => format!("{state}\n\n{reply}"),
                None => state,
            }
        }
        "read_doc" => text(value, "content"),
        _ => match value {
            Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        },
    }
}
