pub mod commands;
mod editors;
pub mod error;
mod events;
mod runtime_events;
mod state;
mod webview_gestures;
mod window_geometry;

use std::path::PathBuf;

use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::runtime::{HyprlayerRuntime, RuntimeOptions};
#[cfg(target_os = "macos")]
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use tauri_specta::{collect_commands, collect_events, Builder, Event};

use crate::state::AppState;

fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::projects::get_bootstrap,
            commands::projects::get_project,
            commands::projects::inspect_repo_path,
            commands::projects::import_project,
            commands::projects::import_local_folder,
            commands::projects::clone_project,
            commands::projects::create_project,
            commands::projects::list_github_owners,
            commands::projects::publish_project_to_github,
            commands::projects::remove_project,
            commands::projects::reorder_projects,
            commands::projects::list_recent_repos,
            commands::projects::list_thoughts_profiles,
            commands::projects::create_thoughts_profile,
            commands::projects::sync_thoughts,
            commands::projects::run_backend_health,
            commands::projects::get_worktree_files,
            commands::projects::set_worktree_files,
            commands::projects::check_worktree_files,
            commands::library::query_docs,
            commands::library::get_doc,
            commands::library::save_doc,
            commands::library::patch_doc_frontmatter,
            commands::library::create_doc,
            commands::library::reveal_doc,
            commands::library::doc_web_url,
            commands::library::get_thoughts_git_status,
            commands::workstreams::list_workstreams,
            commands::workstreams::list_all_workstreams,
            commands::workstreams::list_workstream_bases,
            commands::workstreams::fetch_project_remote,
            commands::workstreams::discard_project_changes,
            commands::workstreams::create_workstream,
            commands::workstreams::get_branch_prefixes,
            commands::workstreams::set_branch_prefixes,
            commands::workstreams::rename_workstream,
            commands::workstreams::set_workstream_base_branch,
            commands::workstreams::update_workstream_from_base,
            commands::workstreams::fetch_workstream_remote,
            commands::workstreams::set_workstream_stage,
            commands::workstreams::attach_workstream_doc,
            commands::workstreams::detach_workstream_doc,
            commands::workstreams::list_workstream_docs,
            commands::workstreams::archive_workstream,
            commands::workstreams::list_archived_workstreams,
            commands::workstreams::restore_workstream,
            commands::workstreams::delete_archived_workstream,
            commands::workstreams::cleanup_archived_workstreams,
            commands::workstreams::reprovision_workstream,
            commands::workstreams::worktree_git_status,
            commands::workstreams::workstream_prompt_context,
            commands::workstreams::worktree_stage,
            commands::workstreams::worktree_unstage,
            commands::workstreams::worktree_discard,
            commands::workstreams::worktree_file_sides,
            commands::workstreams::worktree_list_dir,
            commands::workstreams::worktree_read_file,
            commands::workstreams::worktree_write_file,
            commands::workstreams::list_slash_commands,
            commands::workstreams::worktree_search_files,
            commands::workstreams::list_workstream_commits,
            commands::workstreams::worktree_commit_files,
            commands::workstreams::worktree_commit_file_sides,
            commands::workstreams::watch_worktree,
            commands::workstreams::workstream_pr_status,
            commands::workstreams::list_workstream_open_prs,
            commands::workstreams::preview_workstream_pr,
            commands::workstreams::attach_workstream_pr,
            commands::workstreams::detach_workstream_pr,
            commands::workstreams::merge_workstream_pr,
            commands::sessions::start_stage_session,
            commands::sessions::resume_session,
            commands::sessions::restart_session,
            commands::sessions::rewind_session,
            commands::sessions::send_user_message,
            commands::sessions::respond_permission,
            commands::sessions::answer_question,
            commands::sessions::interrupt_session,
            commands::sessions::kill_session,
            commands::sessions::set_session_model,
            commands::sessions::set_session_effort,
            commands::sessions::list_sessions,
            commands::sessions::get_session_log,
            commands::sessions::attach_session,
            commands::sessions::list_active_sessions,
            commands::attachments::import_attachment,
            commands::attachments::save_attachment,
            commands::system::set_binary_override,
            commands::system::redetect_binaries,
            commands::system::detect_cli_install_methods,
            commands::system::hyprlayer_cli_update_check,
            commands::system::update_hyprlayer_cli,
            commands::system::install_hyprlayer_cli,
            commands::system::get_app_settings,
            commands::system::set_app_settings,
            commands::system::detect_editors,
            commands::system::github_username,
            commands::system::default_username,
            commands::system::play_sound,
            commands::system::full_disk_access_status,
            commands::system::open_full_disk_access_settings,
            commands::system::get_omarchy_theme,
            commands::system::quit_app,
            commands::harnesses::opencode_catalog,
            commands::harnesses::opencode_auth_list,
            commands::harnesses::opencode_set_provider_key,
            commands::harnesses::opencode_remove_provider,
            commands::harnesses::opencode_set_default_model,
            commands::harnesses::install_opencode,
            commands::system::claude_sync_models,
            commands::harnesses::claude_auth_status,
            commands::harnesses::claude_probe,
            commands::harnesses::codex_probe,
            commands::harnesses::codex_login_with_api_key,
            commands::harnesses::codex_logout,
            commands::models::claude_model_windows,
            commands::ollama::ollama_status,
            commands::ollama::ollama_set_endpoint,
            commands::ollama::ollama_set_selected_models,
            commands::ollama::ollama_set_default_model,
            commands::titles::suggest_tab_title,
            commands::terminal::terminal_open,
            commands::terminal::agent_open,
            commands::terminal::agent_resize,
            commands::terminal::capture_agent_session,
            commands::terminal::mint_codex_session,
            commands::terminal::agent_session_exists,
            commands::terminal::terminal_attach,
            commands::terminal::terminal_write,
            commands::terminal::terminal_resize,
            commands::terminal::terminal_close,
            commands::terminal::spawn_cli_terminal,
            commands::terminal::spawn_shell_terminal,
            commands::terminal::default_shell_name,
            commands::scripts::get_scripts,
            commands::scripts::set_scripts,
            commands::scripts::script_run,
            commands::scripts::script_stop,
            commands::scripts::get_script_runtime,
            commands::scripts::list_running_scripts,
            commands::scripts::get_script_output,
            commands::scripts::script_clear_output,
            commands::onboarding::get_ai_status,
            commands::onboarding::probe_default_thoughts,
            commands::onboarding::init_thoughts_repo,
            commands::prompts::preview_prompt_import,
            commands::prompts::apply_prompt_import,
            commands::prompts::export_prompts,
            commands::agents::list_agent_files,
            commands::agents::save_agent_file,
            commands::agents::delete_agent_file,
            commands::agents::new_agent_file_path,
            commands::agents::agent_context_files,
            commands::agents::agent_mcp_servers,
            commands::agents::agent_skills,
            commands::agents::export_agent_file,
            commands::agents::agent_bundle_status,
            commands::agents::update_agent_bundle,
            commands::agents::agent_bundle_versions,
            commands::agents::install_agent_bundle_version,
            commands::agents::repair_agent_bundle,
        ])
        .events(collect_events![
            events::ConfigChanged,
            events::ProjectCreateProgress,
            events::OmarchyThemeChanged,
            events::BinariesChanged,
            events::ThoughtsIndexUpdated,
            events::PrStatusChanged,
            events::WorktreeChanged,
            events::WorkstreamBranchChanged,
            events::SessionLifecycle,
            events::ScriptRunChanged,
            events::QuitRequested
        ])
}

