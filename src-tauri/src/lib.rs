mod client;
pub mod commands;
mod editors;
pub mod error;
mod events;
mod hosts;
mod logging;
mod provision;
mod runtime_events;
mod server_version;
mod state;
mod supervisor;
mod webview_gestures;
mod window_geometry;

use std::collections::BTreeMap;
use std::sync::Arc;

#[cfg(target_os = "macos")]
use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;
use tauri_specta::{collect_commands, collect_events, Builder, Event};

use crate::error::AppError;
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
            commands::projects::browse_directory,
            commands::library::query_docs,
            commands::library::get_doc,
            commands::library::save_doc,
            commands::library::patch_doc_frontmatter,
            commands::library::save_doc_parts,
            commands::library::resolve_related_docs,
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
            commands::workstreams::list_project_memory,
            commands::workstreams::save_project_memory_file,
            commands::workstreams::archive_workstream,
            commands::workstreams::list_archived_workstreams,
            commands::workstreams::restore_workstream,
            commands::workstreams::delete_archived_workstream,
            commands::workstreams::refresh_archived_workstreams,
            commands::workstreams::keep_archived_workstream,
            commands::workstreams::purge_archived_workstreams,
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
            commands::attachments::read_attachment,
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
            commands::prompts::write_prompt_export,
            commands::prompts::read_prompt_import,
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
            commands::hosts::list_hosts,
            commands::hosts::save_hosts,
            commands::hosts::test_host_connection,
            commands::hosts::connection_status,
            commands::hosts::reconnect_server,
            commands::hosts::restart_server,
            commands::hosts::inspect_host_provisioning,
            commands::hosts::provision_host,
            commands::hosts::update_host_server,
            commands::hosts::list_local_servers,
            commands::diagnostics::reveal_logs,
            commands::diagnostics::collect_diagnostics,
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
            events::QuitRequested,
            events::ServerDisconnected
        ])
}

/// The vars a daemonized server has none of on its own (ADR-0009 §7):
/// `SSH_AUTH_SOCK`, `DISPLAY`, `TERM`, `LANG`, and any `LC_*`.
fn env_overlay_from_desktop() -> BTreeMap<String, String> {
    let mut overlay = BTreeMap::new();
    for key in ["SSH_AUTH_SOCK", "DISPLAY", "TERM", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            overlay.insert(key.to_string(), value);
        }
    }
    for (key, value) in std::env::vars() {
        if key.starts_with("LC_") {
            overlay.insert(key, value);
        }
    }
    overlay
}

