use std::path::Path;

use hyprlayer_app_lib::error::AppError;
use hyprlayer_core::config::ConfigService;
use hyprlayer_core::error::CoreError;
use hyprlayer_core::scripts::ScriptKind;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct Pinned {
    code: String,
    message: String,
    remedy: Option<String>,
}

impl From<AppError> for Pinned {
    fn from(err: AppError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            remedy: err.remedy,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Fixture {
    #[serde(rename = "settingsConfig")]
    settings_config: Pinned,
    binaries: Pinned,
    #[serde(rename = "providersSessions")]
    providers_sessions: Pinned,
    projects: Pinned,
    library: Pinned,
    #[serde(rename = "agentsPrompts")]
    agents_prompts: Pinned,
    workstreams: Pinned,
    attachments: Pinned,
    terminals: Pinned,
    scripts: Pinned,
    #[serde(rename = "profileLock")]
    profile_lock: Pinned,
    #[serde(rename = "configMigration")]
    config_migration: Pinned,
    #[serde(rename = "providerAuth")]
    provider_auth: Pinned,
    #[serde(rename = "credentialLookup")]
    credential_lookup: Pinned,
    #[serde(rename = "migrationLedgerInvalid")]
    migration_ledger_invalid: Pinned,
    #[serde(rename = "migrationLedgerTooNew")]
    migration_ledger_too_new: Pinned,
    #[serde(rename = "projectProvisioning")]
    project_provisioning: Pinned,
    #[serde(rename = "libraryDocuments")]
    library_documents: Pinned,
    #[serde(rename = "agentBundle")]
    agent_bundle: Pinned,
    #[serde(rename = "workstreamLifecycle")]
    workstream_lifecycle: Pinned,
    #[serde(rename = "attachmentTransfer")]
    attachment_transfer: Pinned,
}

fn fixture() -> Fixture {
    let raw = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compat/v0_7/app-errors.json"),
    )
    .expect("app-errors.json fixture is readable");
    serde_json::from_str(&raw).expect("app-errors.json is valid JSON")
}

// commands/mod.rs::effective_for loads the hyprlayer config through ConfigService;
// a missing config file is the real, currently-reachable failure every unconfigured
// profile hits before a project is added.
async fn settings_config_failure() -> AppError {
    let path =
        Path::new("/nonexistent-hyprlayer-config-fixture/hyprlayer/config.json").to_path_buf();
    let service = ConfigService::new(path);
    let err = service
        .load()
        .await
        .expect_err("config file does not exist");
    AppError::from(err)
}

// process/binaries.rs's BinaryResolver::require returns this for real once resolution
// fails; constructed directly here so the assertion does not depend on whether claude
// happens to be on the machine running the test.
fn binaries_failure() -> AppError {
    AppError::from(CoreError::BinaryMissing {
        name: "claude".into(),
    })
}

fn providers_sessions_failure() -> AppError {
    hyprlayer_app_lib::commands::sessions::session_not_found("sess-fixture")
}

// error.rs::AppError::project_not_found, the real public constructor.
fn projects_failure() -> AppError {
    AppError::project_not_found("/tmp/example-project")
}

// runtime::library::LibraryService::write_and_reindex's real failure once a
// document survives its write but produces no index entry; it keeps the generic
// `internal` code the v0.7 envelope pinned before the transaction moved to core.
fn library_failure() -> AppError {
    AppError::from(CoreError::DocNotIndexable)
}

// runtime::prompts's real 4 MiB cap on an import source; constructed directly
// (matching library_documents_failure's own reasoning) now that the check runs
// inside PromptService::preview_import rather than a raw file-read command.
fn agents_prompts_failure() -> AppError {
    AppError::from(CoreError::PromptsFileTooLarge)
}

// commands/workstreams.rs:709, a static AppError::internal call.
fn workstreams_failure() -> AppError {
    AppError::internal("base branch cannot be empty")
}

// runtime::attachments's real workstream lookup guard, now shared with every
// other service that resolves a workstream id (phase 6b).
fn attachments_failure() -> AppError {
    AppError::from(CoreError::WorkstreamNotFound {
        id: "ws-fixture".into(),
    })
}

// runtime::terminals's real empty-command guard on OpenAgentTerminalRequest,
// now a typed CoreError variant that keeps the legacy `internal` code (phase 6c).
fn terminals_failure() -> AppError {
    AppError::from(CoreError::TerminalCommandEmpty)
}

fn scripts_failure() -> AppError {
    hyprlayer_app_lib::commands::scripts::script_empty(ScriptKind::Run)
}

// runtime::HyprlayerRuntime::open returns this for real once a second runtime
// opens the same profile; constructed directly here (matching binaries_failure's
// own reasoning) so the pinned message does not depend on an unpredictable
// tempdir path.
fn profile_lock_failure() -> AppError {
    AppError::from(CoreError::RuntimeInUse {
        path: Path::new("/tmp/example-profile").to_path_buf(),
    })
}

// config/mod.rs::parse_config's real schema-version check; the runtime's own
// migration nudge (`HyprlayerRuntime::migrate_config_if_needed`) hits this
// exact CoreError before it ever reaches the ConfigCli port.
async fn config_migration_failure() -> AppError {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hyprlayer-config.json");
    std::fs::write(&path, r#"{"version": 2}"#).expect("write stale config");
    let service = ConfigService::new(path);
    let err = service
        .load()
        .await
        .expect_err("a v2 config needs migration to v4");
    AppError::from(err)
}

#[tokio::test]
async fn settings_config_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(settings_config_failure().await),
        fixture().settings_config
    );
}

