//! `hyprlayer-server hook <engine> [Event]`: a claude or codex hook that reports one signal and gets out of the way.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_models::{AgentSignal, ClaudeWorktreeAnswer, ClaudeWorktreeEvent};
use serde_json::Value;

use crate::client::RpcClient;

/// A slow or hung server must never stall the agent's turn.
const REPORT_BUDGET: Duration = Duration::from_millis(500);
/// Leaves 30 s of claude's 120 s `WorktreeCreate` timeout for the local fallback.
const CREATE_BUDGET: Duration = Duration::from_secs(90);
const TOOL_TARGET_CHARS: usize = 80;
const LAST_REPLY_BYTES: usize = 4 * 1024;
const PROMPT_BYTES: usize = 2 * 1024;
/// claude's answer that skips its "Switch model?" dialog.
const ALLOW_SWITCH: &str =
    r#"{"hookSpecificOutput":{"hookEventName":"PreModelSwitch","permissionDecision":"allow"}}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEngine {
    Claude,
    Codex,
}

impl HookEngine {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(HookEngine::Claude),
            "codex" => Some(HookEngine::Codex),
            _ => None,
        }
    }
}

/// Never logs, and exits 0 unless `WorktreeCreate` can make no worktree at all: a failing hook
/// would surface in the agent's TUI.
pub async fn run_hook(engine: HookEngine, event: Option<String>) -> i32 {
    // codex parses stdout as JSON after exit 0; a closed stdout must not panic.
    let ack = || {
        if engine == HookEngine::Codex {
            let _ = writeln!(std::io::stdout(), "{{}}");
        }
    };
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let (socket, token) = (env("HYPRLAYER_SOCKET"), env("HYPRLAYER_AGENT_TOKEN"));
    match (engine, event.as_deref()) {
        // claude takes the last stdout line as the worktree, so even a tokenless hook must answer.
        (HookEngine::Claude, Some("WorktreeCreate")) => {
            return create_worktree(socket, token).await
        }
        (HookEngine::Claude, Some(done @ ("WorktreeRemove" | "SubagentStop"))) => {
            return worktree_done(done, socket, token).await;
        }
        _ => {}
    }
    let (Some(token), Some(socket)) = (token, socket) else {
        ack();
        return 0;
    };
    if engine == HookEngine::Claude && event.as_deref() == Some("PreModelSwitch") {
        let _ = read_stdin();
        return allow_driven_switch(PathBuf::from(socket), token).await;
    }
    let Some(signal) = read_stdin().and_then(|raw| trim(&raw, engine, event)) else {
        ack();
        return 0;
    };
    let _ = tokio::time::timeout(REPORT_BUDGET, report(PathBuf::from(socket), token, signal)).await;
    ack();
    0
}

async fn report(socket: PathBuf, token: String, signal: AgentSignal) {
    let Ok(client) = RpcClient::connect_socket(&socket).await else {
        return;
    };
    drop(client.take_events());
    let _ = client.report_agent_signal(token, signal).await;
}

/// Allows only a switch the status bar is driving; silence leaves the user's own switches to claude.
async fn allow_driven_switch(socket: PathBuf, token: String) -> i32 {
    let ask = async {
        let client = RpcClient::connect_socket(&socket).await.ok()?;
        drop(client.take_events());
        client.model_switch_pending(token).await.ok()
    };
    if let Ok(Some(true)) = tokio::time::timeout(REPORT_BUDGET, ask).await {
        let _ = writeln!(std::io::stdout(), "{ALLOW_SWITCH}");
    }
    0
}

async fn create_worktree(socket: Option<String>, token: Option<String>) -> i32 {
    let payload: Option<Value> = read_stdin().and_then(|raw| serde_json::from_slice(&raw).ok());
    let text = |key: &str| payload.as_ref()?.get(key)?.as_str().map(str::to_string);
    let (Some(name), Some(cwd)) = (text("name"), text("cwd")) else {
        eprintln!("hyprlayer: the WorktreeCreate payload has no name or cwd");
        return 1;
    };
    let event = ClaudeWorktreeEvent::Create {
        name: name.clone(),
        cwd: cwd.clone(),
        session_id: text("session_id"),
    };
    let ask = ask_worktree(socket, token, event);
    let path = match tokio::time::timeout(CREATE_BUDGET, ask).await {
        Ok(Some(ClaudeWorktreeAnswer::Path(path))) => path,
        _ => match claude_layout_worktree(Path::new(&cwd), &name) {
            Ok(path) => path,
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        },
    };
    let _ = writeln!(std::io::stdout(), "{path}");
    0
}

