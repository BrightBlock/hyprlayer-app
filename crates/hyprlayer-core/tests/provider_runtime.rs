#![cfg(feature = "test-support")]

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use hyprlayer_core::env::{RuntimeEnvironment, RuntimePaths};
use hyprlayer_core::runtime::builder::RuntimeBuilder;
use hyprlayer_core::runtime::ports::{
    CredentialLookup, CredentialLookupError, CredentialLookupRequest, CredentialLookupResult,
    CredentialValue, ProviderHttp, ProviderHttpError, ProviderHttpRequest, ProviderHttpResponse,
};
use hyprlayer_core::CoreError;

fn test_environment(home: &Path) -> RuntimeEnvironment {
    RuntimeEnvironment {
        home_dir: home.to_path_buf(),
        process_cwd: home.to_path_buf(),
        process_env: std::collections::BTreeMap::new(),
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

const MODELS_BODY: &str = r#"{"data":[{"id":"claude-sonnet-5","max_input_tokens":200000}]}"#;

struct FakeCredentialLookup {
    calls: Arc<AtomicUsize>,
    result: fn() -> std::result::Result<CredentialLookupResult, CredentialLookupError>,
}

impl CredentialLookup for FakeCredentialLookup {
    fn lookup(
        &self,
        _request: &CredentialLookupRequest,
    ) -> std::result::Result<CredentialLookupResult, CredentialLookupError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.result)()
    }
}

fn not_found_lookup(calls: Arc<AtomicUsize>) -> Arc<FakeCredentialLookup> {
    Arc::new(FakeCredentialLookup {
        calls,
        result: || Ok(CredentialLookupResult::NotFound),
    })
}

struct FakeProviderHttp {
    calls: Arc<AtomicUsize>,
    captured_headers: Arc<StdMutex<Vec<(String, String)>>>,
    status: u16,
    body: String,
}

impl FakeProviderHttp {
    fn ok(body: impl Into<String>) -> (Arc<Self>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Self {
                calls: calls.clone(),
                captured_headers: Arc::new(StdMutex::new(Vec::new())),
                status: 200,
                body: body.into(),
            }),
            calls,
        )
    }
}

impl ProviderHttp for FakeProviderHttp {
    fn get<'a>(
        &'a self,
        request: ProviderHttpRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<ProviderHttpResponse, ProviderHttpError>>
                + Send
                + 'a,
        >,
    > {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.captured_headers.lock().unwrap() = request.headers.clone();
        let status = self.status;
        let body = self.body.clone();
        Box::pin(async move { Ok(ProviderHttpResponse { status, body }) })
    }
}

fn credentials_json(token: &str) -> String {
    format!(
        r#"{{"claudeAiOauth":{{"accessToken":"{token}","refreshToken":"x","expiresAt":1,"scopes":["user:inference"]}}}}"#
    )
}

// The explicit key is set the same way the real `SettingsStore::load` would
// pick it up: writing `settings.json` before the profile is opened.
fn write_explicit_api_key_settings(paths: &RuntimePaths, key: &str) {
    std::fs::create_dir_all(&paths.app_config_dir).unwrap();
    let body = serde_json::json!({
        "providers": {
            "claude": {
                "authMode": "apiKey",
                "apiKey": key,
            }
        }
    });
    std::fs::write(
        paths.app_config_dir.join("settings.json"),
        serde_json::to_string(&body).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn step_one_an_explicit_api_key_wins_and_is_sent_as_x_api_key() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    write_explicit_api_key_settings(&paths, "sk-explicit-key");
    let (http, http_calls) = FakeProviderHttp::ok(MODELS_BODY);
    let lookup_calls = Arc::new(AtomicUsize::new(0));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .with_credential_lookup(not_found_lookup(lookup_calls.clone()))
        .with_provider_http(http.clone())
        .open()
        .await
        .unwrap();

    let models = runtime.providers().claude_model_windows().await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "claude-sonnet-5");
    assert_eq!(http_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        lookup_calls.load(Ordering::SeqCst),
        0,
        "an explicit key must short-circuit before the adapter is ever consulted"
    );
    let headers = http.captured_headers.lock().unwrap().clone();
    assert!(headers.contains(&("x-api-key".to_string(), "sk-explicit-key".to_string())));

    runtime.shutdown().await;
}

