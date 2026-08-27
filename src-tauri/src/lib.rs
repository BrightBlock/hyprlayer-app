mod commands;
mod editors;
mod error;
mod events;
mod opencode;
mod pr_watch;
mod script_registry;
mod session_registry;
mod settings;
mod state;
mod terminal_registry;
mod webview_gestures;
mod window_geometry;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::process::binaries::{BinaryKind, BinaryResolver};
#[cfg(target_os = "macos")]
use tauri::Emitter;
use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder, Event};

use crate::settings::SettingsStore;
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
            commands::prompts::read_prompts_file,
            commands::prompts::write_prompts_file,
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

            let config = Arc::new(ConfigService::with_default_path()?);
            let settings_dir = app.path().app_config_dir()?;
            let settings = Arc::new(SettingsStore::load(settings_dir));

            let overrides: HashMap<BinaryKind, PathBuf> =
                tauri::async_runtime::block_on(settings.get())
                    .binary_overrides
                    .iter()
                    .map(|(k, v)| (*k, PathBuf::from(v)))
                    .collect();
            let binaries = Arc::new(BinaryResolver::initialize(overrides));

            let db_path = app.path().app_data_dir()?.join("hyprlayer.db");
            let db = Arc::new(hyprlayer_core::store::Db::open(&db_path)?);

            let settings_snapshot = tauri::async_runtime::block_on(settings.get());
            let worktree_root = settings_snapshot
                .worktree_root
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(hyprlayer_core::worktree::default_worktree_root);

            if let Ok(orphans) = db.list_orphans() {
                if !orphans.is_empty() {
                    log::warn!(
                        "found {} orphan session PID(s) from a prior run: {:?}",
                        orphans.len(),
                        orphans
                    );
                    let _ = db.clear_all_orphans();
                }
            }

            if let Err(e) = app
                .asset_protocol_scope()
                .allow_directory(&worktree_root, true)
            {
                log::warn!("could not register worktree root with the asset scope: {e}");
            }

            let focused = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let focus_changed = Arc::new(tokio::sync::Notify::new());
            if let Some(window) = app.get_webview_window("main") {
                let focused = focused.clone();
                let focus_changed = focus_changed.clone();
                let handle = app.handle().clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::Focused(is_focused) => {
                        focused.store(*is_focused, std::sync::atomic::Ordering::Relaxed);
                        focus_changed.notify_waiters();
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        request_quit(&handle);
                    }
                    _ => {}
                });
            }

            app.manage(AppState {
                config: config.clone(),
                binaries: binaries.clone(),
                settings,
                db,
                worktrees: Arc::new(hyprlayer_core::worktree::WorktreeManager::new(
                    worktree_root,
                )),
                index: Arc::new(hyprlayer_core::index::ThoughtsIndex::new()),
                index_watch: tokio::sync::Mutex::new(None),
                worktree_watch: tokio::sync::Mutex::new(None),
                sessions: Arc::new(session_registry::SessionRegistry::new()),
                opencode: Arc::new(opencode::OpenCodeManager::new()),
                terminals: Arc::new(terminal_registry::TerminalRegistry::new()),
                scripts: Arc::new(script_registry::ScriptRegistry::new()),
                focused,
                focus_changed,
            });

            let handle = app.handle().clone();
            let binaries_bg = binaries.clone();
            let config_bg = config.clone();
            tauri::async_runtime::spawn(async move {
                if binaries_bg.enrich_with_login_shell().await {
                    let _ = events::BinariesChanged.emit(&handle);
                }
                migrate_config_if_needed(&binaries_bg, &config_bg, &handle).await;
                rebuild_thoughts_index(handle.clone()).await;
            });

            let handle = app.handle().clone();
            let config_bg = config.clone();
            tauri::async_runtime::spawn(async move {
                let path = config_bg.path().to_path_buf();
                let key = |p: &std::path::Path| {
                    std::fs::metadata(p)
                        .ok()
                        .map(|m| (m.modified().ok(), m.len()))
                };
                let mut last = key(&path);
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let current = key(&path);
                    if current != last {
                        last = current;
                        config_bg.invalidate().await;
                        let _ = events::ConfigChanged.emit(&handle);
                        rebuild_thoughts_index(handle.clone()).await;
                    }
                }
            });

            if let Some(omarchy_dir) = hyprlayer_core::omarchy::current_dir() {
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
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        let current = key(&omarchy_dir);
                        if current != last {
                            last = current;
                            let _ = events::OmarchyThemeChanged.emit(&handle);
                        }
                    }
                });
            }

            pr_watch::spawn(app.handle().clone());

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
                state.terminals.kill_all_blocking();
                state.sessions.kill_all_blocking();
                state.opencode.kill_blocking();
            }
        });
}