async fn worktree_done(event: &str, socket: Option<String>, token: Option<String>) -> i32 {
    let payload: Option<Value> = read_stdin().and_then(|raw| serde_json::from_slice(&raw).ok());
    let text = |key: &str| payload.as_ref()?.get(key)?.as_str().map(str::to_string);
    let (event, removed) = match (event, text("worktree_path"), text("agent_id")) {
        ("WorktreeRemove", Some(worktree_path), _) => (
            ClaudeWorktreeEvent::Remove {
                worktree_path: worktree_path.clone(),
            },
            Some(worktree_path),
        ),
        ("SubagentStop", _, Some(agent_id)) => {
            (ClaudeWorktreeEvent::SubagentStop { agent_id }, None)
        }
        _ => return 0,
    };
    let ask = ask_worktree(socket, token, event);
    let answer = tokio::time::timeout(REPORT_BUDGET, ask)
        .await
        .ok()
        .flatten();
    if let Some(path) = removed.filter(|_| !matches!(answer, Some(ClaudeWorktreeAnswer::Path(_)))) {
        remove_claude_layout(Path::new(&path));
    }
    0
}

async fn ask_worktree(
    socket: Option<String>,
    token: Option<String>,
    event: ClaudeWorktreeEvent,
) -> Option<ClaudeWorktreeAnswer> {
    let client = RpcClient::connect_socket(Path::new(&socket?)).await.ok()?;
    drop(client.take_events());
    client.claude_worktree(token?, event).await.ok()
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    match out.status.success() {
        true => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// With a create hook installed claude removes neither the worktree nor its branch; `-d` keeps an
/// unmerged branch.
fn remove_claude_layout(path: &Path) {
    let layout = path.parent().filter(|p| p.ends_with(".claude/worktrees"));
    let (Some(top), Some(name)) = (layout.and_then(|p| p.parent()?.parent()), path.file_name())
    else {
        return;
    };
    let path = path.to_string_lossy();
    let _ = git(top, &["worktree", "remove", "--force", &path]);
    let _ = git(
        top,
        &[
            "branch",
            "-d",
            &format!("worktree-{}", name.to_string_lossy()),
        ],
    );
}

/// The worktree claude makes with no hook installed: `worktree-<name>` off the origin default.
fn claude_layout_worktree(cwd: &Path, name: &str) -> Result<String, String> {
    let top = PathBuf::from(git(cwd, &["rev-parse", "--show-toplevel"])?);
    let path = top.join(".claude").join("worktrees").join(name);
    let path_str = path.to_string_lossy().into_owned();
    if path.is_dir() {
        return Ok(path_str);
    }
    let branch = format!("worktree-{name}");
    let base =
        git(&top, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).unwrap_or_else(|_| "HEAD".into());
    let local = format!("refs/heads/{branch}");
    let add: Vec<&str> = match git(&top, &["rev-parse", "--verify", "--quiet", &local]) {
        Ok(_) => vec!["worktree", "add", &path_str, &branch],
        Err(_) => vec!["worktree", "add", "-b", &branch, &path_str, &base],
    };
    git(&top, &add)?;
    Ok(path_str)
}

pub(crate) fn read_stdin() -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    std::io::stdin().read_to_end(&mut raw).ok()?;
    Some(raw)
}

/// The fields the fold reads, cut to size; `None` for a payload that isn't a JSON object.
fn trim(raw: &[u8], engine: HookEngine, event: Option<String>) -> Option<AgentSignal> {
    let payload: Value = serde_json::from_slice(raw).ok()?;
    let payload = payload.as_object()?;
    let text = |key: &str| payload.get(key)?.as_str().map(str::to_string);
    let event = event.or_else(|| text("hook_event_name"))?;
    // `StopFailure`'s kind (`authentication_failed`), else the first line of its details.
    let failure = || {
        text("error")
            .or_else(|| text("error_details"))
            .and_then(|e| {
                e.lines()
                    .next()
                    .map(|l| l.trim().chars().take(TOOL_TARGET_CHARS).collect())
            })
    };
    let tool_target = payload.get("tool_input").and_then(|input| {
        ["command", "file_path", "pattern", "url"]
            .iter()
            .find_map(|key| input.get(key)?.as_str())
            .map(|target| target.chars().take(TOOL_TARGET_CHARS).collect())
    });
    Some(AgentSignal {
        event,
        session_id: text("session_id"),
        source: text("source"),
        reason: text("reason").or_else(failure),
        tool_name: text("tool_name"),
        tool_target,
        notification_type: text("notification_type"),
        last_assistant_message: text("last_assistant_message")
            .map(|m| truncate(m, LAST_REPLY_BYTES)),
        turn_id: text("turn_id"),
        prompt: text("prompt").map(|p| truncate(p, PROMPT_BYTES)),
        permission_mode: text("permission_mode"),
        effort: payload
            .get("effort")
            .and_then(|e| e.get("level")?.as_str())
            .map(str::to_string),
        model: text("model"),
        to_model: text("to_model"),
        // Only codex's rollout carries what its hooks lack.
        transcript_path: text("transcript_path").filter(|_| engine == HookEngine::Codex),
        written_paths: written_paths(payload),
    })
}

fn written_paths(payload: &serde_json::Map<String, Value>) -> Vec<String> {
    let tool = payload.get("tool_name").and_then(Value::as_str);
    let (Some("Write" | "Edit" | "MultiEdit"), Some(input)) = (tool, payload.get("tool_input"))
    else {
        return Vec::new();
    };
    let cwd = payload.get("cwd").and_then(Value::as_str).map(Path::new);
    let edits = input.get("edits").and_then(Value::as_array);
    let mut paths: Vec<String> = std::iter::once(input.get("file_path"))
        .chain(edits.into_iter().flatten().map(|e| e.get("file_path")))
        .filter_map(|p| p?.as_str())
        // Without its `cwd` a relative path would resolve against the server's.
        .filter_map(|p| match cwd {
            Some(cwd) => Some(cwd.join(p).to_string_lossy().into_owned()),
            None => Path::new(p).is_absolute().then(|| p.to_string()),
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn truncate(mut s: String, max_bytes: usize) -> String {
    if s.len() > max_bytes {
        let cut = (0..=max_bytes)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        s.truncate(cut);
    }
    s
}

const MUTATING_GIT: [&str; 24] = [
    "add",
    "am",
    "apply",
    "checkout",
    "checkout-index",
    "cherry-pick",
    "clean",
    "commit",
    "merge",
    "mv",
    "pull",
    "push",
    "read-tree",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "stash",
    "switch",
    "symbolic-ref",
    "update-index",
    "update-ref",
    "worktree",
];

const WORKTREE_READS: [&str; 1] = ["list"];
const BRANCH_WRITES: [&str; 12] = [
    "-d", "-D", "-m", "-M", "-c", "-C", "-f", "--delete", "--move", "--copy", "--force", "-u",
];
const PREFIXES: [&str; 7] = ["env", "command", "exec", "nohup", "time", "builtin", "eval"];
const KEYWORDS: [&str; 8] = ["!", "if", "then", "else", "elif", "do", "while", "until"];
const SHELLS: [&str; 5] = ["sh", "bash", "zsh", "dash", "ksh"];

/// `hyprlayer-server lead-guard <checkout>`: a Lead's PreToolUse hook that refuses writes to its
/// main checkout. It reads the shell forms a model writes: git and file-writing commands, redirects
/// and paths through symlinks. Code run by an interpreter (`python -c`, `node -e`, `perl -e`) is
/// not read, so the Lead's prompt rules that out.
pub fn run_lead_guard(checkout: &str) -> i32 {
    let env = |name: &str| std::env::var(name).ok();
    let reason = read_stdin().map_or_else(
        || Some(UNREADABLE.to_string()),
        |raw| refusal_for(&raw, Path::new(checkout), &env),
    );
    if let Some(reason) = reason {
        let answer = serde_json::json!({ "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        } });
        let _ = writeln!(std::io::stdout(), "{answer}");
    }
    0
}

const UNREADABLE: &str = "The Lead's guard could not read this tool call, so it is refused.";

fn refusal_for(raw: &[u8], checkout: &Path, env: &Env) -> Option<String> {
    match serde_json::from_slice::<Value>(raw) {
        Ok(payload) => lead_refusal(&payload, checkout, env),
        Err(_) => Some(UNREADABLE.to_string()),
    }
}

/// The hook runs in claude's environment, which its Bash tool shares, so `env` stands in for
/// variables the command does not assign itself.
type Env = dyn Fn(&str) -> Option<String>;

fn lead_refusal(payload: &Value, checkout: &Path, env: &Env) -> Option<String> {
    let checkout = resolve(checkout);
    let input = payload.get("tool_input");
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map_or(checkout.clone(), PathBuf::from);
    let refuse = |what: &str| {
        Some(format!(
            "The Lead writes no code: {what} in the main checkout is refused. \
Create a workstream and spawn an agent to make the change."
        ))
    };
    let Some(tool) = payload.get("tool_name").and_then(Value::as_str) else {
        return Some(UNREADABLE.to_string());
    };
    match tool {
        "Edit" | "Write" | "NotebookEdit" => {
            let Some(path) = ["file_path", "notebook_path"]
                .iter()
                .find_map(|key| input?.get(key)?.as_str())
            else {
                return Some(UNREADABLE.to_string());
            };
            under(&cwd.join(path), &checkout).then(|| refuse("a file edit"))?
        }
        "Bash" => {
            let Some(command) = input.and_then(|i| i.get("command")).and_then(Value::as_str) else {
                return Some(UNREADABLE.to_string());
            };
            let shell = Shell {
                env,
                vars: HashMap::new(),
            };
            bash_refusal(command, &cwd, &checkout, shell).and_then(|what| refuse(&what))
        }
        _ => None,
    }
}

fn bash_refusal(command: &str, cwd: &Path, checkout: &Path, mut shell: Shell) -> Option<String> {
    let mut at = cwd.to_path_buf();
    let mut previous: Option<PathBuf> = None;
    let mut pushed: Vec<PathBuf> = Vec::new();
    for segment in segments(command) {
        for target in redirect_targets(&segment) {
            match shell.resolve(&at, target) {
                Some(path) if path == Path::new("/dev/null") || !under(&path, checkout) => {}
                Some(_) => return Some("a shell redirect".into()),
                None => {
                    return Some(format!(
                        "a shell redirect to `{target}`, which the guard cannot resolve,"
                    ))
                }
            }
        }
        let words: Vec<String> = segment
            .split_whitespace()
            .map(|w| w.trim_matches(['(', ')', '{', '}']).to_string())
            .filter(|w| !w.is_empty() && !w.contains('>'))
            .collect();
        if let Some(assigned) = assignments(&words) {
            for (name, value) in assigned {
                let value = shell.expand(value);
                shell.vars.insert(name.to_string(), value);
            }
            continue;
        }
        let words = program_words(&words);
        let Some((program, args)) = words.split_first() else {
            continue;
        };
        let program = unquote(program);
        let program = Path::new(program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(program);
        let into_checkout = |word: &str| {
            shell
                .resolve(&at, word)
                .is_none_or(|path| under(&path, checkout))
        };
        match program {
            "cd" | "pushd" => {
                let dir = args.iter().find(|a| *a == "-" || !a.starts_with('-'));
                // An unknown directory may be the checkout.
                let to = match dir.map(String::as_str) {
                    Some("-") => Some(previous.clone().unwrap_or_else(|| checkout.to_path_buf())),
                    Some(dir) => match shell.resolve(&at, dir) {
                        Some(path) => Some(path).filter(|d| d.is_dir()),
                        None => Some(checkout.to_path_buf()),
                    },
                    None => shell.lookup("HOME").map(PathBuf::from),
                };
                if let Some(to) = to {
                    if program == "pushd" {
                        pushed.push(at.clone());
                    }
                    previous = Some(std::mem::replace(&mut at, to));
                }
            }
            "popd" => {
                if let Some(to) = pushed.pop() {
                    previous = Some(std::mem::replace(&mut at, to));
                }
            }
            "git" => {
                let (repo, sub, rest) = git_target(&shell, &at, args);
                let in_checkout = repo.is_none_or(|r| under(&r, checkout));
                if in_checkout && sub.is_some_and(|s| git_writes(s, rest)) {
                    return Some(format!("`git {}`", sub.unwrap_or_default()));
                }
            }
            _ => {
                if shell_writes(program, args).into_iter().any(into_checkout) {
                    return Some(format!("`{program}`"));
                }
            }
        }
    }
    None
}

/// Splits on `;`, newlines, `&` and `|`, except the `|` and `&` of `>|`, `>&` and `&>`.
fn segments(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut out = vec![String::new()];
    for (i, &c) in chars.iter().enumerate() {
        let glued = i > 0 && chars[i - 1] == '>' || chars.get(i + 1) == Some(&'>');
        match c {
            ';' | '\n' => out.push(String::new()),
            '&' | '|' if !glued => out.push(String::new()),
            _ => out.last_mut().expect("never empty").push(c),
        }
    }
    out
}

fn redirect_targets(segment: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = segment;
    while let Some(at) = rest.find('>') {
        let mut after = rest[at + 1..].trim_start_matches('>');
        let dup = after.starts_with('&');
        after = after.trim_start_matches(['|', '&']);
        let skip = after.len();
        if dup && after.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
            rest = &rest[rest.len() - skip..];
            continue;
        }
        let after = after.trim_start();
        let end = after
            .find(|c: char| c.is_whitespace() || "<>;|&()".contains(c))
            .unwrap_or(after.len());
        let target = &after[..end];
        if !target.is_empty() {
            targets.push(target);
        }
        rest = &after[end..];
    }
    targets
}

/// Drops what runs a command rather than being it: `VAR=x`, `env -u X`, `command`, `sh -c`.
fn program_words(words: &[String]) -> &[String] {
    let mut i = 0;
    while let Some(word) = words.get(i) {
        let assignment = word.split_once('=').is_some_and(|(name, _)| is_name(name));
        if assignment || PREFIXES.contains(&word.as_str()) || KEYWORDS.contains(&word.as_str()) {
            i += 1;
        } else if word == "-u" && i > 0 && words[i - 1] == "env" {
            i += 2;
        } else if word.starts_with('-') && i > 0 && PREFIXES.contains(&words[i - 1].as_str()) {
            i += 1;
        } else if SHELLS.contains(&word.as_str()) && words.get(i + 1).is_some_and(|w| w == "-c") {
            i += 2;
        } else {
            break;
        }
    }
    &words[i.min(words.len())..]
}

fn git_writes(sub: &str, rest: &[String]) -> bool {
    match sub {
        "worktree" => rest
            .first()
            .is_some_and(|w| !WORKTREE_READS.contains(&w.as_str())),
        "branch" => rest.iter().any(|w| BRANCH_WRITES.contains(&w.as_str())),
        _ => MUTATING_GIT.contains(&sub),
    }
}

/// The paths a file-writing command changes, for the commands a model reaches for.
fn shell_writes<'a>(program: &str, args: &'a [String]) -> Vec<&'a str> {
    let positional = |skip_value: &[&str]| {
        let mut out = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if skip_value.contains(&arg.as_str()) {
                args.next();
            } else if !arg.starts_with('-') {
                out.push(arg.as_str());
            }
        }
        out
    };
    match program {
        "tee" | "rm" | "rmdir" | "touch" | "mkdir" | "mv" | "shred" | "unlink" => positional(&[]),
        "truncate" => positional(&["-s", "-r"]),
        "cp" | "ln" | "install" | "rsync" => {
            let target = args
                .iter()
                .position(|a| a == "-t" || a == "--target-directory")
                .and_then(|i| args.get(i + 1));
            target.map_or_else(
                || positional(&[]).last().copied().into_iter().collect(),
                |t| vec![t.as_str()],
            )
        }
        "sed"
            if args
                .iter()
                .any(|a| a.starts_with("-i") || a.starts_with("--in-place")) =>
        {
            let scripted = args.iter().any(|a| a == "-e" || a == "-f");
            let mut files = positional(&["-e", "-f"]);
            if !scripted && !files.is_empty() {
                files.remove(0);
            }
            files
        }
        "dd" => args.iter().filter_map(|a| a.strip_prefix("of=")).collect(),
        _ => Vec::new(),
    }
}

/// The repo a git command runs in, `None` when it can't be known, its subcommand and the rest.
fn git_target<'a>(
    shell: &Shell,
    at: &Path,
    args: &'a [String],
) -> (Option<PathBuf>, Option<&'a str>, &'a [String]) {
    let mut repo = Some(at.to_path_buf());
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "-C" => {
                let dir = args.get(i + 1).map_or("", String::as_str);
                repo = repo.and_then(|r| shell.resolve(&r, dir));
                i += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
            flag if flag.starts_with('-') => i += 1,
            sub => return (repo, Some(unquote(sub)), &args[i + 1..]),
        }
    }
    (repo, None, &[])
}

struct Shell<'a> {
    env: &'a Env,
    /// A shell variable's value; `None` when the guard cannot know it.
    vars: HashMap<String, Option<String>>,
}