#[tokio::test]
async fn step_two_the_credential_lookup_adapter_supplies_an_oauth_bearer_token() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    let (http, http_calls) = FakeProviderHttp::ok(MODELS_BODY);
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup = Arc::new(FakeCredentialLookup {
        calls: calls.clone(),
        result: || {
            Ok(CredentialLookupResult::Found(CredentialValue::new(
                "adapter-token",
            )))
        },
    });

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .with_credential_lookup(lookup)
        .with_provider_http(http.clone())
        .open()
        .await
        .unwrap();

    let models = runtime.providers().claude_model_windows().await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(http_calls.load(Ordering::SeqCst), 1);
    let headers = http.captured_headers.lock().unwrap().clone();
    assert!(headers.contains(&(
        "authorization".to_string(),
        "Bearer adapter-token".to_string()
    )));

    runtime.shutdown().await;
}

#[tokio::test]
async fn not_found_falls_through_to_the_credentials_file() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.claude_config_dir).unwrap();
    std::fs::write(
        paths.claude_config_dir.join(".credentials.json"),
        credentials_json("file-token"),
    )
    .unwrap();
    let (http, http_calls) = FakeProviderHttp::ok(MODELS_BODY);
    let calls = Arc::new(AtomicUsize::new(0));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .with_credential_lookup(not_found_lookup(calls.clone()))
        .with_provider_http(http.clone())
        .open()
        .await
        .unwrap();

    let models = runtime.providers().claude_model_windows().await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(http_calls.load(Ordering::SeqCst), 1);
    let headers = http.captured_headers.lock().unwrap().clone();
    assert!(headers.contains(&("authorization".to_string(), "Bearer file-token".to_string())));

    runtime.shutdown().await;
}

#[tokio::test]
async fn no_source_produces_a_credential_missing_error_and_never_calls_http() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    let (http, http_calls) = FakeProviderHttp::ok(MODELS_BODY);
    let calls = Arc::new(AtomicUsize::new(0));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .with_credential_lookup(not_found_lookup(calls.clone()))
        .with_provider_http(http)
        .open()
        .await
        .unwrap();

    let err = runtime
        .providers()
        .claude_model_windows()
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        CoreError::ProviderCredentialsMissing { ref provider } if provider == "Claude"
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(http_calls.load(Ordering::SeqCst), 0);

    runtime.shutdown().await;
}

#[tokio::test]
async fn an_adapter_error_short_circuits_and_never_falls_through() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.claude_config_dir).unwrap();
    std::fs::write(
        paths.claude_config_dir.join(".credentials.json"),
        credentials_json("should-never-be-read"),
    )
    .unwrap();
    let (http, http_calls) = FakeProviderHttp::ok(MODELS_BODY);
    let calls = Arc::new(AtomicUsize::new(0));
    let lookup = Arc::new(FakeCredentialLookup {
        calls: calls.clone(),
        result: || Err(CredentialLookupError("keychain is locked".into())),
    });

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .with_credential_lookup(lookup)
        .with_provider_http(http)
        .open()
        .await
        .unwrap();

    let err = runtime
        .providers()
        .claude_model_windows()
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::CredentialLookup { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        http_calls.load(Ordering::SeqCst),
        0,
        "an adapter error must never fall through to a weaker source"
    );

    runtime.shutdown().await;
}

#[test]
fn credential_values_never_leak_through_debug_or_display() {
    let value = CredentialValue::new("sk-super-secret");
    assert_eq!(format!("{value:?}"), "CredentialValue(<redacted>)");
    assert_eq!(format!("{value}"), "<redacted>");
}

#[tokio::test]
async fn the_credential_and_http_adapters_survive_shutdown_but_are_never_touched_again() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    let (http, http_calls) = FakeProviderHttp::ok(MODELS_BODY);
    let calls = Arc::new(AtomicUsize::new(0));

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths)
        .with_credential_lookup(not_found_lookup(calls.clone()))
        .with_provider_http(http)
        .open()
        .await
        .unwrap();

    runtime.shutdown().await;

    let err = runtime
        .providers()
        .claude_model_windows()
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::RuntimeStopped));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a stopped runtime must short-circuit before touching the credential adapter"
    );
    assert_eq!(http_calls.load(Ordering::SeqCst), 0);
}