fn request_quit(handle: &tauri::AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let confirm = match handle.try_state::<AppState>() {
            Some(state) => state.settings.get().await.quit.confirm,
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

pub(crate) async fn rebuild_thoughts_index(app: tauri::AppHandle) {
    use tauri::Manager;

    let state = app.state::<AppState>();
    let roots = match state.config.load().await {
        Ok(loaded) => hyprlayer_core::index::thoughts_roots(&loaded.config),
        Err(_) => Vec::new(),
    };

    let index = state.index.clone();
    let scan_roots = roots.clone();
    let stats = tauri::async_runtime::spawn_blocking(move || index.full_scan(&scan_roots)).await;
    if let Ok(stats) = &stats {
        log::info!(
            "thoughts index: {} docs ({} skipped, {} errors) across {} roots",
            stats.indexed,
            stats.skipped,
            stats.errors,
            roots.len()
        );
    }

    let watch = if roots.is_empty() {
        None
    } else {
        let emit_handle = app.clone();
        let callback = Box::new(move |event: hyprlayer_core::index::watcher::IndexEvent| {
            use hyprlayer_core::index::watcher::IndexEvent;
            let payload = match event {
                IndexEvent::Updated { ids } => events::ThoughtsIndexUpdated {
                    kind: "updated".into(),
                    ids,
                },
                IndexEvent::Removed { ids } => events::ThoughtsIndexUpdated {
                    kind: "removed".into(),
                    ids,
                },
                IndexEvent::Rescanned => events::ThoughtsIndexUpdated {
                    kind: "rescanned".into(),
                    ids: Vec::new(),
                },
            };
            let _ = payload.emit(&emit_handle);
        });
        let watch_index = state.index.clone();
        let armed = tauri::async_runtime::spawn_blocking(move || {
            hyprlayer_core::index::watcher::watch_roots(watch_index, roots, callback)
        })
        .await;
        match armed {
            Ok(Ok(handle)) => Some(handle),
            Ok(Err(e)) => {
                log::warn!("thoughts watcher failed to start: {e}");
                None
            }
            Err(e) => {
                log::warn!("thoughts watcher task failed: {e}");
                None
            }
        }
    };
    *state.index_watch.lock().await = watch;

    let _ = events::ThoughtsIndexUpdated {
        kind: "rescanned".into(),
        ids: Vec::new(),
    }
    .emit(&app);
}

pub(crate) async fn migrate_config_if_needed(
    binaries: &BinaryResolver,
    config: &ConfigService,
    handle: &tauri::AppHandle,
) {
    if !config.exists() {
        return;
    }
    let needs = matches!(
        config.load().await,
        Err(ref e) if e.code() == "config_needs_migration"
    );
    if !needs {
        return;
    }
    let Some(home) = dirs::home_dir() else {
        return;
    };
    match HyprCli::new(binaries).storage_info(&home).await {
        Ok(_) => {
            config.invalidate().await;
            let _ = events::ConfigChanged.emit(handle);
        }
        Err(e) => log::warn!("config migration nudge failed: {e}"),
    }
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