fn hyprlayer_config_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hyprlayer")
        .join("config.json")
}

pub(crate) fn opencode_auth_file() -> PathBuf {
    if let Some(p) = std::env::var_os("OPENCODE_AUTH_JSON") {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")))
        .map(|base| base.join("opencode").join("auth.json"))
        .unwrap_or_else(|| PathBuf::from(".").join("opencode-auth.json"))
}

pub(crate) fn opencode_config_file() -> PathBuf {
    if let Some(p) = std::env::var_os("OPENCODE_CONFIG") {
        return PathBuf::from(p);
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .map(|base| base.join("opencode").join("opencode.json"))
        .unwrap_or_else(|| PathBuf::from(".").join("opencode.json"))
}

fn claude_config_dir(home: &std::path::Path) -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

fn claude_managed_settings_files() -> Vec<PathBuf> {
    vec![PathBuf::from(if cfg!(target_os = "windows") {
        r"C:\Program Files\ClaudeCode\managed-settings.json"
    } else if cfg!(target_os = "macos") {
        "/Library/Application Support/ClaudeCode/managed-settings.json"
    } else {
        "/etc/claude-code/managed-settings.json"
    })]
}

fn resolve_paths(app: &tauri::App) -> tauri::Result<RuntimePaths> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let app_config_dir = app.path().app_config_dir()?;
    let app_data_dir = app.path().app_data_dir()?;
    Ok(RuntimePaths {
        hyprlayer_config_file: hyprlayer_config_file(),
        app_config_dir,
        app_data_dir,
        default_worktree_root: home.join("hyprlayer").join("worktrees"),
        default_repos_root: home.join("hyprlayer").join("repos"),
        opencode_config_file: opencode_config_file(),
        opencode_auth_file: opencode_auth_file(),
        opencode_data_dir: dirs::data_dir()
            .unwrap_or_else(|| home.join(".local").join("share"))
            .join("opencode"),
        claude_config_dir: claude_config_dir(&home),
        claude_managed_settings_files: claude_managed_settings_files(),
        codex_home: std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex")),
        agent_roots: vec![home.join(".claude").join("agents")],
        download_dir: dirs::download_dir().unwrap_or_else(|| home.clone()),
    })
}

fn resolve_environment() -> RuntimeEnvironment {
    RuntimeEnvironment {
        home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
        process_cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        process_env: std::env::vars_os().collect(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    #[cfg(debug_assertions)]
    if let Err(e) = builder.export(
        specta_typescript::Typescript::default(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../src/lib/bindings.ts"),
    ) {
        eprintln!("skipping typescript bindings export: {e}");
    }

    let tauri_builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .skip_initial_state("main")
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init());

    #[cfg(debug_assertions)]
    let tauri_builder = tauri_builder.plugin(tauri_plugin_mcp_bridge::init());

    #[cfg(target_os = "macos")]
    let tauri_builder = tauri_builder.menu(macos_menu);

    tauri_builder
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

            window_geometry::restore_main_window(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                webview_gestures::disable_pinch_zoom(&window);
            }

            let paths = resolve_paths(app)?;
            let environment = resolve_environment();
            let options = RuntimeOptions::new(environment, paths);
            #[cfg(target_os = "macos")]
            let options = options.with_credential_lookup(std::sync::Arc::new(
                commands::models::KeychainCredentialLookup,
            ));

            let runtime = match tauri::async_runtime::block_on(HyprlayerRuntime::open(options)) {
                Ok(runtime) => runtime,
                Err(hyprlayer_core::CoreError::RuntimeInUse { path }) => {
                    app.dialog()
                        .message(format!(
                            "Another Hyprlayer window or process is already using the profile at {}.",
                            path.display()
                        ))
                        .title("Hyprlayer is already running")
                        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                        .blocking_show();
                    std::process::exit(1);
                }
                Err(e @ hyprlayer_core::CoreError::RuntimeMigrationLedgerInvalid { .. })
                | Err(e @ hyprlayer_core::CoreError::RuntimeMigrationLedgerTooNew { .. }) => {
                    let detail = match e.remedy() {
                        Some(remedy) => format!("{e}\n\n{remedy}"),
                        None => e.to_string(),
                    };
                    app.dialog()
                        .message(detail)
                        .title("Hyprlayer cannot start")
                        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                        .blocking_show();
                    std::process::exit(1);
                }
                Err(e) => return Err(e.into()),
            };

            if let Err(e) = app
                .asset_protocol_scope()
                .allow_directory(runtime.worktree_root(), true)
            {
                log::warn!("could not register worktree root with the asset scope: {e}");
            }

            runtime_events::spawn(app.handle().clone(), runtime.clone());

            if let Some(window) = app.get_webview_window("main") {
                let runtime = runtime.clone();
                let handle = app.handle().clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::Focused(is_focused) => {
                        runtime.set_foreground(*is_focused);
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        request_quit(&handle);
                    }
                    _ => {}
                });
            }

            app.manage(AppState::new(runtime.clone()));

            tauri::async_runtime::spawn(async move {
                if let Err(e) = runtime.start().await {
                    log::warn!("runtime failed to start: {e}");
                }
            });

            if let Some(omarchy_dir) = hyprlayer_core::omarchy::current_dir(
                dirs::state_dir().as_deref(),
                dirs::config_dir().as_deref(),
            ) {
                log::info!("watching Omarchy theme state at {}", omarchy_dir.display());
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let key = |dir: &std::path::Path| {
                        let stat = |p: std::path::PathBuf| {
                            std::fs::metadata(p)
                                .ok()
                                .map(|m| (m.modified().ok(), m.len()))
                        };
                        (
                            stat(dir.join("theme.name")),
                            stat(dir.join("theme").join("colors.toml")),
                        )
                    };
                    let mut last = key(&omarchy_dir);
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let current = key(&omarchy_dir);
                        if current != last {
                            last = current;
                            let _ = events::OmarchyThemeChanged.emit(&handle);
                        }
                    }
                });
            }

            #[cfg(target_os = "macos")]
            app.on_menu_event(|app, event| match event.id().as_ref() {
                "check-updates" => {
                    let _ = app.emit("menu:check-for-updates", ());
                }
                "open-settings" => {
                    let _ = app.emit("menu:open-settings", ());
                }
                "save-file" => {
                    let _ = app.emit("menu:save-file", ());
                }
                "quit" => request_quit(app),
                _ => {}
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                use tauri::Manager;
                let state = app.state::<AppState>();
                state.runtime.shutdown_now();
            }
        });
}

