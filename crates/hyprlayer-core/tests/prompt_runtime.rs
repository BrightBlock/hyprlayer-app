#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::runtime::builder::RuntimeBuilder;
use hyprlayer_core::runtime::projects::ImportFolderRequest;
use hyprlayer_core::runtime::prompts::{
    parse_prompts_file, PromptConflictAction, PromptConflictKind, PromptExportEntry,
    PromptExportRequest, PromptExportSelection, PromptImportDecision, PromptImportMode,
    PromptImportPreview, PromptImportRequest, PromptImportResult,
};
use hyprlayer_core::runtime::HyprlayerRuntime;
use hyprlayer_core::settings::PromptSettings;
use hyprlayer_core::CoreError;

fn test_environment(home: &Path) -> RuntimeEnvironment {
    let mut process_env = BTreeMap::new();
    if let Some(path) = std::env::var_os("PATH") {
        process_env.insert(OsString::from("PATH"), path);
    }
    RuntimeEnvironment {
        home_dir: home.to_path_buf(),
        process_cwd: home.to_path_buf(),
        process_env,
    }
}

fn test_paths(root: &Path) -> RuntimePaths {
    RuntimePaths {
        hyprlayer_config_file: root.join("hyprlayer-config.json"),
        app_config_dir: root.join("app-config"),
        app_data_dir: root.join("app-data"),
        default_worktree_root: root.join("worktrees"),
        default_repos_root: root.join("repos"),
        opencode_config_file: root.join("opencode.json"),
        opencode_auth_file: root.join("opencode-auth.json"),
        opencode_data_dir: root.join("opencode-data"),
        claude_config_dir: root.join("claude"),
        claude_managed_settings_files: Vec::new(),
        codex_home: root.join("codex"),
        agent_roots: Vec::new(),
        download_dir: root.join("downloads"),
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    runtime: Arc<HyprlayerRuntime>,
}

async fn fixture() -> Fixture {
    fixture_with(serde_json::json!({})).await
}

async fn fixture_with(prompts: serde_json::Value) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("profile");
    let paths = test_paths(&root);
    std::fs::create_dir_all(&paths.app_config_dir).unwrap();
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    std::fs::write(
        paths.app_config_dir.join("settings.json"),
        serde_json::to_string(&serde_json::json!({ "prompts": prompts })).unwrap(),
    )
    .unwrap();

    let runtime = RuntimeBuilder::new(test_environment(dir.path()), paths)
        .open()
        .await
        .unwrap();

    Fixture {
        root: dir.path().to_path_buf(),
        _dir: dir,
        runtime,
    }
}

impl Fixture {
    fn write_source(&self, doc: serde_json::Value) -> String {
        let path = self.root.join("import.hlprompts");
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        path.display().to_string()
    }

    async fn register_project(&self, name: &str) -> String {
        let folder = self.root.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        self.runtime
            .projects()
            .import_folder(ImportFolderRequest {
                path: folder.display().to_string(),
                directory_name: None,
            })
            .await
            .unwrap()
            .path
    }

    async fn preview(&self, path: &str, taken: &[&str]) -> PromptImportPreview {
        self.runtime
            .prompts()
            .preview_import(PromptImportRequest {
                path: path.to_string(),
                taken_chords: taken.iter().map(|c| c.to_string()).collect(),
            })
            .await
            .unwrap()
    }

    async fn apply(
        &self,
        path: &str,
        preview: &PromptImportPreview,
        taken: &[&str],
        actions: &[(&str, PromptConflictAction)],
    ) -> PromptImportResult {
        self.runtime
            .prompts()
            .apply_import(PromptImportDecision {
                path: path.to_string(),
                expected_source_sha256: preview.source_sha256.clone(),
                mode: PromptImportMode::Merge,
                conflict_actions: actions
                    .iter()
                    .map(|(key, action)| (key.to_string(), *action))
                    .collect(),
                taken_chords: taken.iter().map(|c| c.to_string()).collect(),
            })
            .await
            .unwrap()
    }
}

fn file(prompts: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "hlprompts": 1, "prompts": prompts })
}