#[test]
fn binaries_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(binaries_failure()), fixture().binaries);
}

#[test]
fn providers_sessions_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(providers_sessions_failure()),
        fixture().providers_sessions
    );
}

#[test]
fn projects_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(projects_failure()), fixture().projects);
}

#[test]
fn library_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(library_failure()), fixture().library);
}

#[test]
fn agents_prompts_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(agents_prompts_failure()),
        fixture().agents_prompts
    );
}

#[test]
fn workstreams_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(workstreams_failure()), fixture().workstreams);
}

#[test]
fn attachments_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(attachments_failure()), fixture().attachments);
}

#[test]
fn terminals_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(terminals_failure()), fixture().terminals);
}

#[test]
fn scripts_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(scripts_failure()), fixture().scripts);
}

#[test]
fn profile_lock_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(profile_lock_failure()), fixture().profile_lock);
}

#[tokio::test]
async fn config_migration_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(config_migration_failure().await),
        fixture().config_migration
    );
}

// runtime::providers::ProviderService::claude_model_windows's real failure once
// no credential source (explicit key, adapter, or credentials file) produces one.
fn provider_auth_failure() -> AppError {
    AppError::from(CoreError::ProviderCredentialsMissing {
        provider: "Claude".into(),
    })
}

// runtime::providers's real failure once the `CredentialLookup` adapter itself
// errors rather than reporting `NotFound` — constructed directly (matching
// binaries_failure's own reasoning) so the message does not depend on a real
// platform credential store being present or locked.
fn credential_lookup_failure() -> AppError {
    AppError::from(CoreError::CredentialLookup {
        detail: "keychain is locked".into(),
    })
}

// runtime::migrations's real failure once runtime-migrations.json fails to
// parse; constructed directly so the pinned message does not depend on an
// unpredictable tempdir path (matching profile_lock_failure's reasoning).
fn migration_ledger_invalid_failure() -> AppError {
    AppError::from(CoreError::RuntimeMigrationLedgerInvalid {
        path: Path::new("/tmp/example-profile/runtime-migrations.json").to_path_buf(),
        detail: "missing or non-numeric \"version\"".into(),
    })
}

// runtime::migrations's real failure once runtime-migrations.json is a
// version newer than this build supports.
fn migration_ledger_too_new_failure() -> AppError {
    AppError::from(CoreError::RuntimeMigrationLedgerTooNew {
        path: Path::new("/tmp/example-profile/runtime-migrations.json").to_path_buf(),
        found: 2,
        max: 1,
    })
}

#[test]
fn provider_auth_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(provider_auth_failure()),
        fixture().provider_auth
    );
}

#[test]
fn credential_lookup_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(credential_lookup_failure()),
        fixture().credential_lookup
    );
}

#[test]
fn migration_ledger_invalid_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(migration_ledger_invalid_failure()),
        fixture().migration_ledger_invalid
    );
}

#[test]
fn migration_ledger_too_new_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(migration_ledger_too_new_failure()),
        fixture().migration_ledger_too_new
    );
}

// runtime::projects's real failure once a clone/create destination is claimed
// by something else; representative of the project-provisioning error family
// that moved from ad-hoc AppError literals to typed CoreError variants in
// runtime::projects (phase 5a).
fn project_provisioning_failure() -> AppError {
    AppError::from(CoreError::RepoAlreadyExists {
        path: Path::new("/tmp/example-repo").to_path_buf(),
        offer_remove: true,
    })
}

#[test]
fn project_provisioning_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(project_provisioning_failure()),
        fixture().project_provisioning
    );
}

// runtime::library's real optimistic-concurrency failure once a document is
// edited outside the app between load and save; representative of the document
// error family that moved from ad-hoc AppError literals to typed CoreError
// variants in runtime::library (phase 5b).
fn library_documents_failure() -> AppError {
    AppError::from(CoreError::DocConflict)
}

#[test]
fn library_documents_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(library_documents_failure()),
        fixture().library_documents
    );
}

// runtime::agents's real gate on a CLI that has not provisioned both harnesses;
// representative of the agent-bundle error family that moved from ad-hoc
// AppError literals to typed CoreError variants in runtime::agents (phase 5c).
fn agent_bundle_failure() -> AppError {
    AppError::from(CoreError::AgentBundleNotManageable)
}

#[test]
fn agent_bundle_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(Pinned::from(agent_bundle_failure()), fixture().agent_bundle);
}

// runtime::workstreams's real one-active-in-place guard, held under the
// per-project operation lock added in this phase; representative of the
// workstream-lifecycle error family that moved from ad-hoc AppError literals
// to typed CoreError variants in runtime::workstreams (phase 6a).
fn workstream_lifecycle_failure() -> AppError {
    AppError::from(CoreError::InplaceWorkstreamExists)
}

#[test]
fn workstream_lifecycle_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(workstream_lifecycle_failure()),
        fixture().workstream_lifecycle
    );
}

// runtime::attachments's real escape-rejection guard on a save/import
// destination name that resolves outside the workstream's attachments
// directory; representative of the attachment error family that moved from
// ad-hoc AppError literals to typed CoreError variants in
// runtime::attachments (phase 6b).
fn attachment_transfer_failure() -> AppError {
    AppError::from(CoreError::InvalidPath {
        path: "../escape.txt".into(),
    })
}

#[test]
fn attachment_transfer_failure_matches_the_pinned_v0_7_fixture() {
    assert_eq!(
        Pinned::from(attachment_transfer_failure()),
        fixture().attachment_transfer
    );
}
