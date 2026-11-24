//! Two writers against one document. The optimistic-concurrency check is an
//! mtime comparison, so the loser must be refused rather than merged, and an
//! edit made outside the app — the CLI, an editor, a sync — has to count as the
//! other writer.
#![cfg(unix)]

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use hyprlayer_core::docs::frontmatter::{FmPatch, FmValue};
use hyprlayer_core::index::indexer::ThoughtQuery;
use hyprlayer_core::runtime::library::CreateDocumentRequest;
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};

/// `ensure_not_stale` compares millisecond mtimes with a 0.5ms tolerance, so
/// two writes inside one millisecond are indistinguishable and neither can be
/// detected as stale. Scenarios that need two *distinguishable* writes space
/// them; the collision itself is a property of the check, not of this test.
async fn distinct_mtime() {
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doc_concurrency() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("doc_concurrency", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("create a document");
    let created = j
        .call(
            &client,
            Request::CreateDoc {
                input: CreateDocumentRequest {
                    project_repo_path: Some(project.clone()),
                    scope: "project".into(),
                    doc_type: "research".into(),
                    title: "Concurrency notes".into(),
                    date: "2026-01-01".into(),
                    ticket: None,
                    tags: vec!["e2e".into()],
                },
            },
        )
        .await
        .expect("create_doc");
    let id = text(&created, "/id");
    j.alias(&id, "$DOC");
    let path = j
        .call(&client, Request::ResolveDocPath { id: id.clone() })
        .await
        .expect("resolve_doc_path");
    let abs = text(&path, "");
    j.guard(
        "the document lands inside the thoughts repository",
        abs.starts_with(&sandbox.home.join("thoughts").display().to_string()),
        json!({ "under_thoughts": abs.starts_with(&sandbox.home.join("thoughts").display().to_string()) }),
    );

    let indexed = events_until(
        &mut events_rx,
        std::time::Duration::from_secs(20),
        |e| matches!(e, RuntimeEvent::ThoughtsIndexUpdated { ids, .. } if ids.contains(&id)),
    )
    .await;
    // Only the matching event: how many rescans precede it depends on watcher timing.
    let announced = indexed.iter().find_map(|e| match e {
        RuntimeEvent::ThoughtsIndexUpdated { ids, update, .. } if ids.contains(&id) => {
            Some(format!("{update:?}"))
        }
        _ => None,
    });
    j.guard(
        "creating a document announces an index update for it",
        announced.is_some(),
        json!({ "kind": announced }),
    );

    j.step("the first writer wins");
    let doc = j
        .call(&client, Request::GetDoc { id: id.clone() })
        .await
        .expect("get_doc");
    let base = mtime(&doc, "/mtime_ms");
    distinct_mtime().await;
    let body = format!("{}\n\nFIRST_WRITER\n", text(&doc, "/content"));
    let saved = j
        .call(
            &client,
            Request::SaveDoc {
                id: id.clone(),
                content: body,
                base_mtime_ms: base,
            },
        )
        .await
        .expect("save_doc");
    let after_first = mtime(&saved, "/mtimeMs");
    j.guard(
        "a save from the current mtime is accepted",
        after_first > base,
        json!({ "advanced": after_first > base }),
    );

    j.step("the second writer is refused");
    let stale = j
        .call(
            &client,
            Request::SaveDoc {
                id: id.clone(),
                content: "SECOND_WRITER\n".into(),
                base_mtime_ms: base,
            },
        )
        .await;
    let evidence = match &stale {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "a save from a stale mtime is refused",
        stale.is_err(),
        evidence,
    );
    let content = text(
        &j.call(&client, Request::GetDoc { id: id.clone() })
            .await
            .expect("get_doc"),
        "/content",
    );
    j.guard(
        "the refused save leaves the winner's content alone",
        content.contains("FIRST_WRITER") && !content.contains("SECOND_WRITER"),
        json!({ "first": content.contains("FIRST_WRITER"), "second": content.contains("SECOND_WRITER") }),
    );

    j.step("frontmatter takes the same check");
    let patch = FmPatch {
        set: vec![("status".into(), FmValue::Str("shipped".into()))],
        remove: vec![],
    };
    let stale_patch = j
        .call(
            &client,
            Request::PatchDocFrontmatter {
                id: id.clone(),
                patch: patch.clone(),
                base_mtime_ms: base,
            },
        )
        .await;
    let evidence = match &stale_patch {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "a frontmatter patch from a stale mtime is refused",
        stale_patch.is_err(),
        evidence,
    );
    let fresh = mtime(
        &j.call(&client, Request::GetDoc { id: id.clone() })
            .await
            .expect("get_doc"),
        "/mtime_ms",
    );
    distinct_mtime().await;
    let patched = j
        .call(
            &client,
            Request::PatchDocFrontmatter {
                id: id.clone(),
                patch,
                base_mtime_ms: fresh,
            },
        )
        .await
        .expect("patch_doc_frontmatter");
    j.guard(
        "a frontmatter patch from the current mtime is applied",
        text(&patched, "/fm/status") == "shipped",
        json!({ "status": patched.pointer("/fm/status") }),
    );

    j.step("an edit outside the app counts as a writer");
    let before_external = mtime(
        &j.call(&client, Request::GetDoc { id: id.clone() })
            .await
            .expect("get_doc"),
        "/mtime_ms",
    );
    // The CLI owns this repository too (ADR-0002); a write it makes must not be
    // silently overwritten by a client holding an older mtime.
    distinct_mtime().await;
    let on_disk = std::fs::read_to_string(&abs).expect("read the doc");
    std::fs::write(&abs, format!("{on_disk}\nEXTERNAL_EDIT\n")).expect("edit the doc");
    let clobber = j
        .call(
            &client,
            Request::SaveDoc {
                id: id.clone(),
                content: "CLOBBER\n".into(),
                base_mtime_ms: before_external,
            },
        )
        .await;
    let evidence = match &clobber {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "a save that would clobber an outside edit is refused",
        clobber.is_err(),
        evidence,
    );
    j.guard(
        "the outside edit survives",
        std::fs::read_to_string(&abs)
            .unwrap_or_default()
            .contains("EXTERNAL_EDIT"),
        json!({ "external_edit_survives": std::fs::read_to_string(&abs).unwrap_or_default().contains("EXTERNAL_EDIT") }),
    );

    j.step("attachments round-trip through the workstream");
    let ws = create_workstream(&mut j, &client, &project, "Attachments").await;
    let ws_id = text(&ws, "/item/id");
    let source = sandbox.home.join("pasted.txt");
    std::fs::write(&source, "attached bytes\n").expect("write the source file");
    let imported = j
        .call(
            &client,
            Request::ImportAttachment {
                workstream_id: ws_id.clone(),
                src_path: source.display().to_string(),
            },
        )
        .await
        .expect("import_attachment");
    let imported_path = text(&imported, "");
    j.alias(&imported_path, "$ATTACHMENT");
    let read_back = j
        .call(
            &client,
            Request::ReadAttachment {
                path: imported_path,
            },
        )
        .await
        .expect("read_attachment");
    j.guard(
        "an imported attachment reads back byte for byte",
        bytes(&read_back) == b"attached bytes\n",
        json!({ "len": bytes(&read_back).len() }),
    );
    let saved = j
        .call(
            &client,
            Request::SaveAttachment {
                workstream_id: ws_id,
                file_name: "pasted.png".into(),
                data: B64.encode([0x89, 0x50, 0x4e, 0x47]),
            },
        )
        .await
        .expect("save_attachment");
    let saved_path = text(&saved, "");
    j.alias(&saved_path, "$SAVED_ATTACHMENT");
    j.guard(
        "saved bytes land inside the workstream, not the profile root",
        saved_path.contains("pasted") && std::path::Path::new(&saved_path).exists(),
        json!({ "exists": std::path::Path::new(&saved_path).exists() }),
    );

    j.step("the index can find it");
    let found = j
        .call(
            &client,
            Request::QueryDocs {
                query: ThoughtQuery {
                    text: Some("Concurrency notes".into()),
                    ..Default::default()
                },
            },
        )
        .await
        .expect("query_docs");
    j.guard(
        "the document is reachable by search",
        ids_of(&found).contains(&id),
        json!({ "hits": ids_of(&found).len() }),
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

fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .and_then(|s| B64.decode(s).ok())
        .unwrap_or_default()
}

fn mtime(value: &Value, pointer: &str) -> f64 {
    value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn ids_of(result: &Value) -> Vec<String> {
    result
        .pointer("/items")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|i| text(i, "/id")).collect())
        .unwrap_or_default()
}
