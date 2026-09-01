use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::env::RuntimePaths;
use crate::error::{CoreError, Result};

const LEDGER_FILE: &str = "runtime-migrations.json";
const CURRENT_VERSION: u64 = 1;
const OPENCODE_REMOVE_GITHUB_COPILOT_DEFAULT_V1: &str = "opencode_remove_github_copilot_default_v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeMigrationRecord {
    #[serde(rename = "completedAt")]
    pub completed_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Ledger {
    version: u64,
    completed: BTreeMap<String, RuntimeMigrationRecord>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            completed: BTreeMap::new(),
        }
    }
}

fn ledger_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LEDGER_FILE)
}

fn load_ledger(path: &Path) -> Result<Ledger> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
        Err(e) => return Err(e.into()),
    };
    let invalid = |detail: String| CoreError::RuntimeMigrationLedgerInvalid {
        path: path.to_path_buf(),
        detail,
    };
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| invalid(e.to_string()))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| invalid("missing or non-numeric \"version\"".into()))?;
    if version > CURRENT_VERSION {
        return Err(CoreError::RuntimeMigrationLedgerTooNew {
            path: path.to_path_buf(),
            found: version,
            max: CURRENT_VERSION,
        });
    }
    serde_json::from_value(value).map_err(|e| invalid(e.to_string()))
}

fn write_ledger(path: &Path, ledger: &Ledger) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Stamp the version this build actually writes. Echoing back the version read
    // from disk would leave an older ledger claiming its original stamp forever,
    // so a future CURRENT_VERSION bump could never tell an upgraded profile from
    // one written by the newer build.
    let ledger = Ledger {
        version: CURRENT_VERSION,
        completed: ledger.completed.clone(),
    };
    let json = serde_json::to_string_pretty(&ledger).map_err(|e| CoreError::OutputParse {
        program: "runtime migration ledger".into(),
        detail: e.to_string(),
    })?;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Runs every tracked runtime migration against the profile's own paths,
/// under the caller's `.runtime.lock`. A malformed or too-new ledger stops
/// here, before any provider config is touched. Each migration mutates its
/// target config atomically and idempotently before the ledger is updated,
/// so a crash between the two safely reruns the migration on the next open.
///
/// A migration whose own mutation fails is logged and left unrecorded — it
/// retries on the next open — rather than failing `open`. These migrations
/// tidy provider config; an unwritable `opencode.json` must not be the reason
/// the app will not launch, which is the resilience the pre-runtime background
/// scrub had. Only a corrupt ledger, which would make *every* later migration
/// decision unsound, is fatal.
pub(crate) fn apply_pending(paths: &RuntimePaths) -> Result<()> {
    let path = ledger_path(&paths.app_data_dir);
    let mut ledger = load_ledger(&path)?;

    if !ledger
        .completed
        .contains_key(OPENCODE_REMOVE_GITHUB_COPILOT_DEFAULT_V1)
    {
        match crate::opencode::support::opencode_remove_github_copilot_default_v1(
            &paths.opencode_config_file,
        ) {
            Ok(_) => {
                ledger.completed.insert(
                    OPENCODE_REMOVE_GITHUB_COPILOT_DEFAULT_V1.to_string(),
                    RuntimeMigrationRecord {
                        completed_at: now_ms(),
                    },
                );
                write_ledger(&path, &ledger)?;
            }
            Err(error) => tracing::warn!(
                migration = OPENCODE_REMOVE_GITHUB_COPILOT_DEFAULT_V1,
                path = %paths.opencode_config_file.display(),
                "runtime migration did not apply; retrying on the next open: {error}"
            ),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/compat/runtime-migrations")
                .join(name),
        )
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
    }

    #[test]
    fn missing_ledger_is_an_empty_v1_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = load_ledger(&dir.path().join("runtime-migrations.json")).unwrap();
        assert_eq!(ledger.version, 1);
        assert!(ledger.completed.is_empty());
    }

    #[test]
    fn malformed_ledger_is_rejected_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime-migrations.json");
        std::fs::write(&path, fixture("malformed.json")).unwrap();
        let err = load_ledger(&path).unwrap_err();
        assert!(matches!(
            err,
            CoreError::RuntimeMigrationLedgerInvalid { .. }
        ));
    }

    #[test]
    fn newer_version_ledger_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime-migrations.json");
        std::fs::write(&path, fixture("version-too-new.json")).unwrap();
        let err = load_ledger(&path).unwrap_err();
        assert!(matches!(
            err,
            CoreError::RuntimeMigrationLedgerTooNew {
                found: 2,
                max: 1,
                ..
            }
        ));
    }

    #[test]
    fn unknown_migration_ids_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime-migrations.json");
        std::fs::write(&path, fixture("unknown-id.json")).unwrap();
        let ledger = load_ledger(&path).unwrap();
        assert_eq!(
            ledger
                .completed
                .get("some_future_migration_v7")
                .map(|r| r.completed_at),
            Some(1_700_000_000_000.0)
        );

        write_ledger(&path, &ledger).unwrap();
        let reloaded = load_ledger(&path).unwrap();
        assert_eq!(reloaded.completed, ledger.completed);
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

    #[test]
    fn applies_and_records_completion_exactly_once() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(
            &paths.opencode_config_file,
            r#"{"model":"github-copilot/gpt"}"#,
        )
        .unwrap();

        apply_pending(&paths).unwrap();

        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.opencode_config_file).unwrap())
                .unwrap();
        assert!(config.get("model").is_none());

        let ledger = load_ledger(&ledger_path(&paths.app_data_dir)).unwrap();
        assert!(ledger
            .completed
            .contains_key(OPENCODE_REMOVE_GITHUB_COPILOT_DEFAULT_V1));

        // Rerunning is a no-op: already completed, config already clean.
        apply_pending(&paths).unwrap();
        let reloaded = load_ledger(&ledger_path(&paths.app_data_dir)).unwrap();
        assert_eq!(reloaded.completed, ledger.completed);
    }

    #[test]
    fn a_crash_between_config_and_ledger_reruns_the_idempotent_migration() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        // Simulate the config mutation having already landed before a crash.
        std::fs::write(&paths.opencode_config_file, r#"{}"#).unwrap();

        apply_pending(&paths).unwrap();

        let ledger = load_ledger(&ledger_path(&paths.app_data_dir)).unwrap();
        assert!(ledger
            .completed
            .contains_key(OPENCODE_REMOVE_GITHUB_COPILOT_DEFAULT_V1));
    }

    #[test]
    fn mutation_failure_is_not_fatal_and_leaves_the_ledger_untouched() {
        let root = tempfile::tempdir().unwrap();
        let paths = test_paths(root.path());
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        // A directory where the config file is expected: every read fails.
        std::fs::create_dir_all(&paths.opencode_config_file).unwrap();

        apply_pending(&paths).expect("a failed migration must not fail open()");
        assert!(!ledger_path(&paths.app_data_dir).exists());

        // With the obstruction gone the retry lands and records completion.
        std::fs::remove_dir(&paths.opencode_config_file).unwrap();
        std::fs::write(
            &paths.opencode_config_file,
            r#"{"model":"github-copilot/gpt"}"#,
        )
        .unwrap();

        apply_pending(&paths).unwrap();

        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.opencode_config_file).unwrap())
                .unwrap();
        assert!(config.get("model").is_none());
        assert!(load_ledger(&ledger_path(&paths.app_data_dir))
            .unwrap()
            .completed
            .contains_key(OPENCODE_REMOVE_GITHUB_COPILOT_DEFAULT_V1));
    }
}
