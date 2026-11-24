//! How each PTY engine starts, resumes and bypasses a provider session (ported from the desktop's `engines.ts`).

use hyprlayer_models::{Harness, PtyLaunchSession};

use crate::settings::Engine;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionMode {
    /// The launch names the session id up front.
    Assign,
    /// The session id comes from a `codex exec` run before the launch.
    ExecMint,
    Capture,
    None,
}

struct EngineSession {
    mode: SessionMode,
    fresh: Option<fn(&str) -> Vec<String>>,
    resume_by_id: Option<fn(&str) -> Vec<String>>,
    resume_latest: &'static [&'static str],
    bypass: &'static [&'static str],
    model: Option<&'static str>,
    permission_mode: Option<&'static str>,
    effort: Option<&'static str>,
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn engine_session(engine_id: &str) -> EngineSession {
    match engine_id {
        "claude-code" => EngineSession {
            mode: SessionMode::Assign,
            fresh: Some(|id| owned(&["--session-id", id])),
            resume_by_id: Some(|id| owned(&["--resume", id])),
            resume_latest: &["--continue"],
            bypass: &["--dangerously-skip-permissions"],
            model: Some("--model"),
            permission_mode: Some("--permission-mode"),
            effort: Some("--effort"),
        },
        "gemini" => EngineSession {
            mode: SessionMode::Assign,
            fresh: Some(|id| owned(&["--session-id", id])),
            resume_by_id: Some(|id| owned(&["--resume", id])),
            resume_latest: &["--resume", "latest"],
            bypass: &["--yolo"],
            model: None,
            permission_mode: None,
            effort: None,
        },
        "codex" => EngineSession {
            mode: SessionMode::ExecMint,
            fresh: Some(|id| owned(&["resume", id])),
            resume_by_id: Some(|id| owned(&["resume", id])),
            resume_latest: &["resume", "--last"],
            bypass: &["--dangerously-bypass-approvals-and-sandbox"],
            model: Some("--model"),
            permission_mode: None,
            effort: None,
        },
        "antigravity" => EngineSession {
            mode: SessionMode::Capture,
            fresh: None,
            resume_by_id: Some(|id| owned(&["--conversation", id])),
            resume_latest: &["--continue"],
            bypass: &["--dangerously-skip-permissions"],
            model: None,
            permission_mode: None,
            effort: None,
        },
        _ => EngineSession {
            mode: SessionMode::None,
            fresh: None,
            resume_by_id: None,
            resume_latest: &[],
            bypass: &[],
            model: None,
            permission_mode: None,
            effort: None,
        },
    }
}

/// The configured engine, else the builtin, else one whose command is its id.
pub fn resolve(configured: &[Engine], engine_id: &str) -> Engine {
    configured
        .iter()
        .find(|e| e.id == engine_id)
        .cloned()
        .or_else(|| Engine::builtins().into_iter().find(|e| e.id == engine_id))
        .unwrap_or_else(|| Engine {
            id: engine_id.to_string(),
            command: engine_id.to_string(),
            ..Engine::default()
        })
}

/// Configured or builtin: an id `resolve` would run as a bare command is neither.
pub fn is_known(configured: &[Engine], engine_id: &str) -> bool {
    configured.iter().any(|e| e.id == engine_id)
        || Engine::builtins().iter().any(|e| e.id == engine_id)
}

/// The composer harness an engine is the TUI of; a spawned child defaults to it.
pub fn harness_of(engine_id: &str) -> Option<Harness> {
    match engine_id {
        "claude-code" => Some(Harness::Claude),
        "codex" => Some(Harness::Codex),
        _ => None,
    }
}

/// Whether a fresh launch without a session id needs a `codex exec` mint first.
pub fn mints_via_exec(engine_id: &str) -> bool {
    engine_session(engine_id).mode == SessionMode::ExecMint
}

/// `[...engine.args, ...session args, ...bypass]` and the provider session id it names.
/// A hooked codex starts fresh with no `resume`: its id arrives with `SessionStart`.
pub fn session_argv(
    engine: &Engine,
    s: &PtyLaunchSession,
    hooked_codex: bool,
) -> (Vec<String>, Option<String>) {
    let spec = engine_session(&engine.id);
    let (session_args, session_id) = if s.resume {
        match (&s.session_id, spec.resume_by_id) {
            (Some(id), Some(by_id)) => (by_id(id), Some(id.clone())),
            _ => (owned(spec.resume_latest), None),
        }
    } else if hooked_codex {
        (Vec::new(), None)
    } else {
        let id = s.session_id.clone().or_else(|| {
            (spec.mode == SessionMode::Assign).then(|| uuid::Uuid::new_v4().to_string())
        });
        match (id, spec.fresh) {
            (Some(id), Some(fresh)) => (fresh(&id), Some(id)),
            _ => (Vec::new(), None),
        }
    };
    let mut argv = engine.args.clone();
    argv.extend(session_args);
    if s.bypass {
        argv.extend(owned(spec.bypass));
    }
    if let (Some(flag), Some(model)) = (spec.model, &s.model) {
        argv.extend([flag.to_string(), model.clone()]);
    }
    // Bypass is the bypass flag's alone: a mode key must never turn it on.
    let mode = s
        .permission_mode
        .as_ref()
        .filter(|m| *m != "bypassPermissions");
    if let (Some(flag), Some(mode)) = (spec.permission_mode, mode) {
        argv.extend([flag.to_string(), mode.clone()]);
    }
    if let (Some(flag), Some(effort)) = (spec.effort, &s.effort) {
        argv.extend([flag.to_string(), effort.clone()]);
    }
    (argv, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv_for(engine_id: &str, model: Option<&str>) -> Vec<String> {
        let session = PtyLaunchSession {
            model: model.map(str::to_string),
            ..PtyLaunchSession::default()
        };
        session_argv(&resolve(&[], engine_id), &session, false).0
    }

    #[test]
    fn claude_and_codex_launch_the_model_asked_for() {
        for engine_id in ["claude-code", "codex"] {
            let argv = argv_for(engine_id, Some("haiku"));
            let at = argv.iter().position(|a| a == "--model");
            assert_eq!(
                at.and_then(|i| argv.get(i + 1)).map(String::as_str),
                Some("haiku"),
                "{engine_id}: {argv:?}"
            );
            assert!(!argv_for(engine_id, None).contains(&"--model".to_string()));
        }
    }
}
