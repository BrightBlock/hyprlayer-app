//! Device-local state a restart must not lose, and migrations a restart must
//! not repeat. ADR-0008 narrows runtime migrations to run exactly once, so the
//! ledger entry standing still across a restart is the property, not a detail.
#![cfg(unix)]

use std::path::Path;

use hyprlayer_core::branch_prefix::BranchPrefix;
use hyprlayer_core::process::binaries::BinaryKind;
use hyprlayer_core::settings::AppSettings;
use hyprlayer_e2e::flows::onboard_and_import;
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_persistence() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("settings_persistence", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let db = sandbox.app_data.join("hyprlayer.db");
    let ledger_path = sandbox.app_data.join("runtime-migrations.json");

    j.step("the profile migrated on first open");
    let ledger = sandbox.read_json(&ledger_path);
    let ran = completed(&ledger);
    j.guard(
        "the runtime-migration ledger records what ran",
        !ran.is_empty(),
        json!({ "completed": ran }),
    );
    let before_stamps = stamps(&ledger);

    j.step("change device-local settings");
    let current = j
        .call(&client, Request::GetAppSettings {})
        .await
        .expect("get_app_settings");
    let mut settings: AppSettings =
        serde_json::from_value(current).expect("decode the current settings");
    settings.dev_tools = !settings.dev_tools;
    let wanted_dev_tools = settings.dev_tools;
    let worktree_root = sandbox.home.join("elsewhere").display().to_string();
    settings.worktree_root = Some(worktree_root.clone());
    let saved = j
        .call(&client, Request::SetAppSettings { settings })
        .await
        .expect("set_app_settings");
    j.guard(
        "the server returns the settings it stored",
        saved["devTools"] == json!(wanted_dev_tools)
            && text(&saved, "/worktreeRoot") == worktree_root,
        json!({ "dev_tools": saved["devTools"], "worktree_root": saved["worktreeRoot"] }),
    );
    let prefixes = vec![BranchPrefix {
        label: "Spike".into(),
        prefix: "spike/".into(),
        keywords: vec!["spike".into()],
    }];
    j.call(
        &client,
        Request::SetBranchPrefixes {
            project_path: project.clone(),
            prefixes,
        },
    )
    .await
    .expect("set_branch_prefixes");

    j.step("override a binary");
    let absent = sandbox.root.join("absent").join("codex");
    let statuses = j
        .call(
            &client,
            Request::SetBinaryOverride {
                kind: BinaryKind::Codex,
                path: Some(sandbox.git.display().to_string()),
            },
        )
        .await
        .expect("set_binary_override");
    j.guard(
        "an override points a binary at a real file",
        state_of(&statuses, "codex") == "ok",
        json!({ "codex": state_of(&statuses, "codex") }),
    );
    let redetected = j
        .call(&client, Request::RedetectBinaries {})
        .await
        .expect("redetect_binaries");
    j.guard(
        "redetection keeps the override rather than rediscovering past it",
        state_of(&redetected, "codex") == "ok",
        json!({ "codex": state_of(&redetected, "codex") }),
    );
    let cleared = j
        .call(
            &client,
            Request::SetBinaryOverride {
                kind: BinaryKind::Codex,
                path: Some(absent.display().to_string()),
            },
        )
        .await
        .expect("set_binary_override");
    j.guard(
        "an override at a missing path reports missing, not found",
        state_of(&cleared, "codex") == "missing",
        json!({ "codex": state_of(&cleared, "codex") }),
    );

    let announced = events_until(&mut events_rx, std::time::Duration::from_secs(10), |e| {
        matches!(e, RuntimeEvent::BinariesChanged)
    })
    .await;
    j.guard(
        "changing a binary override announces it",
        announced
            .iter()
            .any(|e| matches!(e, RuntimeEvent::BinariesChanged)),
        json!({ "binaries_changed": announced.iter().filter(|e| matches!(e, RuntimeEvent::BinariesChanged)).count() }),
    );

    j.step("restart the server");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the first server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code() }),
    );
    // `user_version` lives in the main database header, which a WAL journal does
    // not update until it checkpoints, so this reads 0 until the server that
    // holds the database has stopped.
    let schema = user_version(&db);
    j.guard(
        "the database is at its migrated schema version",
        schema > 0,
        json!({ "user_version": schema }),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();

    j.step("nothing was lost or repeated");
    let after = j
        .call(&client, Request::GetAppSettings {})
        .await
        .expect("get_app_settings");
    j.guard(
        "settings survive the restart",
        after["devTools"] == json!(wanted_dev_tools)
            && text(&after, "/worktreeRoot") == worktree_root,
        json!({ "dev_tools": after["devTools"], "worktree_root": after["worktreeRoot"] }),
    );
    let resolved = j
        .call(
            &client,
            Request::GetBranchPrefixes {
                project_path: project.clone(),
            },
        )
        .await
        .expect("get_branch_prefixes");
    let stored = prefixes_of(&resolved);
    j.guard(
        "branch prefixes survive the restart",
        stored.contains(&"spike/".to_string()),
        json!({ "prefixes": stored }),
    );
    let ledger_after = sandbox.read_json(&ledger_path);
    j.guard(
        "a completed runtime migration is not rerun",
        stamps(&ledger_after) == before_stamps,
        json!({ "unchanged": stamps(&ledger_after) == before_stamps, "completed": completed(&ledger_after) }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );
    j.guard(
        "the schema is not migrated a second time",
        user_version(&db) == schema,
        json!({ "user_version": user_version(&db) }),
    );

    j.finish();
}

/// SQLite keeps `user_version` as a big-endian u32 at byte 60 of the database
/// header, so the schema version reads without a `sqlite3` binary on the host.
fn user_version(db: &Path) -> u32 {
    let bytes = std::fs::read(db).unwrap_or_default();
    bytes
        .get(60..64)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(0)
}

/// One binary's state. `BinaryState` is internally tagged with `tag = "state"`,
/// so it nests one level below the field of the same name.
fn state_of(statuses: &Value, name: &str) -> String {
    statuses
        .as_array()
        .and_then(|all| all.iter().find(|s| text(s, "/name") == name))
        .map(|s| text(s, "/state/state"))
        .unwrap_or_default()
}

fn prefixes_of(resolved: &Value) -> Vec<String> {
    resolved
        .pointer("/prefixes")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|p| text(p, "/prefix")).collect())
        .unwrap_or_default()
}

fn completed(ledger: &Value) -> Vec<String> {
    ledger
        .pointer("/completed")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// Each migration's completion timestamp, which a rerun would move.
fn stamps(ledger: &Value) -> Vec<(String, String)> {
    ledger
        .pointer("/completed")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v["completedAt"].to_string()))
                .collect()
        })
        .unwrap_or_default()
}