fn request_quit(handle: &tauri::AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let confirm = match handle.try_state::<AppState>() {
            Some(state) => state.runtime.settings().get().await.quit.confirm,
            None => false,
        };
        let Some(window) = handle.get_webview_window("main").filter(|_| confirm) else {
            handle.exit(0);
            return;
        };
        #[cfg(target_os = "macos")]
        let _ = handle.show();
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = events::QuitRequested.emit(&handle);
    });
}

#[cfg(target_os = "macos")]
fn macos_menu(handle: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{AboutMetadata, MenuBuilder, SubmenuBuilder};

    let pkg = handle.package_info();
    let config = handle.config();
    let about = AboutMetadata {
        name: Some(pkg.name.clone()),
        version: Some(pkg.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    let check_updates = tauri::menu::MenuItem::with_id(
        handle,
        "check-updates",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;
    let open_settings = tauri::menu::MenuItem::with_id(
        handle,
        "open-settings",
        "Settings",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let save_file =
        tauri::menu::MenuItem::with_id(handle, "save-file", "Save", true, Some("CmdOrCtrl+S"))?;
    let quit = tauri::menu::MenuItem::with_id(
        handle,
        "quit",
        format!("Quit {}", pkg.name),
        true,
        Some("CmdOrCtrl+Q"),
    )?;

    let app_menu = SubmenuBuilder::new(handle, &pkg.name)
        .about(Some(about))
        .separator()
        .item(&check_updates)
        .item(&open_settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .separator()
        .item(&quit)
        .build()?;

    let file_menu = SubmenuBuilder::new(handle, "File")
        .item(&save_file)
        .build()?;

    let edit_menu = SubmenuBuilder::new(handle, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_menu = SubmenuBuilder::new(handle, "View").fullscreen().build()?;

    let window_menu = SubmenuBuilder::new(handle, "Window")
        .minimize()
        .maximize()
        .build()?;

    let help_menu = SubmenuBuilder::new(handle, "Help").build()?;

    MenuBuilder::new(handle)
        .items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ])
        .build()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn typescript_bindings_export() {
        std::fs::create_dir_all("../src/lib").expect("frontend src/lib exists");
        super::specta_builder()
            .export(
                specta_typescript::Typescript::default(),
                "../src/lib/bindings.ts",
            )
            .expect("bindings must export cleanly");
    }

    #[test]
    fn selected_ipc_commands_stay_registered() {
        let out = tempfile::NamedTempFile::new().expect("tempfile");
        super::specta_builder()
            .export(specta_typescript::Typescript::default(), out.path())
            .expect("bindings must export cleanly");
        let bindings = std::fs::read_to_string(out.path()).unwrap();
        for command in [
            "getSessionLog:",
            "answerQuestion:",
            "setBinaryOverride:",
            "getAppSettings:",
        ] {
            assert!(
                bindings.contains(command),
                "{command} is missing from the generated bindings"
            );
        }
    }

    fn merge(base: &mut Value, overlay: &Value) {
        match (base, overlay) {
            (Value::Object(base), Value::Object(overlay)) => {
                for (key, value) in overlay {
                    merge(base.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
            (base, overlay) => *base = overlay.clone(),
        }
    }

    #[test]
    fn platform_configs_merge_into_a_valid_config() {
        let base: Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json is valid JSON");
        for (platform, overlay) in [
            ("linux", include_str!("../tauri.linux.conf.json")),
            ("windows", include_str!("../tauri.windows.conf.json")),
        ] {
            let overlay: Value =
                serde_json::from_str(overlay).unwrap_or_else(|e| panic!("{platform}: {e}"));
            let mut merged = base.clone();
            merge(&mut merged, &overlay);
            serde_json::from_value::<tauri::utils::config::Config>(merged)
                .unwrap_or_else(|e| panic!("{platform} config does not deserialize: {e}"));
        }
    }

    #[test]
    fn linux_bundles_deb_and_rpm_only() {
        let overlay: Value =
            serde_json::from_str(include_str!("../tauri.linux.conf.json")).unwrap();
        let targets = overlay["bundle"]["targets"]
            .as_array()
            .expect("linux config pins bundle.targets");
        assert_eq!(targets, &[Value::from("deb"), Value::from("rpm")]);
    }

    #[test]
    fn linux_min_size_stays_below_the_tiling_squeeze_threshold() {
        let overlay: Value =
            serde_json::from_str(include_str!("../tauri.linux.conf.json")).unwrap();
        let windows = overlay["app"]["windows"]
            .as_array()
            .expect("linux config declares app.windows");
        let main = windows
            .iter()
            .find(|w| w["label"] == "main")
            .expect("linux config declares the main window");
        assert!(main["minWidth"].as_f64().unwrap() <= 500.0);
        assert!(main["minHeight"].as_f64().unwrap() <= 400.0);
    }
}
