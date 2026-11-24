//! Skill roots and installs, end to end. Hermetic: sources are local `file://`
//! repos and skills.sh is a canned in-process HTTP server, so unlike
//! `agent_files`' bundle commands nothing here reaches the network.
//!
//! Every guard is independent of where the CLI puts bundle links under
//! `CLAUDE_CONFIG_DIR`.
#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyprlayer_core::models::{SkillScope, SkillTarget};
use hyprlayer_core::skill_sources::skills_sh::SkillsShView;
use hyprlayer_core::skill_sources::SkillSourceInput;
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{eventually, json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::RpcClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const LINK_BASE: &str = "https://github.com/code/skill-src";
const BOTH: [SkillTarget; 2] = [SkillTarget::Claude, SkillTarget::Agents];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_sources() {
    let mut sandbox = Sandbox::new();
    let claude_config = sandbox.root.join("claude-config");
    sandbox.set_env("CLAUDE_CONFIG_DIR", claude_config.clone());
    sandbox.set_env(
        "HYPRLAYER_SKILLS_GIT_BASE",
        format!("file://{}", sandbox.home.display()),
    );
    let (api_addr, api_requests) = skills_sh_stub().await;
    sandbox.set_env("HYPRLAYER_SKILLS_API_URL", format!("http://{api_addr}"));
    let repo = sandbox.fixture_repo("app", &[("README.md", "# app\n")]);
    let src = skill_source(&sandbox);
    let mut j = Journey::new("skill_sources", &sandbox);
    j.alias(claude_config.display().to_string(), "$CLAUDE_CONFIG_DIR");
    let skill_store = sandbox
        .cli_config()
        .parent()
        .expect("CLI config dir")
        .join("skills");
    j.alias(skill_store.display().to_string(), "$SKILL_STORE");
    let staging = sandbox.app_data.join("skill-staging");
    j.alias(staging.display().to_string(), "$STAGING");
    let head = sandbox.git(&src, &["rev-parse", "HEAD"]);
    j.alias(head.clone(), "$HEAD");
    let global_lock = skill_store.with_file_name("skills.lock");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("read model");
    let store = repo.join(".agents/hyprlayer/skills");
    write_skill(&store.join("sibling"), "sibling");
    for (parent, target) in [
        (".claude/skills", "../../.agents/hyprlayer/skills/sibling"),
        (".agents/skills", "../hyprlayer/skills/sibling"),
    ] {
        let parent = repo.join(parent);
        std::fs::create_dir_all(&parent).expect("create a link parent");
        std::os::unix::fs::symlink(target, parent.join("sibling")).expect("link the sibling");
    }
    write_skill(
        &sandbox.home.join(".agents/skills/agents-only"),
        "agents-only",
    );
    let agents_only_dir = sandbox.home.join(".agents/skills/agents-only");
    std::fs::create_dir_all(agents_only_dir.join("refs")).expect("create refs");
    std::fs::write(agents_only_dir.join("refs/notes.md"), "notes\n").expect("write notes");
    std::fs::write(agents_only_dir.join("data.bin"), [0u8, 1, 2]).expect("write a binary");
    std::fs::write(sandbox.home.join("outside.txt"), "outside\n").expect("write outside");
    std::os::unix::fs::symlink(
        sandbox.home.join("outside.txt"),
        agents_only_dir.join("escape.txt"),
    )
    .expect("link outside the skill");
    write_skill(&claude_config.join("skills/ccd-probe"), "ccd-probe");
    write_skill(
        &sandbox.home.join(".claude/skills/home-probe"),
        "home-probe",
    );

    let files = j
        .call(
            &client,
            Request::ListSkillFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list_skill_files");
    let agent_skills = j
        .call(
            &client,
            Request::AgentSkills {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("agent_skills");

    let sibling = rows_named(&files, "sibling");
    let sibling_row = sibling.first().copied();
    j.guard(
        "a sibling store reached by two links lists once",
        sibling.len() == 1
            && sibling_row.is_some_and(|r| {
                strings(r, "/targets") == ["claude", "agents"]
                    && r.pointer("/installed") == Some(&Value::Bool(true))
                    && text(r, "/source") == "project"
                    && text(r, "/path").ends_with("/.claude/skills/sibling/SKILL.md")
            }),
        json!({
            "rows": sibling.len(),
            "targets": sibling_row.map(|r| strings(r, "/targets")),
            "installed": sibling_row.and_then(|r| r.pointer("/installed")),
            "source": sibling_row.map(|r| text(r, "/source")),
            "path": sibling_row.map(|r| text(r, "/path")),
        }),
    );

    let agents_only = rows_named(&files, "agents-only");
    let agents_only_row = agents_only.first().copied();
    let read = j
        .call(
            &client,
            Request::ReadSkillFile {
                project_path: Some(project.clone()),
                path: agents_only_row
                    .map(|r| text(r, "/path"))
                    .unwrap_or_default(),
            },
        )
        .await;
    j.guard(
        "an .agents-only skill is listed and readable",
        agents_only.len() == 1
            && agents_only_row.is_some_and(|r| {
                strings(r, "/targets") == ["agents"]
                    && text(r, "/source") == "user"
                    && r.pointer("/installed") == Some(&Value::Bool(false))
            })
            && read
                .as_ref()
                .is_ok_and(|v| text(v, "").starts_with("---\nname: agents-only")),
        json!({
            "rows": agents_only.len(),
            "targets": agents_only_row.map(|r| strings(r, "/targets")),
            "source": agents_only_row.map(|r| text(r, "/source")),
            "installed": agents_only_row.and_then(|r| r.pointer("/installed")),
            "path": agents_only_row.map(|r| text(r, "/path")),
            "read_ok": read.is_ok(),
        }),
    );

    let skill_path = agents_only_row
        .map(|r| text(r, "/path"))
        .unwrap_or_default();
    let mut assets = Vec::new();
    for rel in [
        "refs/notes.md",
        "data.bin",
        "escape.txt",
        "../ccd-probe/SKILL.md",
    ] {
        let got = j
            .call(
                &client,
                Request::ReadSkillAsset {
                    project_path: Some(project.clone()),
                    skill_path: skill_path.clone(),
                    rel_path: rel.into(),
                },
            )
            .await;
        assets.push((
            rel,
            got.map(|v| v.pointer("/text").cloned()).map_err(|e| e.code),
        ));
    }
    let outcome = |rel: &str| {
        assets
            .iter()
            .find(|(r, _)| *r == rel)
            .map(|(_, got)| got.clone())
    };
    j.guard(
        "a bundled file reads as text, a binary as none, and nothing outside the folder",
        outcome("refs/notes.md") == Some(Ok(Some(json!("notes\n"))))
            && outcome("data.bin") == Some(Ok(Some(Value::Null)))
            && outcome("escape.txt") == Some(Err("invalid_path".into()))
            && outcome("../ccd-probe/SKILL.md") == Some(Err("invalid_path".into())),
        json!(assets
            .iter()
            .map(|(rel, got)| json!({ "rel": rel, "result": match got {
                Ok(text) => json!({ "text": text }),
                Err(code) => json!({ "error": code }),
            } }))
            .collect::<Vec<_>>()),
    );

    let ccd = rows_named(&files, "ccd-probe");
    let ccd_row = ccd.first().copied();
    let home_in_files = !rows_named(&files, "home-probe").is_empty();
    let home_in_agent_skills = !rows_named(&agent_skills, "home-probe").is_empty();
    let ccd_in_agent_skills = !rows_named(&agent_skills, "ccd-probe").is_empty();
    j.guard(
        "Claude's user root follows CLAUDE_CONFIG_DIR",
        ccd.len() == 1
            && ccd_row.is_some_and(|r| {
                strings(r, "/targets") == ["claude"] && text(r, "/source") == "user"
            })
            && ccd_in_agent_skills
            && !home_in_files
            && !home_in_agent_skills,
        json!({
            "ccd_probe": ccd_row.map(|r| json!({
                "targets": strings(r, "/targets"),
                "source": text(r, "/source"),
                "path": text(r, "/path"),
            })),
            "ccd_probe_in_agent_skills": ccd_in_agent_skills,
            "home_probe_in_files": home_in_files,
            "home_probe_in_agent_skills": home_in_agent_skills,
        }),
    );

    let view = create_workstream(&mut j, &client, &project, "Skill roots").await;
    let commands = j
        .call(
            &client,
            Request::ListSlashCommands {
                id: text(&view, "/item/id"),
            },
        )
        .await
        .expect("list_slash_commands");
    let has_command = |name: &str| !rows_named(&commands, name).is_empty();
    j.guard(
        "slash commands follow CLAUDE_CONFIG_DIR",
        has_command("ccd-probe") && !has_command("home-probe"),
        json!({
            "ccd_probe": has_command("ccd-probe"),
            "home_probe": has_command("home-probe"),
        }),
    );

    j.step("preview from a GitHub link");
    let demo = preview(&mut j, &client, &project, "tree/main/skills/demo")
        .await
        .expect("preview demo");
    j.alias(text(&demo, "/token"), "$TOKEN_DEMO");
    let demo_hash = text(&demo, "/hash");
    let files_of = |v: &Value| -> Vec<String> {
        v.pointer("/files")
            .and_then(Value::as_array)
            .map(|all| all.iter().map(|f| text(f, "/relPath")).collect())
            .unwrap_or_default()
    };
    let run_sh = demo
        .pointer("/files")
        .and_then(Value::as_array)
        .and_then(|all| all.iter().find(|f| text(f, "/relPath") == "scripts/run.sh"));
    j.guard(
        "a GitHub link previews its skill, instructions, files and trust flags",
        text(&demo, "/name") == "demo"
            && text(&demo, "/repo") == "code/skill-src"
            && text(&demo, "/path") == "skills/demo"
            && text(&demo, "/ref") == "main"
            && text(&demo, "/commit") == head
            && text(&demo, "/fetchedFrom")
                == format!("file://{}/code/skill-src", sandbox.home.display())
            && files_of(&demo) == ["SKILL.md", "refs/notes.md", "scripts/run.sh"]
            && run_sh.is_some_and(|f| {
                f.pointer("/executable") == Some(&Value::Bool(true))
                    && f.pointer("/script") == Some(&Value::Bool(true))
            })
            && demo.pointer("/flags")
                == Some(&json!({ "scripts": true, "executables": true, "bashAllowed": true }))
            && demo.pointer("/conflicts") == Some(&json!([]))
            && text(&demo, "/instructions").trim() == "Demo body.",
        json!({
            "name": text(&demo, "/name"),
            "instructions": text(&demo, "/instructions"),
            "repo": text(&demo, "/repo"),
            "path": text(&demo, "/path"),
            "ref": text(&demo, "/ref"),
            "commit_is_head": text(&demo, "/commit") == head,
            "fetchedFrom": text(&demo, "/fetchedFrom"),
            "files": demo.pointer("/files"),
            "flags": demo.pointer("/flags"),
            "conflicts": demo.pointer("/conflicts"),
            "hash": demo_hash,
        }),
    );
    let filtered = eventually(Duration::from_secs(5), || {
        sandbox.server_log().contains("--filter=blob:none")
    })
    .await;
    j.guard(
        "the filtered fetch path ran",
        filtered,
        json!({ "log_has_filter": filtered }),
    );

    j.step("every link form resolves the same skill");
    let mut forms = Vec::new();
    for (label, url, want_ref) in [
        (
            "blob",
            format!("{LINK_BASE}/blob/main/skills/demo/SKILL.md"),
            "main",
        ),
        (
            "shorthand",
            "code/skill-src@main/skills/demo".to_string(),
            "main",
        ),
        (
            "slashed branch",
            format!("{LINK_BASE}/tree/release/v2/skills/demo"),
            "release/v2",
        ),
        ("tag", format!("{LINK_BASE}/tree/v1/skills/demo"), "v1"),
        (
            "dot git",
            format!("{LINK_BASE}.git/tree/main/skills/demo"),
            "main",
        ),
        (
            "trailing slash",
            format!("{LINK_BASE}/tree/main/skills/demo/"),
            "main",
        ),
        (
            "pathless shorthand",
            "code/skill-src/skills/demo".to_string(),
            "main",
        ),
        (
            "no scheme",
            "github.com/code/skill-src/tree/main/skills/demo".to_string(),
            "main",
        ),
        (
            "no scheme, www",
            "www.github.com/code/skill-src/tree/main/skills/demo".to_string(),
            "main",
        ),
    ] {
        let got = j
            .call(&client, preview_request(&project, &url))
            .await
            .expect("preview a link form");
        j.alias(text(&got, "/token"), format!("$TOKEN_FORM_{}", forms.len()));
        let ok = text(&got, "/hash") == demo_hash
            && text(&got, "/ref") == want_ref
            && text(&got, "/path") == "skills/demo";
        forms.push((label, ok));
    }
    j.guard(
        "blob, shorthand, slashed-branch, tag, .git, slash and scheme-less forms agree",
        forms.iter().all(|(_, ok)| *ok),
        json!(forms
            .iter()
            .map(|(label, ok)| json!({ "form": label, "same_hash_and_ref": ok }))
            .collect::<Vec<_>>()),
    );
    let mut refusals = Vec::new();
    for (label, url, want) in [
        (
            "no path, several skills",
            LINK_BASE.to_string(),
            "skill_not_found_in_source",
        ),
        (
            "dotdot segment",
            format!("{LINK_BASE}/tree/main/skills/../x"),
            "skill_source_invalid",
        ),
        (
            "other host",
            "https://gitlab.com/a/b".to_string(),
            "skill_source_invalid",
        ),
        (
            "ssh form",
            "git@github.com:code/skill-src.git".to_string(),
            "skill_source_invalid",
        ),
        (
            "backslash",
            format!("{LINK_BASE}/tree/main/skills\\demo"),
            "skill_source_invalid",
        ),
        (
            "empty segment",
            format!("{LINK_BASE}/tree/main//demo"),
            "skill_source_invalid",
        ),
        (
            "leading slash",
            "/code/skill-src".to_string(),
            "skill_source_invalid",
        ),
        (
            "empty owner",
            "https://github.com//skill-src".to_string(),
            "skill_source_invalid",
        ),
        (
            "unknown subpage",
            format!("{LINK_BASE}/issues/1"),
            "skill_source_invalid",
        ),
        (
            "dead repo",
            "https://github.com/code/missing".to_string(),
            "skill_fetch_failed",
        ),
        (
            "unknown ref",
            format!("{LINK_BASE}/tree/nope/skills/demo"),
            "skill_fetch_failed",
        ),
        (
            "invalid name",
            format!("{LINK_BASE}/tree/main/skills/bad"),
            "skill_name_invalid",
        ),
    ] {
        let code = j
            .call(&client, preview_request(&project, &url))
            .await
            .err()
            .map(|e| e.code);
        refusals.push((label, code.clone(), code.as_deref() == Some(want)));
    }
    j.guard(
        "malformed, dead and invalid links are refused with their codes",
        refusals.iter().all(|(_, _, ok)| *ok),
        json!(refusals
            .iter()
            .map(|(label, code, ok)| json!({ "link": label, "code": code, "as_expected": ok }))
            .collect::<Vec<_>>()),
    );

    j.step("install global for both targets");
    let report = j
        .call(
            &client,
            install_request(&project, &text(&demo, "/token"), SkillScope::Global, &BOTH),
        )
        .await
        .expect("install demo globally");
    let report_steps: Vec<Value> = report
        .pointer("/steps")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .map(|s| json!({ "kind": s.pointer("/kind"), "target": s.pointer("/target"), "status": s.pointer("/status") }))
                .collect()
        })
        .unwrap_or_default();
    let store_demo = skill_store.join("demo");
    let want_link = json!(store_demo);
    let claude_links = sandbox.listing(&claude_config.join("skills"));
    let agents_links = sandbox.listing(&sandbox.home.join(".agents/skills"));
    let executable = std::fs::metadata(store_demo.join("scripts/run.sh"))
        .map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0)
        .unwrap_or(false);
    let store_files =
        ["SKILL.md", "refs/notes.md", "scripts/run.sh"].map(|f| store_demo.join(f).is_file());
    let token_dir = staging.join(text(&demo, "/token"));
    let claimed_dir = staging.join(format!(".claimed-{}", text(&demo, "/token")));
    let reused = install_with(
        &client,
        &project,
        &text(&demo, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await;
    let listed = j
        .call(&client, Request::ListSkillFiles { project_path: None })
        .await
        .expect("list global skills");
    let global_row = rows_named(&listed, "demo").first().copied().cloned();
    j.guard(
        "a global install stores once and links both targets",
        claude_links.pointer("/demo") == Some(&want_link)
            && agents_links.pointer("/demo") == Some(&want_link)
            && store_demo.join("SKILL.md").is_file()
            && store_demo.join("refs/notes.md").is_file()
            && executable
            && !store_demo.join("link").exists()
            && !token_dir.exists()
            && !claimed_dir.exists(),
        json!({
            "claude_link": claude_links.pointer("/demo"),
            "agents_link": agents_links.pointer("/demo"),
            "store_files": store_files,
            "run_sh_executable": executable,
            "symlink_copied": store_demo.join("link").exists(),
            "staging_left": [token_dir.exists(), claimed_dir.exists()],
        }),
    );
    j.guard(
        "the install report lists each step",
        report_steps
            == [
                json!({ "kind": "verify", "target": null, "status": "done" }),
                json!({ "kind": "store", "target": null, "status": "done" }),
                json!({ "kind": "link", "target": "claude", "status": "done" }),
                json!({ "kind": "link", "target": "agents", "status": "done" }),
                json!({ "kind": "lock", "target": null, "status": "done" }),
            ],
        json!({ "steps": report_steps }),
    );
    j.guard(
        "a token installs once",
        reused.as_ref().err().map(|e| e.code.as_str()) == Some("skill_preview_expired"),
        json!({ "code": reused.err().map(|e| e.code) }),
    );
    j.guard(
        "the list shows the install with its origin",
        global_row.as_ref().is_some_and(|r| {
            strings(r, "/targets") == ["claude", "agents"]
                && r.pointer("/installed") == Some(&Value::Bool(true))
                && r.pointer("/origin")
                    == Some(&json!({
                        "source": "github",
                        "repo": "code/skill-src",
                        "path": "skills/demo",
                        "ref": "main",
                        "commit": head,
                    }))
        }),
        json!({
            "targets": global_row.as_ref().map(|r| strings(r, "/targets")),
            "installed": global_row.as_ref().and_then(|r| r.pointer("/installed")),
            "origin": global_row.as_ref().and_then(|r| r.pointer("/origin")),
        }),
    );

    j.step("install project for Claude only");
    let commits_before = sandbox.git(&repo, &["rev-list", "--count", "HEAD"]);
    let again = preview(&mut j, &client, &project, "tree/main/skills/demo")
        .await
        .expect("preview demo again");
    j.alias(text(&again, "/token"), "$TOKEN_DEMO_PROJECT");
    install_with(
        &client,
        &project,
        &text(&again, "/token"),
        SkillScope::Project,
        &[SkillTarget::Claude],
    )
    .await
    .expect("install demo into the project");
    let project_store = repo.join(".agents/hyprlayer/skills/demo");
    let project_links = sandbox.listing(&repo.join(".claude/skills"));
    let untracked: Vec<String> = sandbox
        .git(&repo, &["status", "--porcelain", "--untracked-files=all"])
        .lines()
        .filter(|l| l.contains("demo") || l.contains("skills.lock"))
        .map(str::to_string)
        .collect();
    let commits_after = sandbox.git(&repo, &["rev-list", "--count", "HEAD"]);
    let listed = j
        .call(
            &client,
            Request::ListSkillFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list project skills");
    let mut sources: Vec<String> = rows_named(&listed, "demo")
        .iter()
        .map(|r| text(r, "/source"))
        .collect();
    sources.sort();
    j.guard(
        "a project install lands in the checkout, uncommitted",
        project_store.join("SKILL.md").is_file()
            && project_links.pointer("/demo") == Some(&json!("../../.agents/hyprlayer/skills/demo"))
            && std::fs::symlink_metadata(repo.join(".agents/skills/demo")).is_err()
            && untracked.len() == 5
            && untracked.iter().all(|l| l.starts_with("?? "))
            && commits_before == commits_after
            && sources == ["project", "user"],
        json!({
            "store_has_skill": project_store.join("SKILL.md").is_file(),
            "claude_link": project_links.pointer("/demo"),
            "agents_link_absent": std::fs::symlink_metadata(repo.join(".agents/skills/demo")).is_err(),
            "untracked": untracked,
            "commits_unchanged": commits_before == commits_after,
            "row_sources": sources,
        }),
    );

    j.step("concurrent installs serialize");
    let first = preview(&mut j, &client, &project, "tree/main/skills/other")
        .await
        .expect("preview other");
    let second = preview(&mut j, &client, &project, "tree/main/skills/other")
        .await
        .expect("preview other");
    j.alias(text(&first, "/token"), "$TOKEN_OTHER_A");
    j.alias(text(&second, "/token"), "$TOKEN_OTHER_B");
    let (a, b) = tokio::join!(
        client.call(install_request(
            &project,
            &text(&first, "/token"),
            SkillScope::Global,
            &BOTH
        )),
        client.call(install_request(
            &project,
            &text(&second, "/token"),
            SkillScope::Global,
            &BOTH
        )),
    );
    let outcomes = [a.map(|_| ()), b.map(|_| ())];
    let succeeded = outcomes.iter().filter(|r| r.is_ok()).count();
    let taken = outcomes
        .iter()
        .filter(|r| {
            r.as_ref()
                .err()
                .is_some_and(|e| e.code == "skill_name_taken")
        })
        .count();
    let leftovers: Vec<String> = std::fs::read_dir(&skill_store)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(".tmp-"))
                .collect()
        })
        .unwrap_or_default();
    let lock_keys = lock_skills(&sandbox, &global_lock);
    j.guard(
        "two installs of one name leave one winner and no debris",
        succeeded == 1 && taken == 1 && leftovers.is_empty() && skill_store.join("other/SKILL.md").is_file() && lock_keys == ["demo", "other"],
        json!({ "succeeded": succeeded, "skill_name_taken": taken, "tmp_dirs": leftovers, "lock_keys": lock_keys }),
    );
    j.call(
        &client,
        Request::RemoveSkill {
            project_path: None,
            scope: SkillScope::Global,
            name: "other".into(),
        },
    )
    .await
    .expect("remove other");

    j.step("collisions are refused");
    let commit_skill = preview(&mut j, &client, &project, "tree/main/skills/commit")
        .await
        .expect("preview commit");
    j.alias(text(&commit_skill, "/token"), "$TOKEN_COMMIT");
    let reserved_install = install_with(
        &client,
        &project,
        &text(&commit_skill, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await;
    j.guard(
        "a stage command name is reserved in both scopes",
        commit_skill.pointer("/conflicts")
            == Some(&json!([
                { "scope": "project", "reason": "reserved" },
                { "scope": "global", "reason": "reserved" },
            ]))
            && reserved_install.as_ref().err().map(|e| e.code.as_str())
                == Some("skill_name_reserved"),
        json!({
            "conflicts": commit_skill.pointer("/conflicts"),
            "install_code": reserved_install.err().map(|e| e.code),
        }),
    );
    let dup = preview(&mut j, &client, &project, "tree/main/skills/demo")
        .await
        .expect("re-preview demo");
    j.alias(text(&dup, "/token"), "$TOKEN_DEMO_DUP");
    let dup_install = install_with(
        &client,
        &project,
        &text(&dup, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await;
    let dup_err = dup_install.as_ref().err();
    j.guard(
        "an installed name conflicts in the scopes that hold it",
        dup.pointer("/conflicts")
            == Some(&json!([
                { "scope": "project", "reason": "installed" },
                { "scope": "global", "reason": "installed" },
            ]))
            && dup_err.is_some_and(|e| {
                e.code == "skill_name_taken" && e.remedy.as_deref() == Some("Remove it first.")
            }),
        json!({
            "conflicts": dup.pointer("/conflicts"),
            "code": dup_err.map(|e| e.code.clone()),
            "remedy": dup_err.map(|e| e.remedy.clone()),
        }),
    );

    // A managed row laid out as the CLI does: a link into `<config>/agents`.
    let bundle_skill = skill_store
        .with_file_name("agents")
        .join("0.0.0/claude/skills/bundlelike");
    write_skill(&bundle_skill, "bundlelike");
    std::os::unix::fs::symlink(&bundle_skill, claude_config.join("skills/bundlelike"))
        .expect("link a bundle skill");
    write_skill(&repo.join(".claude/skills/handmade"), "handmade");
    commit_on_branch(&sandbox, &src, "clash", &["bundlelike", "handmade"]);
    let listed = j
        .call(
            &client,
            Request::ListSkillFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list with a bundle skill");
    let bundle_row = rows_named(&listed, "bundlelike").first().copied().cloned();
    let bundle = preview(&mut j, &client, &project, "tree/clash/skills/bundlelike")
        .await
        .expect("preview the bundle clash");
    j.alias(text(&bundle, "/token"), "$TOKEN_BUNDLE");
    let bundle_install = install_with(
        &client,
        &project,
        &text(&bundle, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await;
    let bundle_err = bundle_install.as_ref().err();
    j.guard(
        "a bundle skill's name is taken and not removable here",
        bundle_row.as_ref().and_then(|r| r.pointer("/managed")) == Some(&Value::Bool(true))
            && bundle_err.is_some_and(|e| e.code == "skill_name_taken" && e.remedy.is_none()),
        json!({
            "managed": bundle_row.as_ref().and_then(|r| r.pointer("/managed")),
            "code": bundle_err.map(|e| e.code.clone()),
            "remedy": bundle_err.map(|e| e.remedy.clone()),
        }),
    );
    let handmade = preview(&mut j, &client, &project, "tree/clash/skills/handmade")
        .await
        .expect("preview the handmade clash");
    j.alias(text(&handmade, "/token"), "$TOKEN_HANDMADE");
    let handmade_install = install_with(
        &client,
        &project,
        &text(&handmade, "/token"),
        SkillScope::Project,
        &[SkillTarget::Claude],
    )
    .await;
    let handmade_err = handmade_install.as_ref().err();
    j.guard(
        "a hand-made folder is never overwritten",
        handmade.pointer("/conflicts") == Some(&json!([{ "scope": "project", "reason": "taken" }]))
            && handmade_err.is_some_and(|e| e.code == "skill_name_taken" && e.remedy.is_none())
            && repo.join(".claude/skills/handmade/SKILL.md").is_file(),
        json!({
            "conflicts": handmade.pointer("/conflicts"),
            "code": handmade_err.map(|e| e.code.clone()),
            "remedy": handmade_err.map(|e| e.remedy.clone()),
        }),
    );

    j.step("the staged content is what was approved");
    let other = preview(&mut j, &client, &project, "tree/main/skills/other")
        .await
        .expect("preview other again");
    j.alias(text(&other, "/token"), "$TOKEN_TAMPER");
    let staged = staging.join(text(&other, "/token")).join("skill/SKILL.md");
    let mut body = std::fs::read_to_string(&staged).expect("read the staged SKILL.md");
    body.push_str("tampered\n");
    std::fs::write(&staged, body).expect("tamper with the staged SKILL.md");
    let tampered = install_with(
        &client,
        &project,
        &text(&other, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await;
    let unknown = install_with(&client, &project, "nope", SkillScope::Global, &BOTH).await;
    let traversal = install_with(&client, &project, "../x", SkillScope::Global, &BOTH).await;
    j.guard(
        "edited staging is refused and installs nothing",
        tampered.as_ref().err().map(|e| e.code.as_str()) == Some("skill_content_changed")
            && !skill_store.join("other").exists()
            && std::fs::symlink_metadata(claude_config.join("skills/other")).is_err()
            && !repo.join(".agents/hyprlayer/skills/other").exists(),
        json!({
            "tampered": tampered.err().map(|e| e.code),
            "installed_anywhere": skill_store.join("other").exists(),
        }),
    );
    j.guard(
        "an unknown or path-like token has expired",
        unknown.as_ref().err().map(|e| e.code.as_str()) == Some("skill_preview_expired")
            && traversal.as_ref().err().map(|e| e.code.as_str()) == Some("skill_preview_expired"),
        json!({ "unknown": unknown.err().map(|e| e.code), "traversal": traversal.err().map(|e| e.code) }),
    );
    let untargeted = preview(&mut j, &client, &project, "tree/main/skills/other")
        .await
        .expect("preview other again");
    j.alias(text(&untargeted, "/token"), "$TOKEN_OTHER_UNTARGETED");
    let refused = install_with(
        &client,
        &project,
        &text(&untargeted, "/token"),
        SkillScope::Global,
        &[],
    )
    .await;
    j.guard(
        "an install with no targets is refused and installs nothing",
        refused.as_ref().err().map(|e| e.code.as_str()) == Some("skill_targets_empty")
            && !skill_store.join("other").exists(),
        json!({ "code": refused.err().map(|e| e.code) }),
    );

    j.step("lockfiles");
    let project_lock =
        std::fs::read_to_string(repo.join(".agents/hyprlayer/skills.lock")).unwrap_or_default();
    let global_lock_value = sandbox.read_json(&global_lock);
    let global_entry = global_lock_value.pointer("/skills/demo");
    let entry_keys: Vec<String> = global_entry
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    j.guard(
        "the project lock is sorted, has no installedAt, and ends at version",
        project_lock.contains("\"targets\": [\n        \"claude\"\n      ]")
            && !project_lock.contains("installedAt")
            && project_lock.ends_with("\"version\": 1\n}\n"),
        json!({ "lock": project_lock.lines().collect::<Vec<_>>() }),
    );
    j.guard(
        "the global lock adds installedAt and sorts targets by wire string",
        entry_keys
            == [
                "commit",
                "hash",
                "installedAt",
                "path",
                "ref",
                "repo",
                "source",
                "targets",
            ]
            && global_entry.is_some_and(|e| strings(e, "/targets") == ["agents", "claude"]),
        json!({ "lock": global_lock_value }),
    );

    j.step("a copied install still counts");
    let link = repo.join(".claude/skills/demo");
    std::fs::remove_file(&link).expect("remove the project link");
    copy_tree(&project_store, &link);
    // The mode bit is outside the hash, so losing it (a Windows checkout) keeps the copy ours.
    std::fs::set_permissions(
        link.join("scripts/run.sh"),
        std::os::unix::fs::PermissionsExt::from_mode(0o644),
    )
    .expect("drop the executable bit");
    let project_demo = |rows: &Value| {
        rows_named(rows, "demo")
            .into_iter()
            .find(|r| text(r, "/source") == "project")
            .cloned()
    };
    let listed = j
        .call(
            &client,
            Request::ListSkillFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list with a copy");
    let copied = project_demo(&listed);
    std::fs::write(
        link.join("SKILL.md"),
        format!(
            "{}edited\n",
            std::fs::read_to_string(link.join("SKILL.md")).unwrap_or_default()
        ),
    )
    .expect("edit the copy");
    let listed = j
        .call(
            &client,
            Request::ListSkillFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list with an edited copy");
    let edited = project_demo(&listed);
    std::fs::remove_dir_all(&link).expect("remove the copy");
    std::os::unix::fs::symlink("../../.agents/hyprlayer/skills/demo", &link)
        .expect("restore the link");
    j.guard(
        "a pristine copy is installed; an edited copy is not",
        copied.as_ref().is_some_and(|r| r.pointer("/installed") == Some(&Value::Bool(true)) && r.pointer("/origin").is_some_and(|o| !o.is_null()))
            && edited.as_ref().is_some_and(|r| r.pointer("/installed") == Some(&Value::Bool(false)) && r.pointer("/origin").is_none_or(Value::is_null)),
        json!({
            "pristine_installed": copied.as_ref().and_then(|r| r.pointer("/installed")),
            "pristine_has_origin": copied.as_ref().is_some_and(|r| r.pointer("/origin").is_some_and(|o| !o.is_null())),
            "edited_installed": edited.as_ref().and_then(|r| r.pointer("/installed")),
            "edited_has_origin": edited.as_ref().is_some_and(|r| r.pointer("/origin").is_some_and(|o| !o.is_null())),
        }),
    );

    j.step("skills.sh");
    let searched = j
        .call(
            &client,
            Request::SearchSkillSource {
                query: "demo tools".into(),
            },
        )
        .await
        .expect("search skills.sh");
    let requests_after_search = api_requests.lock().expect("request log").clone();
    j.guard(
        "search maps results",
        searched
            == json!([
                { "name": "demo", "source": "code/skill-src", "skillId": "demo", "installs": 42.0 },
                { "name": "tools", "source": "code/skill-src", "skillId": "tools", "installs": 1.0 },
            ])
            && requests_after_search == ["GET /api/search?q=demo+tools&limit=20 HTTP/1.1"],
        json!({ "results": searched, "requests": requests_after_search }),
    );
    let short = j
        .call(
            &client,
            Request::SearchSkillSource {
                query: " d ".into(),
            },
        )
        .await
        .expect("search with a short query");
    let requests_after_short = api_requests.lock().expect("request log").len();
    j.guard(
        "a short query never calls out",
        short == json!([]) && requests_after_short == requests_after_search.len(),
        json!({ "results": short, "requests": requests_after_short }),
    );
    let board = j
        .call(
            &client,
            Request::SkillLeaderboard {
                view: SkillsShView::Trending,
                page: 1,
            },
        )
        .await
        .expect("load a leaderboard page");
    let board_request = api_requests
        .lock()
        .expect("request log")
        .last()
        .cloned()
        .unwrap_or_default();
    j.guard(
        "a leaderboard page maps rows and paging",
        board
            == json!({
                "skills": [{ "name": "demo", "source": "code/skill-src", "skillId": "demo", "installs": 3700000.0 }],
                "hasMore": true,
            })
            && board_request == "GET /api/skills/trending/1 HTTP/1.1",
        json!({ "page": board, "request": board_request }),
    );

    let picked = j
        .call(
            &client,
            skills_sh_request(&project, "code/skill-src", "demo"),
        )
        .await
        .expect("preview a skills.sh pick");
    j.alias(text(&picked, "/token"), "$TOKEN_SH_DEMO");
    let traversal = j
        .call(&client, skills_sh_request(&project, "../etc", "demo"))
        .await
        .err()
        .map(|e| e.code);
    j.guard(
        "a pick previews the folder it names",
        text(&picked, "/path") == "skills/demo"
            && text(&picked, "/hash") == demo_hash
            && text(&picked, "/source") == "skills.sh"
            && traversal.as_deref() == Some("skill_source_invalid"),
        json!({
            "path": text(&picked, "/path"),
            "same_hash": text(&picked, "/hash") == demo_hash,
            "source": text(&picked, "/source"),
            "traversal": traversal,
        }),
    );

    let tools = j
        .call(
            &client,
            skills_sh_request(&project, "code/skill-src", "tools"),
        )
        .await
        .expect("preview a frontmatter-named pick");
    j.alias(text(&tools, "/token"), "$TOKEN_SH_TOOLS");
    let tools_install = install_with(
        &client,
        &project,
        &text(&tools, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await;
    let tools_entry = sandbox
        .read_json(&global_lock)
        .pointer("/skills/tools")
        .cloned();
    let tools_stored = skill_store.join("tools/SKILL.md").is_file();
    j.guard(
        "a frontmatter name resolves",
        text(&tools, "/path") == "skills/tools-v2"
            && text(&tools, "/name") == "tools"
            && tools_install.is_ok()
            && tools_stored
            && tools_entry
                .as_ref()
                .is_some_and(|e| text(e, "/source") == "skills.sh"),
        json!({
            "path": text(&tools, "/path"),
            "name": text(&tools, "/name"),
            "install_ok": tools_install.is_ok(),
            "store_folder_tools": tools_stored,
            "lock_source": tools_entry.as_ref().map(|e| text(e, "/source")),
        }),
    );
    j.call(
        &client,
        Request::RemoveSkill {
            project_path: None,
            scope: SkillScope::Global,
            name: "tools".into(),
        },
    )
    .await
    .expect("remove tools");

    let solo = sandbox.fixture_repo(
        "solo",
        &[(
            "SKILL.md",
            "---\nname: solo\ndescription: A repo that is one skill.\n---\n\nSolo body.\n",
        )],
    );
    sandbox.git(&solo, &["config", "uploadpack.allowFilter", "true"]);
    let root_pick = j
        .call(&client, skills_sh_request(&project, "code/solo", "solo"))
        .await;
    if let Ok(got) = &root_pick {
        j.alias(text(got, "/token"), "$TOKEN_SH_SOLO");
    }
    j.guard(
        "a pick whose skill is the repo root falls back to the root SKILL.md",
        root_pick
            .as_ref()
            .is_ok_and(|got| text(got, "/path").is_empty() && text(got, "/name") == "solo"),
        json!({
            "path": root_pick.as_ref().ok().map(|got| text(got, "/path")),
            "name": root_pick.as_ref().ok().map(|got| text(got, "/name")),
            "error": root_pick.as_ref().err().map(|e| e.code.clone()),
        }),
    );

    let unknown = j
        .call(
            &client,
            skills_sh_request(&project, "code/skill-src", "nope"),
        )
        .await
        .err()
        .map(|e| e.code);
    j.guard(
        "an unknown id is refused",
        unknown.as_deref() == Some("skill_not_found_in_source"),
        json!({ "code": unknown }),
    );

    j.step("updates");
    let other = preview(&mut j, &client, &project, "tree/v1/skills/other")
        .await
        .expect("preview other at the tag");
    j.alias(text(&other, "/token"), "$TOKEN_OTHER_V1");
    install_with(
        &client,
        &project,
        &text(&other, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await
    .expect("install other from the tag");
    let demo_md = src.join("skills/demo/SKILL.md");
    std::fs::write(
        &demo_md,
        format!(
            "{}Updated line.\n",
            std::fs::read_to_string(&demo_md).unwrap_or_default()
        ),
    )
    .expect("change demo upstream");
    sandbox.git(&src, &["add", "--all"]);
    sandbox.git(&src, &["commit", "--quiet", "--message", "Update demo"]);
    let new_head = sandbox.git(&src, &["rev-parse", "HEAD"]);
    j.alias(new_head.clone(), "$HEAD_UPDATE");
    let checked = j
        .call(
            &client,
            Request::CheckSkillUpdates {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("check for updates");
    let checked_rows: Vec<Value> = checked
        .as_array()
        .map(|all| {
            all.iter()
                .map(|u| {
                    json!({
                        "name": u.pointer("/name"),
                        "scope": u.pointer("/scope"),
                        "ref": u.pointer("/ref"),
                        "current_is_head": text(u, "/current") == head,
                        "latest_is_new_head": text(u, "/latest") == new_head,
                        "latest_is_current": text(u, "/latest") == text(u, "/current"),
                        "updateAvailable": u.pointer("/updateAvailable"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    j.guard(
        "a branch install floats; a tag install is pinned",
        checked_rows
            == [
                json!({ "name": "demo", "scope": "project", "ref": "main", "current_is_head": true, "latest_is_new_head": true, "latest_is_current": false, "updateAvailable": true }),
                json!({ "name": "demo", "scope": "global", "ref": "main", "current_is_head": true, "latest_is_new_head": true, "latest_is_current": false, "updateAvailable": true }),
                json!({ "name": "other", "scope": "global", "ref": "v1", "current_is_head": true, "latest_is_new_head": false, "latest_is_current": true, "updateAvailable": false }),
            ],
        json!({ "updates": checked_rows }),
    );

    let old_hash = lock_field(&sandbox, &global_lock, "demo", "hash");
    let claude_before = sandbox.listing(&claude_config.join("skills"));
    let agents_before = sandbox.listing(&sandbox.home.join(".agents/skills"));
    let update = j
        .call(
            &client,
            Request::PreviewSkillUpdate {
                project_path: Some(project.clone()),
                scope: SkillScope::Global,
                name: "demo".into(),
            },
        )
        .await
        .expect("preview the demo update");
    j.alias(text(&update, "/token"), "$TOKEN_UPDATE_GLOBAL");
    j.guard(
        "an update preview keeps the folder and summarizes the change",
        text(&update, "/name") == "demo"
            && text(&update, "/hash") != old_hash
            && text(&update, "/commit") == new_head
            && update.pointer("/changes")
                == Some(&json!({ "added": [], "removed": [], "changed": ["SKILL.md"] })),
        json!({
            "name": text(&update, "/name"),
            "hash_differs": text(&update, "/hash") != old_hash,
            "commit_is_new_head": text(&update, "/commit") == new_head,
            "changes": update.pointer("/changes"),
        }),
    );
    // The targets argument is ignored for a replace: the lock's are authoritative.
    let applied = j
        .call(
            &client,
            install_request(&project, &text(&update, "/token"), SkillScope::Global, &[]),
        )
        .await
        .expect("apply the update");
    let applied_steps: Vec<Value> = applied
        .pointer("/steps")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .map(|s| json!({ "kind": s.pointer("/kind"), "target": s.pointer("/target"), "detail": s.pointer("/detail") }))
                .collect()
        })
        .unwrap_or_default();
    let debris = |dirs: &[&Path]| -> Vec<String> {
        dirs.iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flat_map(|entries| entries.flatten())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".old-") || n.starts_with(".tmp-") || n.starts_with(".demo."))
            .collect()
    };
    let claude_skills = claude_config.join("skills");
    let agents_skills = sandbox.home.join(".agents/skills");
    let global_debris = debris(&[&skill_store, &claude_skills, &agents_skills]);
    j.guard(
        "applying an update swaps the store in place",
        lock_field(&sandbox, &global_lock, "demo", "commit") == new_head
            && lock_field(&sandbox, &global_lock, "demo", "hash") == text(&update, "/hash")
            && std::fs::read_to_string(skill_store.join("demo/SKILL.md"))
                .unwrap_or_default()
                .contains("Updated line.")
            && sandbox.listing(&claude_skills) == claude_before
            && sandbox.listing(&agents_skills) == agents_before
            && global_debris.is_empty(),
        json!({
            "steps": applied_steps,
            "lock_commit_is_new_head": lock_field(&sandbox, &global_lock, "demo", "commit") == new_head,
            "lock_hash_is_previewed": lock_field(&sandbox, &global_lock, "demo", "hash") == text(&update, "/hash"),
            "store_has_new_line": std::fs::read_to_string(skill_store.join("demo/SKILL.md")).unwrap_or_default().contains("Updated line."),
            "links_unchanged": [sandbox.listing(&claude_skills) == claude_before, sandbox.listing(&agents_skills) == agents_before],
            "debris": global_debris,
        }),
    );

    let project_link = repo.join(".claude/skills/demo");
    std::fs::remove_file(&project_link).expect("unlink the project demo");
    copy_tree(&project_store, &project_link);
    let project_update = j
        .call(
            &client,
            Request::PreviewSkillUpdate {
                project_path: Some(project.clone()),
                scope: SkillScope::Project,
                name: "demo".into(),
            },
        )
        .await
        .expect("preview the project update");
    j.alias(text(&project_update, "/token"), "$TOKEN_UPDATE_PROJECT");
    install_with(
        &client,
        &project,
        &text(&project_update, "/token"),
        SkillScope::Project,
        &[SkillTarget::Claude],
    )
    .await
    .expect("apply the project update");
    let listed = j
        .call(
            &client,
            Request::ListSkillFiles {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list after the copy refresh");
    let refreshed = project_demo(&listed);
    let copy_is_dir = std::fs::symlink_metadata(&project_link).is_ok_and(|m| m.is_dir());
    let project_debris = debris(&[
        &repo.join(".agents/hyprlayer/skills"),
        &repo.join(".claude/skills"),
    ]);
    j.guard(
        "a copy-fallback install is refreshed and stays installed",
        copy_is_dir
            && std::fs::read_to_string(project_link.join("SKILL.md"))
                .unwrap_or_default()
                .contains("Updated line.")
            && refreshed.as_ref().is_some_and(|r| r.pointer("/installed") == Some(&Value::Bool(true)))
            && project_debris.is_empty(),
        json!({
            "copy_is_dir": copy_is_dir,
            "copy_has_new_line": std::fs::read_to_string(project_link.join("SKILL.md")).unwrap_or_default().contains("Updated line."),
            "installed": refreshed.as_ref().and_then(|r| r.pointer("/installed")),
            "debris": project_debris,
        }),
    );
    std::fs::remove_dir_all(&project_link).expect("remove the refreshed copy");
    std::os::unix::fs::symlink("../../.agents/hyprlayer/skills/demo", &project_link)
        .expect("restore the project link");
    j.call(
        &client,
        Request::RemoveSkill {
            project_path: None,
            scope: SkillScope::Global,
            name: "other".into(),
        },
    )
    .await
    .expect("remove other");

    j.step("restore repairs from the lock");
    let good_hash = lock_field(&sandbox, &global_lock, "demo", "hash");
    let agents_demo = sandbox.home.join(".agents/skills/demo");
    std::fs::remove_file(&agents_demo).expect("drop the agents link");
    std::fs::remove_dir_all(skill_store.join("demo")).expect("drop the global store");
    let states = j
        .call(
            &client,
            Request::ListSkillLocks {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list lock states");
    j.guard(
        "a missing store shows as missing",
        states
            == json!([
                { "name": "demo", "scope": "project", "state": "ok" },
                { "name": "demo", "scope": "global", "state": "missing" },
            ]),
        json!({ "states": states }),
    );
    let refetched = j
        .call(
            &client,
            Request::RestoreSkills {
                project_path: Some(project.clone()),
                scope: SkillScope::Global,
            },
        )
        .await
        .expect("restore the global scope");
    let store_hash = hyprlayer_core::skill_sources::content_hash(&skill_store.join("demo")).ok();
    j.guard(
        "restore refetches the locked commit and relinks",
        refetched
            == json!({
                "scope": "global",
                "entries": [{ "name": "demo", "outcome": "refetched", "preview": null, "detail": null }],
            })
            && store_hash.as_deref() == Some(good_hash.as_str())
            && sandbox.listing(&agents_skills).pointer("/demo") == Some(&json!(skill_store.join("demo"))),
        json!({
            "report": refetched,
            "store_hash_matches_lock": store_hash.as_deref() == Some(good_hash.as_str()),
            "agents_link": sandbox.listing(&agents_skills).pointer("/demo"),
        }),
    );

    // A committed symlink checked out on Windows is a text file.
    std::fs::remove_file(&project_link).expect("drop the project link");
    std::fs::write(&project_link, "../../.agents/hyprlayer/skills/demo\n")
        .expect("write a text link");
    let states = j
        .call(
            &client,
            Request::ListSkillLocks {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list lock states with a text link");
    let relinked = j
        .call(
            &client,
            Request::RestoreSkills {
                project_path: Some(project.clone()),
                scope: SkillScope::Project,
            },
        )
        .await
        .expect("restore the project scope");
    let repaired =
        std::fs::symlink_metadata(&project_link).is_ok_and(|m| m.file_type().is_symlink());
    j.guard(
        "a text-file link is broken, then relinked as a symlink",
        states.pointer("/0/state") == Some(&json!("broken"))
            && relinked.pointer("/entries/0/outcome") == Some(&json!("relinked"))
            && repaired,
        json!({
            "state": states.pointer("/0/state"),
            "outcome": relinked.pointer("/entries/0/outcome"),
            "is_symlink": repaired,
        }),
    );

    let mut tampered = sandbox.read_json(&global_lock);
    if let Some(slot) = tampered.pointer_mut("/skills/demo/hash") {
        *slot = json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
    }
    std::fs::write(
        &global_lock,
        serde_json::to_string_pretty(&tampered).unwrap_or_default() + "\n",
    )
    .expect("edit the lock hash");
    std::fs::remove_dir_all(skill_store.join("demo")).expect("drop the store again");
    let approval = j
        .call(
            &client,
            Request::RestoreSkills {
                project_path: Some(project.clone()),
                scope: SkillScope::Global,
            },
        )
        .await
        .expect("restore against a changed lock");
    let pending = approval
        .pointer("/entries/0/preview")
        .cloned()
        .unwrap_or(Value::Null);
    j.alias(text(&pending, "/token"), "$TOKEN_RESTORE");
    j.guard(
        "content that differs from the lock needs approval",
        approval.pointer("/entries/0/outcome") == Some(&json!("needsApproval"))
            && text(&pending, "/hash") == good_hash
            && pending.pointer("/changes").is_some_and(Value::is_null),
        json!({
            "outcome": approval.pointer("/entries/0/outcome"),
            "preview_hash_is_real": text(&pending, "/hash") == good_hash,
            "changes": pending.pointer("/changes"),
        }),
    );
    let approved = install_with(
        &client,
        &project,
        &text(&pending, "/token"),
        SkillScope::Global,
        &BOTH,
    )
    .await;
    j.guard(
        "approving a restore refills the store and fixes the lock",
        approved.is_ok()
            && skill_store.join("demo/SKILL.md").is_file()
            && claude_skills.join("demo/SKILL.md").is_file()
            && agents_skills.join("demo/SKILL.md").is_file()
            && lock_field(&sandbox, &global_lock, "demo", "hash") == good_hash,
        json!({
            "install_ok": approved.is_ok(),
            "store_back": skill_store.join("demo/SKILL.md").is_file(),
            "links_resolve": [claude_skills.join("demo/SKILL.md").is_file(), agents_skills.join("demo/SKILL.md").is_file()],
            "lock_hash_fixed": lock_field(&sandbox, &global_lock, "demo", "hash") == good_hash,
        }),
    );

    j.step("skillfish.json lists candidates");
    let project_fish = repo.join("skillfish.json");
    let home_fish = sandbox.home.join("skillfish.json");
    let project_fish_text = r#"{"version":1,"skills":["code/skill-src@main/skills/demo","code/skill-src/skills/other","nonsense"]}"#;
    let home_fish_text = r#"{"version":1,"skills":["code/skill-src/skills/other"]}"#;
    std::fs::write(&project_fish, project_fish_text).expect("write the project skillfish.json");
    std::fs::write(&home_fish, home_fish_text).expect("write the home skillfish.json");
    let candidates = j
        .call(
            &client,
            Request::ListSkillfishCandidates {
                project_path: Some(project.clone()),
            },
        )
        .await
        .expect("list skillfish candidates");
    let imported = j
        .call(
            &client,
            preview_request(&project, "code/skill-src/skills/other"),
        )
        .await
        .expect("preview a candidate spec");
    j.alias(text(&imported, "/token"), "$TOKEN_SKILLFISH");
    let untouched = std::fs::read_to_string(&project_fish).ok().as_deref()
        == Some(project_fish_text)
        && std::fs::read_to_string(&home_fish).ok().as_deref() == Some(home_fish_text);
    j.guard(
        "candidates list uninstalled specs, skip nonsense, and never rewrite the file",
        candidates
            == json!([
                { "scope": "project", "spec": "code/skill-src@main/skills/demo", "installed": true },
                { "scope": "project", "spec": "code/skill-src/skills/other", "installed": false },
                { "scope": "global", "spec": "code/skill-src/skills/other", "installed": false },
            ])
            && text(&imported, "/name") == "other"
            && untouched,
        json!({
            "candidates": candidates,
            "preview_name": text(&imported, "/name"),
            "files_untouched": untouched,
        }),
    );
    std::fs::remove_file(&project_fish).expect("delete the project skillfish.json");
    std::fs::remove_file(&home_fish).expect("delete the home skillfish.json");

    j.step("remove");
    j.call(
        &client,
        Request::RemoveSkill {
            project_path: None,
            scope: SkillScope::Global,
            name: "demo".into(),
        },
    )
    .await
    .expect("remove demo globally");
    j.guard(
        "removing a global install deletes its links, store and lock",
        std::fs::symlink_metadata(claude_config.join("skills/demo")).is_err()
            && std::fs::symlink_metadata(sandbox.home.join(".agents/skills/demo")).is_err()
            && !skill_store.exists()
            && !global_lock.exists()
            && std::fs::symlink_metadata(claude_config.join("skills/bundlelike")).is_ok(),
        json!({
            "claude_link_gone": std::fs::symlink_metadata(claude_config.join("skills/demo")).is_err(),
            "agents_link_gone": std::fs::symlink_metadata(sandbox.home.join(".agents/skills/demo")).is_err(),
            "store_gone": !skill_store.exists(),
            "lock_gone": !global_lock.exists(),
            "bundle_link_kept": std::fs::symlink_metadata(claude_config.join("skills/bundlelike")).is_ok(),
        }),
    );
    j.call(
        &client,
        Request::RemoveSkill {
            project_path: Some(project.clone()),
            scope: SkillScope::Project,
            name: "sibling".into(),
        },
    )
    .await
    .expect("remove sibling");
    j.guard(
        "a lockless owned skill is removed, leaving the store root while demo stays",
        std::fs::symlink_metadata(repo.join(".claude/skills/sibling")).is_err()
            && std::fs::symlink_metadata(repo.join(".agents/skills/sibling")).is_err()
            && !repo.join(".agents/hyprlayer/skills/sibling").exists()
            && repo.join(".agents/hyprlayer").is_dir(),
        json!({
            "links_gone": [
                std::fs::symlink_metadata(repo.join(".claude/skills/sibling")).is_err(),
                std::fs::symlink_metadata(repo.join(".agents/skills/sibling")).is_err(),
            ],
            "store_gone": !repo.join(".agents/hyprlayer/skills/sibling").exists(),
            "store_root_kept": repo.join(".agents/hyprlayer").is_dir(),
        }),
    );
    std::fs::remove_file(&link).expect("unlink the project demo");
    copy_tree(&project_store, &link);
    j.call(
        &client,
        Request::RemoveSkill {
            project_path: Some(project.clone()),
            scope: SkillScope::Project,
            name: "demo".into(),
        },
    )
    .await
    .expect("remove demo from the project");
    j.guard(
        "a copy-fallback install is removed with its store, lock and empty root",
        std::fs::symlink_metadata(&link).is_err()
            && !project_store.exists()
            && !repo.join(".agents/hyprlayer").exists(),
        json!({
            "copy_gone": std::fs::symlink_metadata(&link).is_err(),
            "store_gone": !project_store.exists(),
            "store_root_gone": !repo.join(".agents/hyprlayer").exists(),
        }),
    );
    let again = j
        .call(
            &client,
            Request::RemoveSkill {
                project_path: Some(project.clone()),
                scope: SkillScope::Project,
                name: "demo".into(),
            },
        )
        .await;
    let foreign = j
        .call(
            &client,
            Request::RemoveSkill {
                project_path: Some(project),
                scope: SkillScope::Project,
                name: "handmade".into(),
            },
        )
        .await;
    j.guard(
        "nothing owned means skill_not_installed, and foreign folders survive",
        again.as_ref().err().map(|e| e.code.as_str()) == Some("skill_not_installed")
            && foreign.as_ref().err().map(|e| e.code.as_str()) == Some("skill_not_installed")
            && repo.join(".claude/skills/handmade/SKILL.md").is_file(),
        json!({
            "again": again.err().map(|e| e.code),
            "handmade": foreign.err().map(|e| e.code),
            "handmade_kept": repo.join(".claude/skills/handmade/SKILL.md").is_file(),
        }),
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

/// `~/code/skill-src`: the GitHub repo `code/skill-src` as `HYPRLAYER_SKILLS_GIT_BASE` sees it.
fn skill_source(sandbox: &Sandbox) -> std::path::PathBuf {
    let skill = |name: &str| {
        format!("---\nname: {name}\ndescription: Source skill {name}.\n---\n\nBody.\n")
    };
    let demo = "---\nname: demo\ndescription: Demo skill.\nallowed-tools: Bash(git:*), Read\n---\n\nDemo body.\n";
    let src = sandbox.fixture_repo(
        "skill-src",
        &[
            ("README.md", "# skill-src\n"),
            ("skills/demo/SKILL.md", demo),
            ("skills/demo/scripts/run.sh", "#!/bin/sh\necho demo\n"),
            ("skills/demo/refs/notes.md", "notes\n"),
            ("skills/other/SKILL.md", &skill("other")),
            ("skills/commit/SKILL.md", &skill("commit")),
            ("skills/bad/SKILL.md", &skill("Bad Name")),
            ("skills/tools-v2/SKILL.md", &skill("tools")),
        ],
    );
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        src.join("skills/demo/scripts/run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("make run.sh executable");
    std::os::unix::fs::symlink("../../README.md", src.join("skills/demo/link"))
        .expect("link README");
    sandbox.git(&src, &["add", "--all"]);
    sandbox.git(
        &src,
        &[
            "commit",
            "--quiet",
            "--message",
            "Mark the script executable",
        ],
    );
    sandbox.git(&src, &["config", "uploadpack.allowFilter", "true"]);
    sandbox.git(&src, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    sandbox.git(&src, &["tag", "v1"]);
    sandbox.git(&src, &["branch", "release/v2"]);
    src
}

/// Answers `/api/skills/...` with a leaderboard page, anything else with a search body, and logs each request line.
async fn skills_sh_stub() -> (std::net::SocketAddr, Arc<Mutex<Vec<String>>>) {
    const BOARD: &str = r#"{"skills":[{"source":"code/skill-src","skillId":"demo","name":"demo","installs":3700000,"weeklyInstalls":[1,2],"isOfficial":true},{"source":"uizze.sh","skillId":"ui","name":"ui","installs":9}],"total":2,"hasMore":true,"page":1}"#;
    const BODY: &str = r#"{"skills":[{"id":"code/skill-src/demo","source":"code/skill-src","skillId":"demo","name":"demo","installs":42},{"id":"code/skill-src/tools","source":"code/skill-src","skillId":"tools","name":"tools","installs":1},{"id":"x","source":"not-a-repo","skillId":"y","name":"y","installs":0}]}"#;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the stub");
    let addr = listener.local_addr().expect("stub address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = requests.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&head);
                let first = line.lines().next().unwrap_or_default().to_string();
                let body = if first.starts_with("GET /api/skills/") {
                    BOARD
                } else {
                    BODY
                };
                log.lock().expect("request log").push(first);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (addr, requests)
}

fn skills_sh_request(project: &str, source: &str, skill_id: &str) -> Request {
    Request::PreviewSkillInstall {
        project_path: Some(project.to_string()),
        source: SkillSourceInput::SkillsSh {
            source: source.to_string(),
            skill_id: skill_id.to_string(),
        },
    }
}

/// A `clash` branch carrying a skill for each name, then back to `main`.
fn commit_on_branch(sandbox: &Sandbox, src: &Path, branch: &str, names: &[&str]) {
    sandbox.git(src, &["checkout", "--quiet", "-b", branch]);
    for name in names {
        write_skill(&src.join("skills").join(name), name);
    }
    sandbox.git(src, &["add", "--all"]);
    sandbox.git(
        src,
        &["commit", "--quiet", "--message", "Add clashing skills"],
    );
    sandbox.git(src, &["checkout", "--quiet", "main"]);
}

fn preview_request(project: &str, url: &str) -> Request {
    Request::PreviewSkillInstall {
        project_path: Some(project.to_string()),
        source: SkillSourceInput::Url {
            url: url.to_string(),
        },
    }
}

/// Previews `<LINK_BASE>/<tail>`.
async fn preview(
    j: &mut Journey,
    client: &RpcClient,
    project: &str,
    tail: &str,
) -> Result<Value, WireError> {
    j.call(
        client,
        preview_request(project, &format!("{LINK_BASE}/{tail}")),
    )
    .await
}

fn install_request(
    project: &str,
    token: &str,
    scope: SkillScope,
    targets: &[SkillTarget],
) -> Request {
    Request::InstallSkill {
        project_path: Some(project.to_string()),
        token: token.to_string(),
        scope,
        targets: targets.to_vec(),
    }
}

/// Unrecorded: refusals and races don't need a transcript line each.
async fn install_with(
    client: &RpcClient,
    project: &str,
    token: &str,
    scope: SkillScope,
    targets: &[SkillTarget],
) -> Result<Value, WireError> {
    client
        .call(install_request(project, token, scope, targets))
        .await
        .map(|response| serde_json::to_value(&response).unwrap_or(Value::Null))
}

fn lock_field(sandbox: &Sandbox, lock: &Path, folder: &str, field: &str) -> String {
    text(
        &sandbox.read_json(lock),
        &format!("/skills/{folder}/{field}"),
    )
}

fn lock_skills(sandbox: &Sandbox, lock: &Path) -> Vec<String> {
    sandbox
        .read_json(lock)
        .pointer("/skills")
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create a copy dir");
    for entry in std::fs::read_dir(from)
        .expect("read a dir to copy")
        .flatten()
    {
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).expect("copy a file");
        }
    }
}

fn write_skill(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).expect("create a skill dir");
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Probe skill {name}.\n---\n\nBody.\n"),
    )
    .expect("write a SKILL.md");
}

fn rows_named<'a>(rows: &'a Value, name: &str) -> Vec<&'a Value> {
    rows.as_array()
        .map(|all| all.iter().filter(|r| text(r, "/name") == name).collect())
        .unwrap_or_default()
}

fn strings(row: &Value, pointer: &str) -> Vec<String> {
    row.pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|i| text(i, "")).collect())
        .unwrap_or_default()
}
