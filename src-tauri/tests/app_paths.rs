//! `AppPaths` must resolve what Tauri's `app.path()` does: a different dir derives a different
//! socket and loses `hosts.json` and `client-id`.

use hyprlayer_client::paths::AppPaths;
use tauri::Manager;

fn tauri_paths(identifier: &str) -> AppPaths {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().identifier = identifier.to_string();
    let app = tauri::test::mock_builder()
        .build(context)
        .expect("mock app builds");
    let path = app.path();
    AppPaths {
        config: path.app_config_dir().unwrap(),
        data: path.app_data_dir().unwrap(),
        log: path.app_log_dir().unwrap(),
    }
}

#[test]
fn app_paths_match_tauris_for_both_identifiers() {
    for identifier in ["ai.brightblock.hyprlayer", "ai.brightblock.hyprlayer.dev"] {
        assert_eq!(
            AppPaths::resolve(identifier).unwrap(),
            tauri_paths(identifier),
            "{identifier}"
        );
    }
}