impl Shell<'_> {
    /// A word as bash would expand it, or `None` when part of it can't be known: a command
    /// substitution, `~user`, a parameter operator, or a variable set nowhere the guard can see.
    fn expand(&self, word: &str) -> Option<String> {
        if let Some(literal) = word.strip_prefix('\'').and_then(|w| w.strip_suffix('\'')) {
            return Some(literal.to_string());
        }
        let word = unquote(word);
        if word.contains("$(") || word.contains('`') {
            return None;
        }
        let mut out = String::new();
        let mut rest = match word.strip_prefix('~') {
            Some(after) if after.is_empty() || after.starts_with('/') => {
                out.push_str(&self.lookup("HOME")?);
                after
            }
            Some(_) => return None,
            None => word,
        };
        while let Some(at) = rest.find('$') {
            out.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            let (name, tail) = match after.strip_prefix('{') {
                Some(braced) => braced.split_at(braced.find('}')?),
                None => after.split_at(
                    after
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .unwrap_or(after.len()),
                ),
            };
            if !is_name(name) {
                return None;
            }
            out.push_str(&self.lookup(name)?);
            rest = tail.strip_prefix('}').unwrap_or(tail);
        }
        out.push_str(rest);
        Some(out)
    }

    fn lookup(&self, name: &str) -> Option<String> {
        match self.vars.get(name) {
            Some(value) => value.clone(),
            None => (self.env)(name),
        }
    }

    fn resolve(&self, at: &Path, word: &str) -> Option<PathBuf> {
        Some(at.join(self.expand(word)?))
    }
}

