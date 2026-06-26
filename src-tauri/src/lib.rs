mod commands;
mod editors;
mod error;
mod events;
mod opencode;
mod pr_watch;
mod session_registry;
mod settings;
mod state;
mod terminal_registry;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hyprlayer_core::config::ConfigService;
use hyprlayer_core::hyprcli::HyprCli;
use hyprlayer_core::process::binaries::{BinaryKind, BinaryResolver};
use tauri::{Emitter, Manager};
use tauri_specta::{collect_commands, collect_events, Builder, Event};

use crate::settings::SettingsStore;
use crate::state::AppState;

fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::projects::get_bootstrap,
            commands::projects::get_project,
            commands::projects::add_project,
            commands::projects::clone_project,
            commands::projects::remove_project,
            commands::projects::list_recent_repos,
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
            commands::workstreams::create_workstream,
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
            commands::workstreams::worktree_stage,
            commands::workstreams::worktree_unstage,
            commands::workstreams::worktree_discard,
            commands::workstreams::worktree_file_sides,
            commands::workstreams::worktree_list_dir,
            commands::workstreams::worktree_read_file,
            commands::workstreams::list_slash_commands,
            commands::workstreams::worktree_search_files,
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
            commands::sessions::list_sessions,
            commands::sessions::get_session_log,
            commands::sessions::attach_session,
            commands::sessions::list_active_sessions,
            commands::attachments::import_attachment,
            commands::attachments::save_attachment,
            commands::system::set_binary_override,
            commands::system::redetect_binaries,
            commands::system::detect_cli_install_methods,
            commands::system::install_hyprlayer_cli,
            commands::system::get_app_settings,
            commands::system::set_app_settings,
            commands::system::detect_editors,
            commands::system::github_username,
            commands::system::default_username,
            commands::system::play_sound,
            commands::harnesses::opencode_catalog,
            commands::harnesses::opencode_auth_list,
            commands::harnesses::opencode_set_provider_key,
            commands::harnesses::opencode_remove_provider,
            commands::harnesses::opencode_set_default_model,
            commands::harnesses::install_opencode,
            commands::harnesses::claude_auth_status,
            commands::ollama::ollama_status,
            commands::ollama::ollama_set_endpoint,
            commands::ollama::ollama_set_selected_models,
            commands::ollama::ollama_set_default_model,
            commands::titles::suggest_tab_title,
            commands::terminal::terminal_open,
            commands::terminal::terminal_attach,
            commands::terminal::terminal_write,
            commands::terminal::terminal_resize,
            commands::terminal::terminal_close,
            commands::terminal::spawn_cli_terminal,
            commands::terminal::spawn_shell_terminal,
            commands::onboarding::get_ai_status,
            commands::onboarding::probe_default_thoughts,
            commands::onboarding::init_thoughts_repo,
        ])
        .events(collect_events![
            events::ConfigChanged,
            events::BinariesChanged,
            events::ThoughtsIndexUpdated,
            events::PrStatusChanged,
            events::WorktreeChanged,
            events::SessionLifecycle
        ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // Dev convenience only — cargo test regenerates bindings authoritatively.
    // Anchored to the crate dir (not cwd: a Finder-launched debug bundle runs
    // with cwd=/) and non-fatal so a read-only checkout can't kill startup.
    #[cfg(debug_assertions)]
    if let Err(e) = builder.export(
        specta_typescript::Typescript::default()
            .header("// Generated by tauri-specta. Do not edit.\n"),
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
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        // In-app self-update. Reads its endpoint + pubkey from tauri.conf.json
        // → plugins.updater; the frontend (src/lib/updater.ts) drives it via
        // @tauri-apps/plugin-updater. `process` provides the relaunch() the
        // frontend calls after an update installs.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init());

    #[cfg(debug_assertions)]
    let tauri_builder = tauri_builder.plugin(tauri_plugin_mcp_bridge::init());

    // macOS only: replace the system default menu with one that omits "Close
    // Window". The default binds ⌘W to it — one key from ⌘⇧W (Close Agent Tab) —
    // so a slipped tab-close shuts the lone window out from under every running
    // session. ⌘Q (Quit) and the red traffic-light button remain the deliberate
    // ways out. See `macos_menu`. (Other OSes get no default menu, so no ⌘W.)
    #[cfg(target_os = "macos")]
    let tauri_builder = tauri_builder.menu(macos_menu);

    tauri_builder
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);

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

            // Reap any orphan claude PIDs a previous crash left in the ledger,
            // then clear it. (We log rather than blind-kill stale PIDs — a PID
            // may have been reused; verified-kill is a follow-up.)
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

            // Let the webview load image attachments (worktree
            // `.hyprlayer/attachments/`) through the asset protocol. Allowing
            // the worktree root at runtime is more reliable than a `$VAR` glob
            // in tauri.conf.json, which doesn't expand for the asset scope.
            if let Err(e) = app
                .asset_protocol_scope()
                .allow_directory(&worktree_root, true)
            {
                log::warn!("could not register worktree root with the asset scope: {e}");
            }

            // Track OS window focus so background pollers can stand down while the
            // app is in the background. Seeded `true` (the window comes up focused);
            // the window event below keeps it current.
            let focused = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let focus_changed = Arc::new(tokio::sync::Notify::new());
            if let Some(window) = app.get_webview_window("main") {
                let focused = focused.clone();
                let focus_changed = focus_changed.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(is_focused) = event {
                        focused.store(*is_focused, std::sync::atomic::Ordering::Relaxed);
                        focus_changed.notify_waiters();
                    }
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
                focused,
                focus_changed,
            });

            // 1. Fold the login-shell PATH in (GUI apps start with launchd's
            //    bare PATH), THEN nudge the CLI to migrate a pre-v4 config,
            //    THEN build the thoughts index — chained, so each step sees
            //    the previous one's effects and never races it.
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

            // 2. Watch the config file for changes (the CLI owns writes); a
            //    cheap (mtime, len) poll is plenty at this cadence — length
            //    catches same-tick rewrites on coarse-mtime filesystems.
            //    Config changes can move the thoughts roots → rebuild.
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

            // 3. Watch each active workstream's PR server-side and push a
            //    PrStatusChanged event on change, so the merge/archive bar stays
            //    in sync (e.g. a PR merged on GitHub flips Merge → Archive)
            //    without the frontend polling gh on a timer.
            pr_watch::spawn(app.handle().clone());

            // Relay macOS menu item clicks to the frontend as Tauri events.
            #[cfg(target_os = "macos")]
            app.on_menu_event(|app, event| {
                match event.id().as_ref() {
                    "check-updates" => { let _ = app.emit("menu:check-for-updates", ()); }
                    "open-settings" => { let _ = app.emit("menu:open-settings", ()); }
                    _ => {}
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Best-effort: kill any live terminal shells when the app quits, so
            // they don't outlive the window. The PTY master's SIGHUP-on-drop at
            // process exit is the backstop if this path is ever missed.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                use tauri::Manager;
                app.state::<AppState>().terminals.kill_all_blocking();
            }
        });
}

