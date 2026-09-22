//! Setup that scenarios share once another scenario has already put it under
//! guard; calls are still recorded in the transcript.

use std::path::Path;

use hyprlayer_core::runtime::projects::ThoughtsSource;
use hyprlayer_core::WorktreeMode;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::rpc::Request;
use serde_json::Value;

use crate::{text, Journey, Sandbox};

/// Onboards a fresh profile the way the desktop's onboarding screen does, then
/// imports `repo`. Returns the registered project path.
pub async fn onboard_and_import(
    j: &mut Journey,
    client: &RpcClient,
    sandbox: &Sandbox,
    repo: &Path,
) -> String {
    let probe = j
        .call(client, Request::ProbeDefaultThoughts {})
        .await
        .expect("probe_default_thoughts");
    j.call(
        client,
        Request::InitThoughtsRepo {
            user: "e2e".into(),
            source: ThoughtsSource::Create {
                path: text(&probe, "/default_path"),
            },
        },
    )
    .await
    .expect("init_thoughts_repo");
    let project = j
        .call(
            client,
            Request::ImportProject {
                path: sandbox.as_given(repo).display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            },
        )
        .await
        .expect("import_project");
    text(&project, "/path")
}

/// An isolated workstream off `main`; its id and slug are aliased in the artifact.
pub async fn create_workstream(
    j: &mut Journey,
    client: &RpcClient,
    project: &str,
    title: &str,
) -> Value {
    let view = j
        .call(
            client,
            Request::CreateWorkstream {
                project_path: project.to_string(),
                title: title.to_string(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let slug = text(&view, "/item/slug");
    j.alias(text(&view, "/item/id"), "$WS");
    if let Some((stem, _)) = slug.rsplit_once('-') {
        j.alias(&slug, format!("{stem}-$SUFFIX"));
    }
    view
}
