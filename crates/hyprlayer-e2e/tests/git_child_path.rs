//! A server started with launchd's minimal `PATH`, as a Dock-launched app is,
//! still hands git the augmented one: a required filter that, like git-lfs,
//! lives only in a user install dir runs at checkout.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use hyprlayer_core::WorktreeMode;
use hyprlayer_e2e::flows::onboard_and_import;
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server};

const SMUDGE: &str = "#!/bin/sh\nsed 's/^pointer$/smudged/'\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_child_path() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo(
        "demo",
        &[
            ("README.md", "# demo\n"),
            (".gitattributes", "*.dat filter=probe\n"),
            ("asset.dat", "pointer\n"),
        ],
    );
    // After the commit, so it stores the raw blob. `required` makes a missing
    // filter fatal, as git-lfs does; a required filter also needs a `clean`.
    for (key, value) in [
        ("filter.probe.smudge", "hl-probe-smudge"),
        ("filter.probe.clean", "cat"),
        ("filter.probe.required", "true"),
    ] {
        sandbox.git(&repo, &["config", key, value]);
    }
    // A resolver well-known dir, not on the server's own PATH.
    let tools = sandbox.home.join(".local").join("bin");
    std::fs::create_dir_all(&tools).expect("create ~/.local/bin");
    let smudge = tools.join("hl-probe-smudge");
    std::fs::write(&smudge, SMUDGE).expect("write the filter");
    std::fs::set_permissions(&smudge, std::fs::Permissions::from_mode(0o755))
        .expect("make the filter executable");

    let mut j = Journey::new("git_child_path", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("a worktree checkout runs the filter");
    let created = j
        .call(
            &client,
            Request::CreateWorkstream {
                project_path: project.clone(),
                title: "Filtered checkout".into(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await;
    if let Ok(view) = &created {
        j.alias(text(view, "/item/id"), "$WS");
        let slug = text(view, "/item/slug");
        if let Some((stem, _)) = slug.rsplit_once('-') {
            j.alias(&slug, format!("{stem}-$SUFFIX"));
        }
    }
    j.guard(
        "creating the workstream finds the filter git runs",
        created.is_ok(),
        json!({ "error": created.as_ref().err().map(|e| e.message.clone()) }),
    );
    let asset = created.as_ref().ok().and_then(|view| {
        sandbox.read(&Path::new(&text(view, "/item/worktreePath")).join("asset.dat"))
    });
    j.guard(
        "the worktree holds the filter's output, not the stored blob",
        asset.as_deref() == Some("smudged\n"),
        json!({ "asset.dat": asset }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success(),
        json!({ "exit": stopped.status.code() }),
    );

    j.finish();
}
