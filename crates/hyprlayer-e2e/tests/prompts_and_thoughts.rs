//! Exporting and re-importing the prompt library, committing the thoughts
//! repository through the CLI, and reaping archived worktrees. What these
//! share is a round trip: whatever comes out has to go back in.
#![cfg(unix)]

use hyprlayer_core::runtime::prompts::{
    PromptExportEntry, PromptExportRequest, PromptExportSelection, PromptImportDecision,
    PromptImportMode,
};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompts_and_thoughts() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("prompts_and_thoughts", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("export the prompt library");
    let exported = j
        .call(
            &client,
            Request::ExportPrompts {
                request: PromptExportRequest {
                    selection: PromptExportSelection {
                        // The caller supplies what to export; a fresh profile's
                        // library is empty, and exporting nothing produces a
                        // document the importer rightly refuses.
                        entries: vec![PromptExportEntry {
                            name: "E2E probe".into(),
                            body: "Probe body".into(),
                            chord: Some("mod+shift+e".into()),
                            overrides: Default::default(),
                        }],
                        include_chords: true,
                        include_overrides: true,
                        source: None,
                    },
                },
            },
        )
        .await
        .expect("export_prompts");
    let bundle = text(&exported, "/content");
    j.guard(
        "exporting produces a document carrying the prompt",
        bundle.contains("E2E probe") && exported.pointer("/prompt_count") == Some(&json!(1)),
        json!({ "prompt_count": exported.pointer("/prompt_count") }),
    );

    j.step("preview importing it back");
    let preview = j
        .call(
            &client,
            Request::PreviewPromptImport {
                source: bundle.clone(),
                taken_chords: vec![],
            },
        )
        .await
        .expect("preview_prompt_import");
    let sha = text(&preview, "/source_sha256");
    j.alias(&sha, "$SOURCE_SHA");
    j.guard(
        "the preview fingerprints the source it was handed",
        !sha.is_empty(),
        json!({
            "version": preview.pointer("/source_version"),
            "candidates": preview.pointer("/prompts").and_then(Value::as_array).map(|p| p.len()),
        }),
    );

    j.step("apply it, and refuse a tampered source");
    // The decision carries the fingerprint the preview returned; the client
    // holds the file, so a mismatch is the only way to catch a swap.
    let tampered = j
        .call(
            &client,
            Request::ApplyPromptImport {
                decision: PromptImportDecision {
                    source: format!("{bundle}\n"),
                    expected_source_sha256: sha.clone(),
                    mode: PromptImportMode::Merge,
                    conflict_actions: Default::default(),
                    taken_chords: vec![],
                },
            },
        )
        .await;
    let evidence = match &tampered {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "a source that no longer matches its fingerprint is refused",
        tampered.is_err(),
        evidence,
    );
    let applied = j
        .call(
            &client,
            Request::ApplyPromptImport {
                decision: PromptImportDecision {
                    source: bundle.clone(),
                    expected_source_sha256: sha,
                    mode: PromptImportMode::Merge,
                    conflict_actions: Default::default(),
                    taken_chords: vec![],
                },
            },
        )
        .await;
    // Derived counts only: the result carries a freshly minted prompt id.
    let evidence = match &applied {
        Ok(v) => json!({
            "added": v.pointer("/added"),
            "renamed": v.pointer("/renamed"),
            "skipped": v.pointer("/skipped"),
        }),
        Err(e) => json!({ "code": e.code }),
    };
    j.guard(
        "the untouched export imports back cleanly",
        applied.is_ok(),
        evidence,
    );

    j.step("commit the thoughts repository");
    let synced = j
        .call(
            &client,
            Request::SyncThoughts {
                path: project.clone(),
                message: Some("e2e sync".into()),
            },
        )
        .await;
    let evidence = match &synced {
        Ok(v) => json!({ "result": v }),
        Err(e) => json!({ "code": e.code }),
    };
    j.guard(
        "syncing the thoughts repo reports an outcome rather than failing",
        synced.is_ok(),
        evidence,
    );

    j.step("reap an archived worktree");
    let view = create_workstream(&mut j, &client, &project, "Reapable").await;
    let id = text(&view, "/item/id");
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: id.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let untouched = j
        .call(
            &client,
            Request::CleanupArchivedWorkstreams {
                before_ms: Some(0.0),
            },
        )
        .await
        .expect("cleanup_archived_workstreams");
    j.guard(
        "a cutoff before every archive deletes nothing",
        untouched.pointer("/deleted").and_then(Value::as_u64) == Some(0),
        json!({ "report": untouched }),
    );
    let reaped = j
        .call(
            &client,
            Request::CleanupArchivedWorkstreams { before_ms: None },
        )
        .await
        .expect("cleanup_archived_workstreams");
    j.guard(
        "no cutoff reaps the archive without failures",
        reaped.pointer("/failed").and_then(Value::as_u64) == Some(0),
        json!({ "report": reaped }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}