/// Everything a live client needs wired up, on launch and again after a
/// reconnect (plan Phase 8 §2). Asset-scope registration deliberately does not
/// repeat: it derives from `client.worktree_root()`, and a host that has not
/// changed has an unchanged root.
pub(crate) async fn attach_client(handle: &tauri::AppHandle, client: Arc<client::RpcClient>) {
    // Re-injected on every connect (ADR-0009 §7): a daemonized server has none
    // of these on its own — a respawned local server least of all — and without
    // SSH_AUTH_SOCK every `git push`/`gh` call would fail as ADR-0005 predicts.
    let overlay = env_overlay_from_desktop();
    log::info!(
        "sending the server {} ({})",
        overlay.keys().cloned().collect::<Vec<_>>().join(", "),
        if overlay.contains_key("SSH_AUTH_SOCK") {
            "with an SSH agent"
        } else {
            "no SSH agent: git and gh cannot authenticate over SSH"
        }
    );
    if let Err(e) = client.set_env_overlay(overlay).await {
        log::warn!(
            "could not send the environment overlay to the server: {}",
            e.message
        );
    }
    runtime_events::spawn(handle.clone(), client);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::install_panic_hook();
    let builder = specta_builder();
    // Bound before the builder so the identifier is readable: the profile id
    // has to be injected into the webview before any script runs.
    let context = tauri::generate_context!();
    let profile = dirs::config_dir()
        .map(|d| d.join(&context.config().identifier))
        .map(|dir| hosts::profile_id(&dir))
        .unwrap_or_else(|| hosts::LOCAL_PROFILE.to_string());

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
        .plugin(logging::plugin())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        // zustand's `persist` fixes its key at store creation, during the
        // import graph — so the namespace has to exist before any script runs,
        // which only an init script guarantees.
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("hyprlayer-profile")
                .js_init_script(format!(
                    "window.__HYPRLAYER_PROFILE__ = {};",
                    serde_json::to_string(&profile).expect("a string serializes")
                ))
                .build(),
        );

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

            let app_config_dir = app.path().app_config_dir()?;
            let app_data_dir = app.path().app_data_dir()?;
            let app_log_dir = app.path().app_log_dir()?;

            // hosts.json is read before connecting anything, because it's the
            // one thing that has to be decided locally: there is no server to
            // ask which server to reach (ADR-0009, plan Phase 5 §2).
            let target = hosts::target(&app_config_dir);
            let package = app.package_info();
            log::info!(
                "{} {} starting: pid {}, {}, {} build, {}/{}, protocol {}, server: {}, logs {}",
                package.name,
                package.version,
                std::process::id(),
                app.config().identifier,
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
                std::env::consts::OS,
                std::env::consts::ARCH,
                hyprlayer_proto::PROTOCOL_VERSION,
                target.name(),
                app_log_dir.display(),
            );
            let outcome = tauri::async_runtime::block_on(supervisor::connect(
                &target,
                &app_config_dir,
                &app_data_dir,
                &app_log_dir,
            ));
            // VS Code Remote-SSH's rule (plan Phase 1 §5): open the window
            // regardless of whether the connection succeeded. The one
            // exception is `runtime_in_use` — another Hyprlayer already holds
            // this profile's lock, which is `tauri-plugin-single-instance`'s
            // job, not a recoverable connection state.
            // `app.manage(AppState::new(..))` moves above `runtime_events::spawn`
            // (plan Phase 1 §5): the spawned task now needs `handle.state::<AppState>()`.
            let mut client = match outcome {
                supervisor::Outcome::Connected { client, outdated } => {
                    app.manage(AppState::new(client.clone(), target.clone(), outdated));
                    Some(client)
                }
                supervisor::Outcome::Blocked { title, error } => {
                    let detail = supervisor::detail_for(&error);
                    // Logged as well as shown: the dialog renders nothing from a
                    // terminal launch, and the log is what a bug report carries.
                    log::error!("{title}: {detail}");
                    if error.code == "runtime_in_use" {
                        app.dialog()
                            .message(detail)
                            .title(title)
                            .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                            .blocking_show();
                        std::process::exit(1);
                    }
                    app.manage(AppState::failed(target.clone(), error));
                    None
                }
            };

            if let Some(live) = &client {
                match tauri::async_runtime::block_on(live.worktree_root()) {
                    Ok(worktree_root) => {
                        if let Err(e) = app
                            .asset_protocol_scope()
                            .allow_directory(worktree_root, true)
                        {
                            log::warn!(
                                "could not register worktree root with the asset scope: {e}"
                            );
                        }
                    }
                    // The handshake passed but the first request did not: the
                    // server is a build this app cannot talk to. Downgrade to
                    // `Failed` rather than exit — a version-skewed server is
                    // still a state the recovery screen can show and act on.
                    Err(e) if e.code == "server_disconnected" => {
                        log::error!(
                            "the server dropped the connection on its first request ({}); \
                             treating it as a build this app cannot talk to",
                            e.message
                        );
                        app.state::<AppState>()
                            .mark_failed(AppError::server_unavailable(
                                supervisor::incompatible_server_detail(),
                            ));
                        client = None;
                    }
                    Err(e) => log::warn!(
                        "could not fetch the worktree root from the server: {}",
                        e.message
                    ),
                }
            }

            if let Some(live) = client.clone() {
                tauri::async_runtime::block_on(attach_client(app.handle(), live));
            }

            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::Focused(is_focused) => {
                        // Read the live client rather than capturing one: a
                        // reconnect swaps it, and a captured clone would leave
                        // the PR watcher's foreground state stale with no error
                        // anywhere (plan Phase 8 §3).
                        let Some(client) = handle
                            .try_state::<AppState>()
                            .and_then(|state| state.client().ok())
                        else {
                            return;
                        };
                        let is_focused = *is_focused;
                        tauri::async_runtime::spawn(async move {
                            let _ = client.set_foreground(is_focused).await;
                        });
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        request_quit(&handle);
                    }
                    _ => {}
                });
            }

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
                "reveal-logs" => {
                    if let Err(e) = commands::diagnostics::reveal(app) {
                        log::warn!("could not reveal the logs: {}", e.message);
                    }
                }
                "copy-diagnostics" => {
                    let _ = app.emit("menu:copy-diagnostics", ());
                }
                _ => {}
            });

            Ok(())
        })
        .build(context)
        .expect("error while building tauri application")
        // Sessions and terminals live on the server now: quitting the app
        // must not touch the runtime, so there is nothing left to do here.
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                log::info!("exiting");
            }
        });
}

fn request_quit(handle: &tauri::AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let confirm = match handle.try_state::<AppState>().and_then(|s| s.client().ok()) {
            Some(client) => match client.get_app_settings().await {
                Ok(settings) => settings.quit.confirm,
                Err(e) => {
                    log::warn!(
                        "quitting without confirmation: settings unavailable: {}",
                        e.message
                    );
                    false
                }
            },
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

    let reveal_logs =
        tauri::menu::MenuItem::with_id(handle, "reveal-logs", "Reveal Logs", true, None::<&str>)?;
    let copy_diagnostics = tauri::menu::MenuItem::with_id(
        handle,
        "copy-diagnostics",
        "Copy Diagnostics",
        true,
        None::<&str>,
    )?;
    let help_menu = SubmenuBuilder::new(handle, "Help")
        .item(&reveal_logs)
        .item(&copy_diagnostics)
        .build()?;

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
