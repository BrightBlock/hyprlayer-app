use std::time::Duration;

use tauri::{Manager, State};
use tokio::sync::broadcast;

use hyprlayer_core::agent::AgentDriver;
use hyprlayer_core::claude::{PermissionMode, SessionEvent};
use hyprlayer_core::codex::{CodexDriver, CodexPosture, SpawnConfig as CodexSpawnConfig};
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::process::Exec;

use crate::error::{AppError, CmdResult};
use crate::opencode::OpenCodeDriver;
use crate::state::AppState;

const MAX_INPUT_CHARS: usize = 1500;
const MAX_TITLE_CHARS: usize = 48;
const TITLE_TIMEOUT: Duration = Duration::from_secs(20);

#[tauri::command]
#[specta::specta]
pub async fn suggest_tab_title(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> CmdResult<String> {
    let settings = state.settings.get().await;
    if !settings.tab_titles.enabled {
        return Err(AppError {
            code: "tab_titles_disabled".into(),
            message: "auto tab titles are turned off".into(),
            remedy: None,
        });
    }
    let model_tag = settings
        .tab_titles
        .effective_model(&settings.providers)
        .to_string();
    let prompt = build_prompt(&text);

    let raw = match parse_model_tag(&model_tag) {
        TitleModel::Claude(flag) => generate_with_claude(&state, flag, &prompt).await?,
        TitleModel::OpenCode(provider_model) => {
            generate_with_opencode(&app, &state, provider_model, &prompt).await?
        }
        TitleModel::Codex(model) => generate_with_codex(&state, model, &prompt).await?,
    };

    let title = sanitize_title(&raw);
    if title.is_empty() {
        return Err(AppError {
            code: "empty_title".into(),
            message: "model produced no usable title".into(),
            remedy: None,
        });
    }
    Ok(title)
}

fn build_prompt(text: &str) -> String {
    let snippet: String = text.chars().take(MAX_INPUT_CHARS).collect();
    format!(
        "Generate a concise tab title (2-5 words, Title Case, no punctuation, no \
         quotes) summarizing this coding task. Reply with ONLY the title.\n\nTask:\n{snippet}"
    )
}

enum TitleModel<'a> {
    Claude(&'a str),
    OpenCode(&'a str),
    Codex(&'a str),
}

fn parse_model_tag(tag: &str) -> TitleModel<'_> {
    if let Some(rest) = tag.strip_prefix("opencode:") {
        TitleModel::OpenCode(rest)
    } else if let Some(rest) = tag.strip_prefix("codex:") {
        TitleModel::Codex(rest)
    } else {
        TitleModel::Claude(tag.strip_prefix("claude:").unwrap_or(tag))
    }
}

async fn generate_with_claude(state: &AppState, flag: &str, prompt: &str) -> CmdResult<String> {
    let bin = state
        .binaries
        .require(BinaryKind::Claude)
        .await
        .map_err(AppError::from)?;
    let path = state.binaries.augmented_path().await;
    let claude_cfg = state.settings.get().await.providers.claude;

    let mut exec = Exec::new(&bin).arg("-p").stdin(prompt.as_bytes().to_vec());
    if !flag.is_empty() {
        exec = exec.arg("--model").arg(flag);
    }
    exec = exec
        .arg("--output-format")
        .arg("json")
        .env("PATH", path)
        .timeout(TITLE_TIMEOUT);
    for (k, v) in super::claude_auth_env(&claude_cfg) {
        exec = exec.env(k, v);
    }

    let out = exec.run().await.map_err(AppError::from)?;
    Ok(serde_json::from_str::<serde_json::Value>(out.stdout.trim())
        .ok()
        .and_then(|v| v.get("result").and_then(|r| r.as_str()).map(str::to_string))
        .unwrap_or(out.stdout))
}

async fn generate_with_opencode(
    app: &tauri::AppHandle,
    state: &AppState,
    provider_model: &str,
    prompt: &str,
) -> CmdResult<String> {
    let bin = state
        .binaries
        .require(BinaryKind::OpenCode)
        .await
        .map_err(AppError::from)?;
    let path = state.binaries.augmented_path().await;
    let data_dir = app
        .path()
        .app_data_dir()
        .map(|d| d.join("opencode"))
        .map_err(|e| AppError::internal(format!("resolve app data dir: {e}")))?;
    let server = state
        .opencode
        .ensure(bin, path, data_dir)
        .await
        .map_err(AppError::internal)?;

    let cwd = std::env::temp_dir().to_string_lossy().into_owned();
    let (driver, mut rx) = OpenCodeDriver::spawn(server, &cwd, Some(provider_model.to_string()))
        .await
        .map_err(AppError::internal)?;

    driver
        .send_user_turn(prompt, None, None)
        .map_err(AppError::from)?;
    let collected = drain_title(&mut rx).await;
    driver.kill();
    driver.delete_session().await;

    collected.ok_or_else(|| AppError {
        code: "opencode_title_failed".into(),
        message: "OpenCode produced no title".into(),
        remedy: None,
    })
}

async fn generate_with_codex(state: &AppState, model: &str, prompt: &str) -> CmdResult<String> {
    let bin = state
        .binaries
        .require(BinaryKind::Codex)
        .await
        .map_err(AppError::from)?;
    let path = state.binaries.augmented_path().await;

    let (driver, mut rx) = CodexDriver::spawn(CodexSpawnConfig {
        codex_bin: bin,
        cwd: std::env::temp_dir(),
        prior_user_turns: 0,
        posture: CodexPosture::from_permission_mode(PermissionMode::Plan),
        add_dirs: Vec::new(),
        skill_roots: Vec::new(),
        resume: None,
        model: (!model.is_empty()).then(|| model.to_string()),
        effort: None,
        ephemeral: true,
        env: vec![("PATH".to_string(), path.to_string_lossy().into_owned())],
    })
    .await
    .map_err(AppError::from)?;

    driver
        .send_user_turn(prompt, None, None)
        .map_err(AppError::from)?;
    let collected = drain_title(&mut rx).await;
    driver.kill();

    collected.ok_or_else(|| AppError {
        code: "codex_title_failed".into(),
        message: "Codex produced no title".into(),
        remedy: None,
    })
}

async fn drain_title(rx: &mut broadcast::Receiver<SessionEvent>) -> Option<String> {
    let deadline = tokio::time::sleep(TITLE_TIMEOUT);
    tokio::pin!(deadline);
    let mut streamed = String::new();
    let mut finalized = String::new();
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            ev = rx.recv() => match ev {
                Ok(SessionEvent::PartialText { text: t, .. }) => streamed.push_str(&t),
                Ok(SessionEvent::AssistantText { text: t, .. }) => {
                    if t.starts_with('⚠') {
                        return None;
                    }
                    finalized.push_str(&t);
                }
                Ok(SessionEvent::TurnDone { .. })
                    if !finalized.trim().is_empty() || !streamed.trim().is_empty() =>
                {
                    break
                }
                Ok(SessionEvent::Exited { .. }) => break,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    let raw = if !finalized.trim().is_empty() {
        finalized
    } else {
        streamed
    };
    let title = raw.trim().to_string();
    (!title.is_empty()).then_some(title)
}

fn sanitize_title(raw: &str) -> String {
    let first_line = raw.lines().next().unwrap_or("").trim();
    let unquoted = first_line
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim();
    let collapsed = unquoted.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_TITLE_CHARS {
        collapsed
            .chars()
            .take(MAX_TITLE_CHARS)
            .collect::<String>()
            .trim_end()
            .to_string()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::{drain_title, parse_model_tag, sanitize_title, TitleModel};
    use hyprlayer_core::claude::SessionEvent;
    use tokio::sync::broadcast;

    #[test]
    fn strips_quotes_and_collapses_whitespace() {
        assert_eq!(
            sanitize_title("\"OAuth Token Refresh\""),
            "OAuth Token Refresh"
        );
        assert_eq!(sanitize_title("`Fix   Flaky Tests`"), "Fix Flaky Tests");
    }

    #[test]
    fn takes_first_line_only() {
        assert_eq!(
            sanitize_title("Add Dark Mode\nHere is why: ..."),
            "Add Dark Mode"
        );
    }

    #[test]
    fn caps_length() {
        let long = "Word ".repeat(40);
        assert!(sanitize_title(&long).chars().count() <= 48);
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(sanitize_title("   \n  "), "");
    }

    #[test]
    fn routes_model_tags() {
        assert!(matches!(
            parse_model_tag("claude:haiku"),
            TitleModel::Claude("haiku")
        ));
        assert!(matches!(parse_model_tag("claude:"), TitleModel::Claude("")));
        assert!(matches!(
            parse_model_tag("opencode:anthropic/claude-haiku-4-5"),
            TitleModel::OpenCode("anthropic/claude-haiku-4-5")
        ));
        assert!(matches!(
            parse_model_tag("codex:gpt-5.4-codex"),
            TitleModel::Codex("gpt-5.4-codex")
        ));
        assert!(matches!(
            parse_model_tag("sonnet"),
            TitleModel::Claude("sonnet")
        ));
    }

    fn partial(text: &str) -> SessionEvent {
        SessionEvent::PartialText {
            text: text.into(),
            parent_id: None,
        }
    }
    fn assistant(text: &str) -> SessionEvent {
        SessionEvent::AssistantText {
            text: text.into(),
            parent_id: None,
            injected: false,
        }
    }
    fn turn_done() -> SessionEvent {
        SessionEvent::TurnDone {
            cost_usd: None,
            num_turns: None,
            context_tokens: None,
            turn_status: None,
        }
    }

    #[tokio::test]
    async fn opencode_title_falls_back_to_streamed_text() {
        let (tx, mut rx) = broadcast::channel(8);
        tx.send(partial("OAuth ")).unwrap();
        tx.send(partial("Token Refresh")).unwrap();
        tx.send(turn_done()).unwrap();
        assert_eq!(
            drain_title(&mut rx).await.as_deref(),
            Some("OAuth Token Refresh"),
        );
    }

    #[tokio::test]
    async fn opencode_title_prefers_finalized_block() {
        let (tx, mut rx) = broadcast::channel(8);
        tx.send(partial("OAuth")).unwrap();
        tx.send(assistant("Refresh Token Flow")).unwrap();
        tx.send(turn_done()).unwrap();
        assert_eq!(
            drain_title(&mut rx).await.as_deref(),
            Some("Refresh Token Flow"),
        );
    }

    #[tokio::test]
    async fn title_rejects_error_sentinels() {
        for sentinel in [
            "⚠ OpenCode error: unknown model",
            "⚠ OpenCode request failed: connection refused",
            "⚠ Codex error: rate limited",
            "⚠ Codex request failed: turn/start: not signed in",
        ] {
            let (tx, mut rx) = broadcast::channel(8);
            tx.send(assistant(sentinel)).unwrap();
            assert_eq!(
                drain_title(&mut rx).await,
                None,
                "sentinel must not become a title: {sentinel:?}",
            );
        }
    }
}