// --- Change group 2: the tracked OpenCode migration ledger ---

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/compat/runtime-migrations")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

fn ledger_path(paths: &RuntimePaths) -> PathBuf {
    paths.app_data_dir.join("runtime-migrations.json")
}

#[tokio::test]
async fn open_runs_the_migration_against_the_supplied_opencode_config_only() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(paths.opencode_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.opencode_config_file,
        r#"{"model":"github-copilot/gpt-4"}"#,
    )
    .unwrap();

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await
        .unwrap();

    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&paths.opencode_config_file).unwrap())
            .unwrap();
    assert!(
        config.get("model").is_none(),
        "the retired default must be cleared"
    );

    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ledger_path(&paths)).unwrap()).unwrap();
    assert_eq!(ledger["version"], 1);
    assert!(ledger["completed"]["opencode_remove_github_copilot_default_v1"].is_object());

    runtime.shutdown().await;
}

#[tokio::test]
async fn a_malformed_ledger_fails_open_without_touching_the_config() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    std::fs::write(ledger_path(&paths), fixture("malformed.json")).unwrap();
    std::fs::create_dir_all(paths.opencode_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.opencode_config_file,
        r#"{"model":"github-copilot/gpt-4"}"#,
    )
    .unwrap();

    let result = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await;
    let Err(err) = result else {
        panic!("a malformed ledger must fail open");
    };
    assert!(matches!(
        err,
        CoreError::RuntimeMigrationLedgerInvalid { .. }
    ));

    let config = std::fs::read_to_string(&paths.opencode_config_file).unwrap();
    assert!(
        config.contains("github-copilot"),
        "must not mutate the config"
    );
}

#[tokio::test]
async fn a_too_new_ledger_fails_open_without_touching_the_config() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    std::fs::write(ledger_path(&paths), fixture("version-too-new.json")).unwrap();
    std::fs::create_dir_all(paths.opencode_config_file.parent().unwrap()).unwrap();
    std::fs::write(
        &paths.opencode_config_file,
        r#"{"model":"github-copilot/gpt-4"}"#,
    )
    .unwrap();

    let result = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await;
    let Err(err) = result else {
        panic!("a too-new ledger must fail open");
    };
    assert!(matches!(
        err,
        CoreError::RuntimeMigrationLedgerTooNew {
            found: 2,
            max: 1,
            ..
        }
    ));

    let config = std::fs::read_to_string(&paths.opencode_config_file).unwrap();
    assert!(
        config.contains("github-copilot"),
        "must not mutate the config"
    );
}

#[tokio::test]
async fn unknown_migration_ids_in_the_ledger_are_preserved_across_an_open() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());
    std::fs::create_dir_all(&paths.app_data_dir).unwrap();
    std::fs::write(ledger_path(&paths), fixture("unknown-id.json")).unwrap();

    let runtime = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await
        .unwrap();

    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ledger_path(&paths)).unwrap()).unwrap();
    assert_eq!(
        ledger["completed"]["some_future_migration_v7"]["completedAt"],
        1_700_000_000_000.0_f64
    );
    assert!(ledger["completed"]["opencode_remove_github_copilot_default_v1"].is_object());

    runtime.shutdown().await;
}

#[tokio::test]
async fn a_concurrent_open_on_the_same_profile_is_rejected_and_the_ledger_is_untouched() {
    let root = tempfile::tempdir().unwrap();
    let paths = test_paths(root.path());

    let first = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await
        .unwrap();
    let ledger_after_first_open = std::fs::read_to_string(ledger_path(&paths)).unwrap();

    let result = RuntimeBuilder::new(test_environment(root.path()), paths.clone())
        .open()
        .await;
    let Err(err) = result else {
        panic!("a concurrent open on the same profile must be rejected");
    };
    assert!(matches!(err, CoreError::RuntimeInUse { .. }));
    assert_eq!(
        std::fs::read_to_string(ledger_path(&paths)).unwrap(),
        ledger_after_first_open,
        "a rejected second open must not touch the migration ledger at all"
    );

    first.shutdown().await;
}