/// The macOS application menu: tauri's standard default, faithfully — clipboard
/// (Edit), Quit (⌘Q), Hide, fullscreen, minimize all unchanged — but with the
/// "Close Window" item dropped so ⌘W is inert (see the call site for why). The
/// default binds Close Window in both a File submenu and the Window submenu, so
/// both go: the File submenu held nothing else and is omitted, and the Window
/// submenu keeps only Minimize/Zoom.
#[cfg(target_os = "macos")]
fn macos_menu(handle: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{AboutMetadata, MenuBuilder, SubmenuBuilder};

    // Mirror tauri's default About metadata so the About panel is unchanged.
    let pkg = handle.package_info();
    let config = handle.config();
    let about = AboutMetadata {
        name: Some(pkg.name.clone()),
        version: Some(pkg.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    let check_updates =
        tauri::menu::MenuItem::with_id(handle, "check-updates", "Check for Updates…", true, None::<&str>)?;
    let open_settings =
        tauri::menu::MenuItem::with_id(handle, "open-settings", "Settings", true, Some("CmdOrCtrl+,"))?;

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
        .quit()
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

    // Standard Window menu, but WITHOUT .close_window(): that's the ⌘W we drop.
    let window_menu = SubmenuBuilder::new(handle, "Window")
        .minimize()
        .maximize()
        .build()?;

    // Empty on macOS by default — the OS injects its own Help search field.
    let help_menu = SubmenuBuilder::new(handle, "Help").build()?;

    MenuBuilder::new(handle)
        .items(&[&app_menu, &edit_menu, &view_menu, &window_menu, &help_menu])
        .build()
}

/// (Re)build the thoughts index and its filesystem watcher from the current
/// config. Called at startup, after config changes (roots may move), and
/// after adding a project. Replacing the stored WatchHandle drops — and
/// thereby stops — the previous watcher.
pub(crate) async fn rebuild_thoughts_index(app: tauri::AppHandle) {
    use tauri::Manager;

    let state = app.state::<AppState>();
    let roots = match state.config.load().await {
        Ok(loaded) => hyprlayer_core::index::thoughts_roots(&loaded.config),
        Err(_) => Vec::new(), // no config -> empty library, stop watching
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
        match hyprlayer_core::index::watcher::watch_roots(state.index.clone(), roots, callback) {
            Ok(handle) => Some(handle),
            Err(e) => {
                log::warn!("thoughts watcher failed to start: {e}");
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

/// If a direct config read fails because the file is pre-v4, run one cheap
/// `hyprlayer storage info` (cwd irrelevant) — the CLI's load path migrates
/// and rewrites the file — then re-read and tell the UI. No-op on v4 configs,
/// missing configs, or when the hyprlayer binary can't be found (logged).
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
    /// Regenerates `src/lib/bindings.ts` (same output as the dev-server
    /// export); CI runs tests then asserts the file is committed unchanged.
    #[test]
    fn typescript_bindings_export() {
        std::fs::create_dir_all("../src/lib").expect("frontend src/lib exists");
        super::specta_builder()
            .export(
                specta_typescript::Typescript::default()
                    .header("// Generated by tauri-specta. Do not edit.\n"),
                "../src/lib/bindings.ts",
            )
            .expect("bindings must export cleanly");
    }
}