fn is_name(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A segment that only assigns variables, which then last for the rest of the command.
fn assignments(words: &[String]) -> Option<Vec<(&str, &str)>> {
    let words = match words.first().map(String::as_str) {
        Some("export" | "local" | "declare" | "readonly") => &words[1..],
        _ => words,
    };
    if words.is_empty() {
        return None;
    }
    words
        .iter()
        .map(|w| w.split_once('=').filter(|(name, _)| is_name(name)))
        .collect()
}

fn unquote(word: &str) -> &str {
    word.trim_matches(['"', '\''])
}

/// Lexically normalised, then resolved through its nearest existing ancestor, so a symlink or
/// another letter case on a case-insensitive filesystem names the same checkout.
fn resolve(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                normal.pop();
            }
            std::path::Component::CurDir => {}
            other => normal.push(other),
        }
    }
    let mut missing = Vec::new();
    let mut base = normal.as_path();
    loop {
        if let Ok(real) = base.canonicalize() {
            return missing.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                base = parent;
            }
            _ => return normal,
        }
    }
}

fn under(path: &Path, checkout: &Path) -> bool {
    resolve(path).starts_with(checkout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn checkout() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("README.md"), "x").unwrap();
        let repo = repo.canonicalize().unwrap();
        (dir, repo)
    }

    fn bash(command: &str, cwd: &Path, checkout: &Path) -> Option<String> {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "cwd": cwd,
            "tool_input": { "command": command },
        });
        lead_refusal(&payload, checkout, &|name| std::env::var(name).ok())
    }

    #[test]
    fn git_that_is_not_the_first_word_is_refused() {
        let (_dir, repo) = checkout();
        for command in [
            "FOO=1 git commit -m x",
            "env git commit -m x",
            "env -u HOME FOO=1 git commit -m x",
            "command git commit -m x",
            "/usr/bin/git commit -m x",
            "(git commit -m x)",
            "{ git commit -m x; }",
            "sh -c \"git commit -m x\"",
            "bash -c 'cd src && git commit -m x'",
            "nohup git push",
        ] {
            assert!(bash(command, &repo, &repo).is_some(), "{command}");
        }
    }

    #[test]
    fn git_inside_shell_keywords_is_refused() {
        let (_dir, repo) = checkout();
        for command in [
            "if true; then git commit -m x; fi",
            "if false; then :; else git commit -m x; fi",
            "for f in a; do git add $f; done",
            "while true; do git push; break; done",
            "! git commit -m x",
        ] {
            assert!(bash(command, &repo, &repo).is_some(), "{command}");
        }
    }

    #[test]
    fn the_directory_a_cd_leaves_the_shell_in_is_tracked() {
        let (dir, repo) = checkout();
        let outside = dir.path().canonicalize().unwrap();
        let (repo_s, outside_s) = (repo.display(), outside.display());
        for command in [
            "cd /nonexistent-hyprlayer; git commit -m x".to_string(),
            "cd /nonexistent-hyprlayer 2>/dev/null; echo hi > README.md".to_string(),
            format!("cd {outside_s}; cd -; git commit -m x"),
            format!("pushd {outside_s}; popd; git commit -m x"),
            format!("cd {outside_s} && cd {repo_s} && cd {outside_s} && cd - && git add -A"),
            "cd -; git commit -m x".to_string(),
        ] {
            assert!(bash(&command, &repo, &repo).is_some(), "{command}");
        }
        for command in [
            format!("cd {repo_s}; cd {outside_s}; git commit -m x"),
            format!("pushd {outside_s}; git commit -m x"),
            format!(
                "cd {repo_s} && pushd {outside_s} && popd && pushd {outside_s} && git commit -m x"
            ),
            format!("cd -P {outside_s} && git commit -m x"),
        ] {
            assert_eq!(bash(&command, &repo, &repo), None, "{command}");
        }
    }

    #[test]
    fn ref_and_worktree_plumbing_is_refused() {
        let (_dir, repo) = checkout();
        for command in [
            "git update-ref refs/heads/main HEAD~1",
            "git symbolic-ref HEAD refs/heads/other",
            "git worktree add ../wt",
            "git read-tree -u --reset HEAD~1",
            "git branch -f main HEAD~1",
            "git branch -D old",
        ] {
            assert!(bash(command, &repo, &repo).is_some(), "{command}");
        }
    }

    #[test]
    fn every_redirect_spelling_is_refused() {
        let (_dir, repo) = checkout();
        for command in [
            "echo hi>README.md",
            "echo hi >| README.md",
            "echo hi >&README.md",
            "echo hi &> README.md",
            "echo hi >>README.md",
        ] {
            assert!(bash(command, &repo, &repo).is_some(), "{command}");
        }
    }

    #[test]
    fn file_writing_commands_are_refused() {
        let (_dir, repo) = checkout();
        for command in [
            "sed -i.bak s/a/b/ README.md",
            "sed -i -e s/a/b/ README.md",
            "printf x | tee README.md",
            "cp /etc/hosts README.md",
            "mv README.md /tmp/x",
            "rm README.md",
            "touch new.txt",
            "mkdir src/new",
            "ln -s /etc/hosts link",
            "truncate -s 0 README.md",
            "dd if=/dev/zero of=README.md count=1",
        ] {
            assert!(bash(command, &repo, &repo).is_some(), "{command}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_the_checkout_is_the_checkout() {
        let (dir, repo) = checkout();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&repo, &link).unwrap();
        let outside = dir.path();
        let link = link.display();
        assert!(bash(&format!("echo hi > {link}/README.md"), outside, &repo).is_some());
        assert!(bash(&format!("git -C {link} commit -m x"), outside, &repo).is_some());
        assert!(bash(&format!("cd {link} && git commit -m x"), outside, &repo).is_some());
        assert!(bash(&format!("echo hi > {link}/new/deeper.txt"), outside, &repo).is_some());
    }

    #[test]
    fn a_payload_the_guard_cannot_read_is_refused() {
        let (_dir, repo) = checkout();
        let array = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": ["rm x"]}});
        assert!(lead_refusal(&array, &repo, &no_env).is_some());
        let no_path = serde_json::json!({"tool_name": "Write", "tool_input": {}});
        assert!(lead_refusal(&no_path, &repo, &no_env).is_some());
        assert!(refusal_for(b"not json", &repo, &no_env).is_some());
    }

    #[test]
    fn reads_and_work_outside_the_checkout_pass() {
        let (dir, repo) = checkout();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let elsewhere = elsewhere.canonicalize().unwrap();
        for command in [
            "git status",
            "git log --oneline -5 | head",
            "git diff HEAD~1 > /tmp/x.diff",
            "git branch",
            "git branch --list 'feat/*'",
            "git worktree list",
            "git fetch origin",
            "rg -n foo src 2>&1",
            "cat README.md 2>/dev/null",
            "ls -la >/dev/null 2>&1",
            "gh pr view 1 --json title",
            "echo done >&2",
            "sed -n 1,20p README.md",
            "FOO=1 cargo metadata --format-version 1",
        ] {
            assert_eq!(bash(command, &repo, &repo), None, "{command}");
        }
        let wt = elsewhere.display();
        for command in [
            format!("git -C {wt} commit -m x"),
            format!("cd {wt} && git commit -m x && echo hi > notes.md"),
            format!("cp README.md {wt}/copy.md"),
            format!("sed -i s/a/b/ {wt}/file.txt"),
        ] {
            assert_eq!(bash(&command, &repo, &repo), None, "{command}");
        }
        let edit = serde_json::json!({
            "tool_name": "Edit",
            "cwd": repo,
            "tool_input": {"file_path": elsewhere.join("a.rs")},
        });
        assert_eq!(lead_refusal(&edit, &repo, &no_env), None);
        let other = serde_json::json!({"tool_name": "Read", "tool_input": {"file_path": "x"}});
        assert_eq!(lead_refusal(&other, &repo, &no_env), None);
    }

    fn bash_at_home(command: &str) -> Option<String> {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "cwd": "/home/u/repo",
            "tool_input": { "command": command },
        });
        let env = |name: &str| (name == "HOME").then(|| "/home/u".to_string());
        lead_refusal(&payload, Path::new("/home/u/repo"), &env)
    }

    #[test]
    fn a_redirect_outside_the_checkout_through_home_or_a_variable_goes() {
        for command in [
            "M=/home/u/.claude/memory; cat > $M/fleet.md <<'EOF'\nhi\nEOF",
            "f=~/.claude/settings.json; jq . \"$f\" > \"$f.tmp\" && mv \"$f.tmp\" \"$f\"",
            "echo hi > ~/notes.txt",
            "echo hi > ${HOME}/notes.txt",
            "export D=/tmp/out; echo hi >> \"${D}/log\"",
            "cd /tmp && echo hi > x.txt",
            "ls 2>/dev/null > /tmp/list",
            "git -C ~/other commit -m x",
        ] {
            assert_eq!(bash_at_home(command), None, "{command}");
        }
    }

    #[test]
    fn a_redirect_that_lands_in_the_checkout_or_cannot_be_resolved_is_refused() {
        for command in [
            "echo hi > README.md",
            "echo hi > ~/repo/README.md",
            "R=~/repo; echo hi > $R/src/main.rs",
            "cd ~/repo/src && echo hi > x.rs",
            "echo hi > $UNSET/x",
            "echo hi > \"$(mktemp)\"",
            "echo hi > ${R:-/tmp}/x",
            "echo hi > ~someone/x",
            "D=$(pwd); echo hi > $D/x",
            "cd $SOMEWHERE && echo hi > x",
            "git -C ~/repo commit -m x",
            "git -C $UNSET commit -m x",
        ] {
            assert!(bash_at_home(command).is_some(), "{command}");
        }
    }
}
