//! What a workstream can be changed into after it exists: renamed, moved to a
//! different base, updated from that base, given documents, moved through the
//! stages, and reprovisioned. `workstream_lifecycle` covers create → archive →
//! restore → purge; these are the edits in between.
#![cfg(unix)]

use std::path::Path;

use hyprlayer_core::{Stage, WorktreeMode};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workstream_mutations() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("workstream_mutations", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let mut events_rx = client.take_events().expect("the client's runtime events");
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Original title").await;
    let id = text(&view, "/item/id");
    let branch = text(&view, "/item/branch");
    let worktree = text(&view, "/item/worktreePath");

    j.step("rename it");
    j.call(
        &client,
        Request::RenameWorkstream {
            id: id.clone(),
            title: "Renamed title".into(),
        },
    )
    .await
    .expect("rename_workstream");
    let all = j
        .call(&client, Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let renamed = all
        .as_array()
        .and_then(|ws| ws.iter().find(|w| text(w, "/item/id") == id))
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "the new title is what is listed",
        text(&renamed, "/item/title") == "Renamed title",
        json!({ "title": renamed.pointer("/item/title") }),
    );
    j.guard(
        "renaming leaves the branch alone",
        text(&renamed, "/item/branch") == branch,
        json!({ "branch_unchanged": text(&renamed, "/item/branch") == branch }),
    );

    j.step("move it to another base");
    let bases = j
        .call(
            &client,
            Request::ListWorkstreamBases {
                project_path: project.clone(),
            },
        )
        .await
        .expect("list_workstream_bases");
    j.guard(
        "the project offers its branches as bases",
        serde_json::to_string(&bases)
            .unwrap_or_default()
            .contains("main"),
        json!({ "has_main": serde_json::to_string(&bases).unwrap_or_default().contains("main") }),
    );
    // A second base to move onto, committed on the source repo so the worktree's
    // remote knows it.
    sandbox.git(&repo, &["branch", "release"]);
    j.call(
        &client,
        Request::SetWorkstreamBaseBranch {
            id: id.clone(),
            base: "release".into(),
        },
    )
    .await
    .expect("set_workstream_base_branch");
    let after_base = j
        .call(&client, Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let moved = after_base
        .as_array()
        .and_then(|ws| ws.iter().find(|w| text(w, "/item/id") == id))
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "the recorded base follows the change",
        text(&moved, "/item/baseBranch") == "release",
        json!({ "base": moved.pointer("/item/baseBranch") }),
    );
    let pulled = j
        .call(
            &client,
            Request::UpdateWorkstreamFromBase { id: id.clone() },
        )
        .await;
    let evidence = match &pulled {
        Ok(v) => json!({ "outcome": v }),
        Err(e) => json!({ "code": e.code }),
    };
    j.guard(
        "updating from an unchanged base reports rather than fails",
        pulled.is_ok(),
        evidence,
    );

    j.step("attach a document");
    let doc = sandbox.home.join("thoughts").join("notes.md");
    std::fs::create_dir_all(doc.parent().expect("thoughts dir")).ok();
    std::fs::write(&doc, "# notes\n").expect("write a doc");
    let attached = j
        .call(
            &client,
            Request::AttachWorkstreamDoc {
                id: id.clone(),
                doc_path: doc.display().to_string(),
                role: "plan".into(),
            },
        )
        .await
        .expect("attach_workstream_doc");
    j.guard(
        "attaching records the document against the workstream",
        roles(&attached).contains(&"plan".to_string()),
        json!({ "roles": roles(&attached) }),
    );
    let listed_docs = j
        .call(&client, Request::ListWorkstreamDocs { id: id.clone() })
        .await
        .expect("list_workstream_docs");
    j.guard(
        "the attachment is listed back",
        listed_docs.as_array().map(|d| d.len()).unwrap_or(0) == 1,
        json!({ "docs": listed_docs.as_array().map(|d| d.len()) }),
    );

    j.step("move through the stages");
    j.call(
        &client,
        Request::SetWorkstreamStage {
            id: id.clone(),
            stage: Stage::Implement,
        },
    )
    .await
    .expect("set_workstream_stage");
    let staged = j
        .call(&client, Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let at_stage = staged
        .as_array()
        .and_then(|ws| ws.iter().find(|w| text(w, "/item/id") == id))
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "the stage is recorded",
        text(&at_stage, "/item/stage") == "implement",
        json!({ "stage": at_stage.pointer("/item/stage") }),
    );
    let context = j
        .call(&client, Request::WorkstreamPromptContext { id: id.clone() })
        .await
        .expect("workstream_prompt_context");
    j.guard(
        "the prompt context resolves for the workstream",
        !context.is_null(),
        json!({ "keys": context.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()) }),
    );

    j.step("reprovision the thoughts links");
    // Reprovision repairs the links inside a worktree, not the worktree itself.
    let link = Path::new(&worktree).join("thoughts");
    let had_link = link.exists();
    if link.is_dir() && !link.is_symlink() {
        std::fs::remove_dir_all(&link).expect("remove the thoughts link");
    } else if link.exists() {
        std::fs::remove_file(&link).expect("remove the thoughts link");
    }
    let reprovisioned = j
        .call(&client, Request::ReprovisionWorkstream { id: id.clone() })
        .await
        .expect("reprovision_workstream");
    j.guard(
        "reprovisioning restores the thoughts link",
        had_link && link.exists(),
        json!({ "had_link": had_link, "restored": link.exists() }),
    );
    j.guard(
        "it keeps the branch it was on",
        text(&reprovisioned, "/item/branch") == branch,
        json!({ "branch_kept": text(&reprovisioned, "/item/branch") == branch }),
    );
    j.step("adopt a branch changed behind the app's back");
    // `follows_head` is Inplace/Existing only: an isolated worktree owns its
    // branch, so a checkout inside it must NOT be adopted. Both halves matter.
    sandbox.git(
        Path::new(&worktree),
        &["checkout", "--quiet", "-b", "not-adopted"],
    );
    let listed_isolated = j
        .call(&client, Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let isolated_branch = listed_isolated
        .as_array()
        .and_then(|ws| ws.iter().find(|w| text(w, "/item/id") == id))
        .map(|w| text(w, "/item/branch"))
        .unwrap_or_default();
    j.guard(
        "an isolated worktree does not follow a checkout made inside it",
        isolated_branch == branch,
        json!({ "still_on": isolated_branch == branch }),
    );

    let inplace = j
        .call(
            &client,
            Request::CreateWorkstream {
                project_path: project.clone(),
                title: "Inplace work".into(),
                base: None,
                mode: WorktreeMode::Inplace,
                // In-place adopts the repo as it stands, so the branch must
                // already exist — `release` was created above.
                branch: Some("release".into()),
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream inplace");
    let inplace_id = text(&inplace, "/item/id");
    let inplace_branch = text(&inplace, "/item/branch");
    let inplace_tree = text(&inplace, "/item/worktreePath");
    // Only `inplace_id` varies per run; `inplace_branch` is the fixed "release"
    // and aliasing it would rewrite every earlier mention of that branch.
    j.alias(&inplace_id, "$INPLACE");
    sandbox.git(
        Path::new(&inplace_tree),
        &["checkout", "--quiet", "-b", "adopted"],
    );
    j.call(&client, Request::ListAllWorkstreams {})
        .await
        .expect("list_all_workstreams");
    let adopted = events_until(&mut events_rx, std::time::Duration::from_secs(20), |e| {
        matches!(e, RuntimeEvent::WorkstreamBranchChanged { workstream_id, .. } if *workstream_id == inplace_id)
    })
    .await;
    let change = adopted.iter().find_map(|e| match e {
        RuntimeEvent::WorkstreamBranchChanged {
            branch, previous, ..
        } => Some((branch.clone(), previous.clone())),
        _ => None,
    });
    j.guard(
        "an in-place worktree adopts an outside checkout and announces it",
        change
            .as_ref()
            .is_some_and(|(b, p)| b == "adopted" && *p == inplace_branch),
        json!({
            "adopted": change.as_ref().map(|(b, _)| b.clone()),
            "was_previous_branch": change.as_ref().is_some_and(|(_, p)| *p == inplace_branch),
        }),
    );

    // The guard that stops it resurrecting a worktree as a bare symlink farm
    // with no .git, which would block a later restore.
    std::fs::remove_dir_all(Path::new(&worktree)).expect("remove the worktree");
    let refused = j
        .call(&client, Request::ReprovisionWorkstream { id: id.clone() })
        .await;
    let evidence = match &refused {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "a missing worktree is refused, not rebuilt",
        refused.is_err() && !Path::new(&worktree).exists(),
        evidence,
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

fn roles(docs: &Value) -> Vec<String> {
    docs.as_array()
        .map(|items| items.iter().map(|d| text(d, "/role")).collect())
        .unwrap_or_default()
}