fn export_entry() -> PromptExportEntry {
    PromptExportEntry {
        name: "Backfill tests".into(),
        body: "Write tests for {{changed_files}}.".into(),
        chord: Some("mod+shift+t".into()),
        overrides: BTreeMap::new(),
    }
}

async fn exported(fx: &Fixture, selection: PromptExportSelection) -> serde_json::Value {
    let target = fx.root.join("out.hlprompts");
    let result = fx
        .runtime
        .prompts()
        .export(PromptExportRequest {
            path: target.display().to_string(),
            selection,
        })
        .await
        .unwrap();
    serde_json::from_str(&std::fs::read_to_string(&result.path).unwrap()).unwrap()
}

// ---------------------------------------------------------------- export

#[tokio::test]
async fn export_round_trips_through_parse_with_chords_and_overrides_on() {
    let fx = fixture().await;
    let project = fx.register_project("app").await;
    let mut entry = export_entry();
    entry
        .overrides
        .insert(project, "vitest override".to_string());

    let doc = exported(
        &fx,
        PromptExportSelection {
            entries: vec![entry],
            include_chords: true,
            include_overrides: true,
            source: None,
        },
    )
    .await;

    let parsed = parse_prompts_file(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(parsed.prompts.len(), 1);
    let only = &parsed.prompts[0];
    assert_eq!(only.name, "Backfill tests");
    assert_eq!(only.body, "Write tests for {{changed_files}}.");
    assert_eq!(only.chord.as_deref(), Some("mod+shift+t"));
    assert_eq!(
        only.overrides,
        BTreeMap::from([("app".to_string(), "vitest override".to_string())])
    );

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn export_omits_chords_when_include_chords_is_off() {
    let fx = fixture().await;
    let project = fx.register_project("app").await;
    let mut entry = export_entry();
    entry
        .overrides
        .insert(project, "vitest override".to_string());

    let doc = exported(
        &fx,
        PromptExportSelection {
            entries: vec![entry],
            include_chords: false,
            include_overrides: true,
            source: None,
        },
    )
    .await;

    let prompt = &doc["prompts"][0];
    assert!(prompt.get("chord").is_none());
    assert_eq!(prompt["overrides"]["app"], "vitest override");

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn export_omits_overrides_when_include_overrides_is_off() {
    let fx = fixture().await;
    let project = fx.register_project("app").await;
    let mut entry = export_entry();
    entry
        .overrides
        .insert(project, "vitest override".to_string());

    let doc = exported(
        &fx,
        PromptExportSelection {
            entries: vec![entry],
            include_chords: true,
            include_overrides: false,
            source: None,
        },
    )
    .await;

    let prompt = &doc["prompts"][0];
    assert!(prompt.get("overrides").is_none());
    assert_eq!(prompt["chord"], "mod+shift+t");

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn export_writes_the_version_and_an_optional_source() {
    let fx = fixture().await;
    let doc = exported(
        &fx,
        PromptExportSelection {
            entries: vec![export_entry()],
            include_chords: true,
            include_overrides: false,
            source: Some("jon".into()),
        },
    )
    .await;

    assert_eq!(doc["hlprompts"], 1);
    assert_eq!(doc["source"], "jon");

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn export_appends_the_extension_and_reports_the_count() {
    let fx = fixture().await;
    let target = fx.root.join("share");
    let result = fx
        .runtime
        .prompts()
        .export(PromptExportRequest {
            path: target.display().to_string(),
            selection: PromptExportSelection {
                entries: vec![export_entry(), export_entry()],
                include_chords: true,
                include_overrides: true,
                source: None,
            },
        })
        .await
        .unwrap();

    assert!(result.path.ends_with(".hlprompts"), "{}", result.path);
    assert_eq!(result.prompt_count, 2);
    assert!(Path::new(&result.path).is_file());

    fx.runtime.shutdown().await;
}

// ----------------------------------------------------------------- parse

#[test]
fn parse_rejects_non_json() {
    let err = parse_prompts_file("not json {").unwrap_err();
    assert_eq!(err.code(), "prompts_file_invalid");
    assert!(err.to_string().contains("valid JSON"), "{err}");
}

#[test]
fn parse_rejects_the_wrong_shape() {
    for text in [r#"{"nope": true}"#, r#"{"prompts": "x"}"#] {
        let err = parse_prompts_file(text).unwrap_err();
        assert_eq!(
            err.to_string(),
            "This doesn't look like a Hyprlayer prompt file."
        );
    }
}

#[test]
fn parse_rejects_a_newer_format_version() {
    let text = serde_json::json!({ "hlprompts": 99, "prompts": [{ "name": "X", "body": "b" }] });
    let err = parse_prompts_file(&text.to_string()).unwrap_err();
    assert!(err.to_string().contains("newer version"), "{err}");
}

#[test]
fn parse_drops_entries_with_no_name_and_tolerates_missing_bodies() {
    let text = serde_json::json!({
        "hlprompts": 1,
        "prompts": [{ "body": "no name" }, { "name": "  Kept  " }],
    });
    let parsed = parse_prompts_file(&text.to_string()).unwrap();
    assert_eq!(parsed.prompts.len(), 1);
    assert_eq!(parsed.prompts[0].name, "Kept");
    assert_eq!(parsed.prompts[0].body, "");
    assert!(parsed.prompts[0].chord.is_none());
    assert!(parsed.prompts[0].overrides.is_empty());
}

#[test]
fn parse_errors_when_there_is_nothing_importable() {
    let text = serde_json::json!({ "hlprompts": 1, "prompts": [{ "body": "x" }] });
    let err = parse_prompts_file(&text.to_string()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "This prompt file has no prompts to import."
    );
}

#[tokio::test]
async fn preview_rejects_a_file_over_the_size_cap() {
    let fx = fixture().await;
    let path = fx.root.join("huge.hlprompts");
    std::fs::write(&path, vec![b'x'; 4 * 1024 * 1024 + 1]).unwrap();

    let err = fx
        .runtime
        .prompts()
        .preview_import(PromptImportRequest {
            path: path.display().to_string(),
            taken_chords: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::PromptsFileTooLarge), "{err:?}");

    fx.runtime.shutdown().await;
}

// ------------------------------------------------------------- preview

fn planning_file() -> serde_json::Value {
    file(serde_json::json!([
        { "name": "Fresh", "body": "new one" },
        { "name": "Commit", "body": "clashes with a built-in" },
        { "name": "Mine", "body": "clashes with a custom", "chord": "mod+shift+t" },
        { "name": "WithOverride", "body": "x", "overrides": { "app": "a", "ghost": "g" } },
    ]))
}

async fn planning_fixture() -> Fixture {
    fixture_with(serde_json::json!({
        "custom": [{ "id": "c1", "name": "Mine", "body": "existing" }],
    }))
    .await
}

#[tokio::test]
async fn preview_flags_builtin_and_custom_name_conflicts_else_new() {
    let fx = planning_fixture().await;
    let path = fx.write_source(planning_file());
    let preview = fx.preview(&path, &["mod+shift+t"]).await;

    let kinds: BTreeMap<&str, PromptConflictKind> = preview
        .conflicts
        .iter()
        .map(|c| (c.key.as_str(), c.kind))
        .collect();
    assert!(!kinds.contains_key("0"));
    assert_eq!(kinds.get("1"), Some(&PromptConflictKind::Builtin));
    assert_eq!(kinds.get("2"), Some(&PromptConflictKind::Custom));

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn preview_detects_a_taken_chord_and_unknown_override_projects() {
    let fx = planning_fixture().await;
    fx.register_project("app").await;
    let path = fx.write_source(planning_file());
    let preview = fx.preview(&path, &["mod+shift+t"]).await;

    assert!(preview.prompts[2].chord_taken);
    assert!(!preview.prompts[0].chord_taken);
    assert_eq!(
        preview.prompts[3].unknown_projects,
        vec!["ghost".to_string()]
    );
    assert_eq!(preview.source_version, 1);
    assert_eq!(preview.source_sha256.len(), 64);

    fx.runtime.shutdown().await;
}

// --------------------------------------------------------------- apply

async fn persisted(fx: &Fixture) -> PromptSettings {
    fx.runtime.prompts().list().await.unwrap()
}

#[tokio::test]
async fn apply_adds_a_brand_new_prompt_with_a_free_chord() {
    let fx = fixture().await;
    let path = fx.write_source(file(
        serde_json::json!([{ "name": "Fresh", "body": "b", "chord": "mod+j" }]),
    ));
    let preview = fx.preview(&path, &[]).await;
    let result = fx.apply(&path, &preview, &[], &[]).await;

    assert_eq!((result.added, result.replaced, result.renamed), (1, 0, 0));
    assert_eq!(result.settings.custom.len(), 1);
    let added = &result.settings.custom[0];
    assert_eq!(added.name, "Fresh");
    assert_eq!(added.body, "b");
    assert_eq!(added.chord.as_deref(), Some("mod+j"));
    assert!(!added.pinned);
    assert!(added.overrides.is_empty());
    assert_eq!(result.first_id.as_deref(), Some(added.id.as_str()));
    assert_eq!(persisted(&fx).await.custom.len(), 1);

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_drops_a_chord_that_is_already_taken() {
    let fx = fixture().await;
    let path = fx.write_source(file(
        serde_json::json!([{ "name": "Fresh", "body": "b", "chord": "mod+shift+t" }]),
    ));
    let preview = fx.preview(&path, &["mod+shift+t"]).await;
    let result = fx.apply(&path, &preview, &["mod+shift+t"], &[]).await;

    assert_eq!(result.settings.custom[0].chord, None);

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_gives_two_imports_competing_for_one_free_chord_to_the_first_only() {
    let fx = fixture().await;
    let path = fx.write_source(file(serde_json::json!([
        { "name": "A", "body": "a", "chord": "mod+k" },
        { "name": "B", "body": "b", "chord": "mod+k" },
    ])));
    let preview = fx.preview(&path, &[]).await;
    let result = fx.apply(&path, &preview, &[], &[]).await;

    assert_eq!(result.settings.custom[0].chord.as_deref(), Some("mod+k"));
    assert_eq!(result.settings.custom[1].chord, None);

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_keep_both_suffixes_a_name_that_collides() {
    let fx = fixture_with(serde_json::json!({
        "custom": [{ "id": "c1", "name": "Mine", "body": "existing" }],
    }))
    .await;
    let path = fx.write_source(file(
        serde_json::json!([{ "name": "Mine", "body": "incoming" }]),
    ));
    let preview = fx.preview(&path, &[]).await;
    let result = fx
        .apply(
            &path,
            &preview,
            &[],
            &[("0", PromptConflictAction::KeepBoth)],
        )
        .await;

    assert_eq!(result.renamed, 1);
    assert_eq!(result.settings.custom.len(), 2);
    assert_eq!(result.settings.custom[0].body, "existing");
    assert_eq!(result.settings.custom[1].name, "Mine (imported)");

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_replace_overwrites_a_custom_prompts_body_in_place() {
    let fx = fixture_with(serde_json::json!({
        "custom": [{ "id": "c1", "name": "Mine", "body": "old", "chord": null }],
    }))
    .await;
    let path = fx.write_source(file(
        serde_json::json!([{ "name": "Mine", "body": "incoming", "chord": "mod+m" }]),
    ));
    let preview = fx.preview(&path, &[]).await;
    let result = fx
        .apply(
            &path,
            &preview,
            &[],
            &[("0", PromptConflictAction::Replace)],
        )
        .await;

    assert_eq!(result.replaced, 1);
    assert_eq!(result.settings.custom.len(), 1);
    assert_eq!(result.settings.custom[0].id, "c1");
    assert_eq!(result.settings.custom[0].body, "incoming");
    assert_eq!(result.settings.custom[0].chord.as_deref(), Some("mod+m"));
    assert_eq!(result.first_id.as_deref(), Some("c1"));

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_replace_of_a_builtin_writes_its_builtin_body_not_a_custom() {
    let fx = fixture().await;
    let path = fx.write_source(file(
        serde_json::json!([{ "name": "Commit", "body": "my commit rules" }]),
    ));
    let preview = fx.preview(&path, &[]).await;
    let result = fx
        .apply(
            &path,
            &preview,
            &[],
            &[("0", PromptConflictAction::Replace)],
        )
        .await;

    assert_eq!(
        result
            .settings
            .builtin_bodies
            .get("commit")
            .map(String::as_str),
        Some("my commit rules")
    );
    assert!(result.settings.custom.is_empty());
    assert_eq!(result.first_id.as_deref(), Some("commit"));

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_skip_drops_a_conflicting_entry_entirely() {
    let fx = fixture().await;
    let path = fx.write_source(file(serde_json::json!([{ "name": "Commit", "body": "x" }])));
    let preview = fx.preview(&path, &[]).await;
    let result = fx
        .apply(&path, &preview, &[], &[("0", PromptConflictAction::Skip)])
        .await;

    assert_eq!(
        (
            result.added,
            result.replaced,
            result.renamed,
            result.skipped
        ),
        (0, 0, 0, 1)
    );
    assert!(result.settings.custom.is_empty());
    assert!(result.settings.builtin_bodies.is_empty());

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_rehomes_known_overrides_by_name_and_drops_unknown_ones() {
    let fx = fixture().await;
    let project = fx.register_project("app").await;
    let path = fx.write_source(file(serde_json::json!([{
        "name": "P",
        "body": "b",
        "overrides": { "app": "for app", "ghost": "dropped" },
    }])));
    let preview = fx.preview(&path, &[]).await;
    let result = fx.apply(&path, &preview, &[], &[]).await;

    let overrides = &result.settings.custom[0].overrides;
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides.get(&project).map(String::as_str), Some("for app"));

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_skips_unchecked_candidates() {
    let fx = fixture().await;
    let path = fx.write_source(file(serde_json::json!([
        { "name": "A", "body": "a" },
        { "name": "B", "body": "b" },
    ])));
    let preview = fx.preview(&path, &[]).await;
    let result = fx
        .apply(&path, &preview, &[], &[("0", PromptConflictAction::Skip)])
        .await;

    assert_eq!((result.added, result.skipped), (1, 1));
    assert_eq!(result.settings.custom.len(), 1);
    assert_eq!(result.settings.custom[0].name, "B");

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_rejects_a_source_that_changed_since_the_preview() {
    let fx = fixture().await;
    let path = fx.write_source(file(serde_json::json!([{ "name": "Fresh", "body": "b" }])));
    let preview = fx.preview(&path, &[]).await;

    fx.write_source(file(
        serde_json::json!([{ "name": "Swapped", "body": "different" }]),
    ));

    let err = fx
        .runtime
        .prompts()
        .apply_import(PromptImportDecision {
            path: path.clone(),
            expected_source_sha256: preview.source_sha256.clone(),
            mode: PromptImportMode::Merge,
            conflict_actions: BTreeMap::new(),
            taken_chords: Vec::new(),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, CoreError::PromptsSourceChanged), "{err:?}");
    assert_eq!(err.code(), "prompts_source_changed");
    assert!(persisted(&fx).await.custom.is_empty());

    fx.runtime.shutdown().await;
}

#[tokio::test]
async fn apply_defaults_absent_keys_to_the_requested_mode() {
    let fx = fixture_with(serde_json::json!({
        "custom": [{ "id": "c1", "name": "Mine", "body": "old" }],
    }))
    .await;
    let path = fx.write_source(file(
        serde_json::json!([{ "name": "Mine", "body": "incoming" }]),
    ));
    let preview = fx.preview(&path, &[]).await;

    let result = fx
        .runtime
        .prompts()
        .apply_import(PromptImportDecision {
            path: path.clone(),
            expected_source_sha256: preview.source_sha256.clone(),
            mode: PromptImportMode::Replace,
            conflict_actions: BTreeMap::new(),
            taken_chords: Vec::new(),
        })
        .await
        .unwrap();

    assert_eq!(result.replaced, 1);
    assert_eq!(result.settings.custom.len(), 1);
    assert_eq!(result.settings.custom[0].body, "incoming");

    fx.runtime.shutdown().await;
}
