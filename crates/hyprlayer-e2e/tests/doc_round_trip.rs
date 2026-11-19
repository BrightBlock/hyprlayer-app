//! The save contract: `split`/`compose` round-trip body and frontmatter
//! losslessly (BOM, CRLF, malformed YAML, unknown keys), and `resolve_related`
//! matches `related:` values by stem, path, URL and title.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::docs::frontmatter::{FmPatch, FmValue};
use hyprlayer_core::docs::parts::split;
use hyprlayer_core::index::indexer::ThoughtQuery;
use hyprlayer_core::runtime::library::{CreateDocumentRequest, FrontmatterEdit};
use hyprlayer_e2e::flows::onboard_and_import;
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::RpcClient;

const PLAIN: &str = "---\ntitle: Plain\ntype: note\ndate: 2026-01-01\nstatus: active\ntags: [alpha, beta]\nrelated: []\n---\n\n# Plain\n\nBody text.\n";

const CRLF: &str = "---\r\ntitle: Crlf\r\ntype: note\r\ndate: 2026-01-01\r\n---\r\n\r\n# Crlf\r\n\r\nBody line.\r\n";

const NO_FRONTMATTER: &str = "# Heading\n\nJust a body, no frontmatter.\n";

const MALFORMED: &str =
    "---\ntitle: Malformed\ntags: [alpha, beta\n---\n\nBody after malformed frontmatter.\n";

const DOTS_CLOSER: &str = "---\ntitle: Dots closer\ntype: note\n...\nBody after dots closer.\n";

const UNKNOWN_KEYS: &str = "---\n# owner: platform team\ntitle: Unknown keys\ntype: note\nstatus: draft\nticket: null\ngit_commit: abc123\nbranch: feature/x\nmeta: {a: 1}\n# trailing comment\n---\n\nBody with unknown keys preserved.\n";

const BOM: &str = "\u{FEFF}---\ntitle: Bom\ntype: note\n---\n\nBody after BOM.\n";

const EMPTY_BLOCK: &str = "---\n---\n\nEmpty block body.\n";

const MIXED_EOL: &str =
    "---\r\ntitle: Mixed eol\r\ntype: note\r\n---\r\nLine one (LF)\nLine two (CRLF)\r\nLine three (LF)\n";

fn related_forms(mapped_other: &str) -> String {
    format!(
        "---\ntitle: Related forms\ntype: note\nrelated:\n  - Anchor\n  - plain\n  - thoughts/shared/notes/plain.md\n  - https://example.com/spec\n  - missing-stem\n  - 2026-01-01-twin\n  - 2026-01-01-dup-stem\n  - \"[[plain]]\"\n  - repos/{mapped_other}/shared/notes/2026-01-01-dup-stem.md\n---\n\nBody.\n"
    )
}

struct Fixture {
    name: &'static str,
    content: String,
    path: PathBuf,
    id: String,
}

fn file_mtime_ms(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Polls `QueryDocs` directly (not through `j.call`, so the polls stay out of
/// the transcript) until every fixture is present with an up-to-date mtime.
async fn await_indexed(client: &RpcClient, fixtures: &[Fixture], within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if let Ok(result) = client.query_docs(ThoughtQuery::default()).await {
            let all_fresh = fixtures.iter().all(|f| {
                result.items.iter().any(|item| {
                    item.id == f.id && (item.mtime_ms - file_mtime_ms(&f.path)).abs() < 0.5
                })
            });
            if all_fresh {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn evidence_map(names: &[&str], f: impl Fn(&str) -> Value) -> Value {
    let map: serde_json::Map<String, Value> = names.iter().map(|n| (n.to_string(), f(n))).collect();
    Value::Object(map)
}

fn f64_at(value: &Value, pointer: &str) -> f64 {
    value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn opt_str_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn strs_at(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn no_op_patch() -> FrontmatterEdit {
    FrontmatterEdit::Patch(FmPatch::default())
}

async fn create_doc(
    j: &mut Journey,
    client: &RpcClient,
    project: &str,
    scope: &str,
    doc_type: &str,
    title: &str,
    date: &str,
) -> Value {
    j.call(
        client,
        Request::CreateDoc {
            input: CreateDocumentRequest {
                project_repo_path: Some(project.to_string()),
                scope: scope.into(),
                doc_type: doc_type.into(),
                title: title.into(),
                date: date.into(),
                ticket: None,
                tags: vec![],
            },
        },
    )
    .await
    .expect("create_doc")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn doc_round_trip() {
    let sandbox = Sandbox::new();
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("doc_round_trip", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("set up the fixtures");
    let anchor = create_doc(
        &mut j,
        &client,
        &project,
        "shared",
        "note",
        "Anchor",
        "2026-01-01",
    )
    .await;
    let anchor_id = text(&anchor, "/id");
    j.alias(&anchor_id, "$ANCHOR");
    let anchor_path = j
        .call(
            &client,
            Request::ResolveDocPath {
                id: anchor_id.clone(),
            },
        )
        .await
        .expect("resolve_doc_path");
    let anchor_abs = text(&anchor_path, "");
    let notes_dir = Path::new(&anchor_abs)
        .parent()
        .expect("anchor has a parent dir")
        .to_path_buf();

    let demo_dup = create_doc(
        &mut j,
        &client,
        &project,
        "shared",
        "note",
        "Dup stem",
        "2026-01-01",
    )
    .await;
    let demo_dup_id = text(&demo_dup, "/id");
    j.alias(&demo_dup_id, "$DEMO_DUP");

    let other_repo = sandbox.fixture_repo("other", &[("README.md", "# other\n")]);
    let imported_other = j
        .call(
            &client,
            Request::ImportProject {
                path: sandbox.as_given(&other_repo).display().to_string(),
                branch: None,
                directory_name: None,
                thoughts_dir: None,
                profile: None,
                bootstrap_user: None,
            },
        )
        .await
        .expect("import_project (other)");
    let other_project = text(&imported_other, "/path");

    let other_dup = create_doc(
        &mut j,
        &client,
        &other_project,
        "shared",
        "note",
        "Dup stem",
        "2026-01-01",
    )
    .await;
    let other_dup_id = text(&other_dup, "/id");
    j.alias(&other_dup_id, "$OTHER_DUP");
    let other_dup_rel_path = text(&other_dup, "/relPath");
    let mapped_other = other_dup_rel_path
        .split('/')
        .nth(1)
        .expect("relPath starts with repos/<mapped>/")
        .to_string();

    create_doc(
        &mut j,
        &client,
        &other_project,
        "shared",
        "note",
        "Twin",
        "2026-01-01",
    )
    .await;
    create_doc(
        &mut j,
        &client,
        &other_project,
        "shared",
        "research",
        "Twin",
        "2026-01-01",
    )
    .await;

    let related_forms_content = related_forms(&mapped_other);
    let raw_fixtures: Vec<(&'static str, String)> = vec![
        ("plain", PLAIN.to_string()),
        ("crlf", CRLF.to_string()),
        ("no-frontmatter", NO_FRONTMATTER.to_string()),
        ("malformed", MALFORMED.to_string()),
        ("dots-closer", DOTS_CLOSER.to_string()),
        ("unknown-keys", UNKNOWN_KEYS.to_string()),
        ("bom", BOM.to_string()),
        ("empty-block", EMPTY_BLOCK.to_string()),
        ("related-forms", related_forms_content),
        ("mixed-eol", MIXED_EOL.to_string()),
    ];
    let fixture_names: Vec<&str> = raw_fixtures.iter().map(|(n, _)| *n).collect();
    let fixtures: Vec<Fixture> = raw_fixtures
        .into_iter()
        .map(|(name, content)| {
            let path = notes_dir.join(format!("{name}.md"));
            std::fs::write(&path, &content).unwrap_or_else(|e| panic!("write fixture {name}: {e}"));
            let id = path.display().to_string();
            Fixture {
                name,
                content,
                path,
                id,
            }
        })
        .collect();
    for f in &fixtures {
        let alias = format!("${}", f.name.replace('-', "_").to_uppercase());
        j.alias(&f.id, alias);
    }
    let find =
        |name: &str| -> &Fixture { fixtures.iter().find(|f| f.name == name).expect("fixture") };

    let indexed = await_indexed(&client, &fixtures, Duration::from_secs(20)).await;
    assert!(indexed, "fixtures did not get indexed in time");

    j.step("every fixture is indexed");
    let queried = j
        .call(
            &client,
            Request::QueryDocs {
                query: ThoughtQuery::default(),
            },
        )
        .await
        .expect("query_docs");
    let present: Vec<String> = queried
        .pointer("/items")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|i| text(i, "/id")).collect())
        .unwrap_or_default();
    let evidence = evidence_map(
        &fixture_names,
        |name| json!({ "indexed": present.contains(&find(name).id) }),
    );
    j.guard(
        "every fixture id is returned by QueryDocs",
        fixtures.iter().all(|f| present.contains(&f.id)),
        evidence,
    );

    j.step("loading splits each fixture");
    let mut docs: Vec<(&str, Value)> = Vec::new();
    for f in &fixtures {
        let doc = j
            .call(&client, Request::GetDoc { id: f.id.clone() })
            .await
            .unwrap_or_else(|e| panic!("get_doc {}: {e:?}", f.name));
        docs.push((f.name, doc));
    }
    let doc_for = |name: &str| -> &Value { &docs.iter().find(|(n, _)| *n == name).expect("doc").1 };

    let split_ok = fixtures.iter().all(|f| {
        let expected = split(&f.content);
        let expected_body = expected.body(&f.content);
        let doc = doc_for(f.name);
        let body_ok = text(doc, "/body") == expected_body;
        let has_fm_error = doc.pointer("/fm_error").is_some_and(|v| !v.is_null());
        body_ok && has_fm_error == (f.name == "malformed")
    });
    let split_evidence = evidence_map(&fixture_names, |name| {
        let doc = doc_for(name);
        json!({
            "frontmatter": doc.pointer("/frontmatter").is_some_and(|v| !v.is_null()),
            "fmError": doc.pointer("/fm_error").is_some_and(|v| !v.is_null()),
            "keys": strs_at(doc, "/fm_keys"),
        })
    });
    j.guard(
        "each fixture's body and fm_error match a fresh split",
        split_ok,
        split_evidence,
    );

    j.step("a save with no edits leaves every file byte-identical");
    let mut unchanged_ok = true;
    let mut unchanged_evidence = serde_json::Map::new();
    for f in &fixtures {
        let doc = doc_for(f.name);
        let before = std::fs::read(&f.path).unwrap();
        let before_mtime = file_mtime_ms(&f.path);
        let saved = j
            .call(
                &client,
                Request::SaveDocParts {
                    id: f.id.clone(),
                    body: text(doc, "/body"),
                    frontmatter: no_op_patch(),
                    base_mtime_ms: f64_at(doc, "/mtime_ms"),
                },
            )
            .await;
        let after = std::fs::read(&f.path).unwrap();
        let after_mtime = file_mtime_ms(&f.path);
        let ok = saved.is_ok() && after == before && (after_mtime - before_mtime).abs() < 0.5;
        unchanged_ok &= ok;
        unchanged_evidence.insert(f.name.to_string(), json!({ "unchanged": ok }));
    }
    j.guard(
        "a no-op save changes no file's bytes or mtime",
        unchanged_ok,
        Value::Object(unchanged_evidence),
    );

    j.step("an edit and its revert are byte-identical");
    let mut revert_ok = true;
    let mut revert_evidence = serde_json::Map::new();
    for f in &fixtures {
        let doc = doc_for(f.name);
        let lf_body = text(doc, "/body").replace("\r\n", "\n");
        let base = f64_at(doc, "/mtime_ms");
        let edited = j
            .call(
                &client,
                Request::SaveDocParts {
                    id: f.id.clone(),
                    body: format!("{lf_body}\nE2E_EDIT\n"),
                    frontmatter: no_op_patch(),
                    base_mtime_ms: base,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("edit {}: {e:?}", f.name));
        let mid_mtime = f64_at(&edited, "/mtimeMs");
        j.call(
            &client,
            Request::SaveDocParts {
                id: f.id.clone(),
                body: lf_body,
                frontmatter: no_op_patch(),
                base_mtime_ms: mid_mtime,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("revert {}: {e:?}", f.name));
        let after = std::fs::read(&f.path).unwrap();
        if f.name == "mixed-eol" {
            let expected_split = split(&f.content);
            let before_fm = &f.content.as_bytes()[..expected_split.body_offset];
            let after_fm = &after[..expected_split.body_offset.min(after.len())];
            let after_body = &after[expected_split.body_offset.min(after.len())..];
            let body_eol_normalised = !after_body.windows(2).any(|w| w == b"\r\n");
            let fm_unchanged = after_fm == before_fm;
            let ok = body_eol_normalised && fm_unchanged;
            revert_ok &= ok;
            revert_evidence.insert(
                f.name.to_string(),
                json!({ "body_eol_normalised": body_eol_normalised, "frontmatter_unchanged": fm_unchanged }),
            );
        } else {
            let ok = after == f.content.as_bytes();
            revert_ok &= ok;
            revert_evidence.insert(f.name.to_string(), json!({ "byte_identical": ok }));
        }
    }
    j.guard(
        "editing and reverting a body leaves the file byte-identical (mixed-eol excepted)",
        revert_ok,
        Value::Object(revert_evidence),
    );

    j.step("a save keeps the file's permissions");
    let f = &fixtures[0];
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let original_mode = mode(&f.path);
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fresh = j
        .call(&client, Request::GetDoc { id: f.id.clone() })
        .await
        .expect("get_doc");
    let body = text(&fresh, "/body").replace("\r\n", "\n");
    j.call(
        &client,
        Request::SaveDocParts {
            id: f.id.clone(),
            body: format!("{body}\nE2E_MODE\n"),
            frontmatter: no_op_patch(),
            base_mtime_ms: f64_at(&fresh, "/mtime_ms"),
        },
    )
    .await
    .expect("save_doc_parts");
    let kept = mode(&f.path);
    std::fs::write(&f.path, &f.content).unwrap();
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(original_mode)).unwrap();
    j.guard(
        "saving a doc leaves its file mode alone",
        kept == 0o755,
        json!({ "mode_after_save": format!("{kept:o}") }),
    );

    j.step("property edits write YAML and keep unknown keys");
    let unknown = find("unknown-keys");
    let before_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: unknown.id.clone(),
            },
        )
        .await
        .expect("get_doc unknown-keys before edit");
    let before_keys = strs_at(&before_doc, "/fm_keys");
    let before_extra = before_doc
        .pointer("/meta/fm/extra")
        .cloned()
        .unwrap_or(Value::Null);
    let before_body = text(&before_doc, "/body");
    let saved_meta = j
        .call(
            &client,
            Request::SaveDocParts {
                id: unknown.id.clone(),
                body: before_body.clone(),
                frontmatter: FrontmatterEdit::Patch(FmPatch {
                    set: vec![
                        ("status".into(), FmValue::Str("active".into())),
                        ("tags".into(), FmValue::List(vec!["x".into(), "y".into()])),
                    ],
                    remove: vec!["ticket".into()],
                }),
                base_mtime_ms: f64_at(&before_doc, "/mtime_ms"),
            },
        )
        .await
        .expect("save_doc_parts unknown-keys");
    let after_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: unknown.id.clone(),
            },
        )
        .await
        .expect("get_doc unknown-keys after edit");
    let after_keys = strs_at(&after_doc, "/fm_keys");
    let after_extra = after_doc
        .pointer("/meta/fm/extra")
        .cloned()
        .unwrap_or(Value::Null);
    let after_body = text(&after_doc, "/body");
    let after_fm_text = opt_str_at(&after_doc, "/frontmatter").unwrap_or_default();
    let status_ok = text(&saved_meta, "/fm/status") == "active";
    let tags_ok = strs_at(&saved_meta, "/fm/tags") == vec!["x".to_string(), "y".to_string()];
    let extra_ok = before_extra == after_extra;
    let old_minus_ticket: Vec<&String> = before_keys
        .iter()
        .filter(|k| k.as_str() != "ticket")
        .collect();
    let keys_no_ticket = !after_keys.iter().any(|k| k == "ticket");
    let keys_has_new =
        after_keys.iter().any(|k| k == "status") && after_keys.iter().any(|k| k == "tags");
    let status_same_place = before_keys.iter().position(|k| k == "status")
        == after_keys.iter().position(|k| k == "status");
    let order_preserved = {
        let mut idx = 0usize;
        old_minus_ticket.iter().all(|k| {
            if let Some(pos) = after_keys[idx..].iter().position(|a| a == *k) {
                idx += pos + 1;
                true
            } else {
                false
            }
        })
    };
    let body_unchanged = before_body == after_body;
    let comments_survive = after_fm_text.contains("# owner: platform team")
        && after_fm_text.contains("# trailing comment");
    let unknown_keys_ok = status_ok
        && tags_ok
        && extra_ok
        && keys_no_ticket
        && keys_has_new
        && status_same_place
        && order_preserved
        && body_unchanged
        && comments_survive;
    j.guard(
        "unknown-keys.md: status/tags set, ticket removed, unknown keys and comments survive",
        unknown_keys_ok,
        json!({
            "status": status_ok,
            "tags": tags_ok,
            "extraUnchanged": extra_ok,
            "ticketRemoved": keys_no_ticket,
            "newKeysPresent": keys_has_new,
            "statusInPlace": status_same_place,
            "orderPreserved": order_preserved,
            "bodyUnchanged": body_unchanged,
            "commentsSurvive": comments_survive,
            "beforeKeys": before_keys,
            "afterKeys": after_keys,
        }),
    );

    let no_fm = find("no-frontmatter");
    let no_fm_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: no_fm.id.clone(),
            },
        )
        .await
        .expect("get_doc no-frontmatter");
    j.call(
        &client,
        Request::SaveDocParts {
            id: no_fm.id.clone(),
            body: text(&no_fm_doc, "/body"),
            frontmatter: FrontmatterEdit::Patch(FmPatch {
                set: vec![("title".into(), FmValue::Str("Synthesized".into()))],
                remove: vec![],
            }),
            base_mtime_ms: f64_at(&no_fm_doc, "/mtime_ms"),
        },
    )
    .await
    .expect("save_doc_parts no-frontmatter");
    let synthesized = std::fs::read_to_string(&no_fm.path).unwrap();
    j.guard(
        "no-frontmatter.md: setting a property synthesizes a block",
        synthesized.starts_with("---\ntitle:"),
        json!({ "starts_with_block": synthesized.starts_with("---\ntitle:") }),
    );

    j.step("a malformed block is only replaced explicitly");
    let malformed = find("malformed");
    let malformed_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: malformed.id.clone(),
            },
        )
        .await
        .expect("get_doc malformed");
    let before_malformed = std::fs::read(&malformed.path).unwrap();

    let refused_patch = j
        .call(
            &client,
            Request::SaveDocParts {
                id: malformed.id.clone(),
                body: text(&malformed_doc, "/body"),
                frontmatter: FrontmatterEdit::Patch(FmPatch {
                    set: vec![("status".into(), FmValue::Str("active".into()))],
                    remove: vec![],
                }),
                base_mtime_ms: f64_at(&malformed_doc, "/mtime_ms"),
            },
        )
        .await;
    let refused_evidence = match &refused_patch {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    let after_refused = std::fs::read(&malformed.path).unwrap();
    j.guard(
        "a Patch on malformed.md is refused with frontmatter_patch, file unchanged",
        matches!(&refused_patch, Err(e) if e.code == "frontmatter_patch")
            && after_refused == before_malformed,
        refused_evidence,
    );

    let body_only = j
        .call(
            &client,
            Request::SaveDocParts {
                id: malformed.id.clone(),
                body: format!("{}\n", text(&malformed_doc, "/body").trim_end()),
                frontmatter: no_op_patch(),
                base_mtime_ms: f64_at(&malformed_doc, "/mtime_ms"),
            },
        )
        .await;
    let split_malformed = split(MALFORMED);
    let before_fm_bytes = &before_malformed[..split_malformed.body_offset];
    let after_body_only = std::fs::read(&malformed.path).unwrap();
    let after_fm_bytes = &after_body_only[..split_malformed.body_offset.min(after_body_only.len())];
    j.guard(
        "a body-only save on malformed.md is accepted and its frontmatter bytes are untouched",
        body_only.is_ok() && after_fm_bytes == before_fm_bytes,
        json!({ "accepted": body_only.is_ok(), "frontmatter_unchanged": after_fm_bytes == before_fm_bytes }),
    );

    let malformed_doc2 = j
        .call(
            &client,
            Request::GetDoc {
                id: malformed.id.clone(),
            },
        )
        .await
        .expect("get_doc malformed (2)");
    let bad_raw = j
        .call(
            &client,
            Request::SaveDocParts {
                id: malformed.id.clone(),
                body: text(&malformed_doc2, "/body"),
                frontmatter: FrontmatterEdit::Raw("tags: [still".into()),
                base_mtime_ms: f64_at(&malformed_doc2, "/mtime_ms"),
            },
        )
        .await;
    let bad_raw_evidence = match &bad_raw {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "an invalid Raw is refused",
        bad_raw.is_err(),
        bad_raw_evidence,
    );

    j.call(
        &client,
        Request::SaveDocParts {
            id: malformed.id.clone(),
            body: text(&malformed_doc2, "/body"),
            frontmatter: FrontmatterEdit::Raw("title: Fixed\ntags: [alpha, beta]\n".into()),
            base_mtime_ms: f64_at(&malformed_doc2, "/mtime_ms"),
        },
    )
    .await
    .expect("save_doc_parts valid Raw on malformed.md");
    let fixed_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: malformed.id.clone(),
            },
        )
        .await
        .expect("get_doc malformed after Raw fix");
    let fm_error_gone = fixed_doc.pointer("/fm_error").is_none_or(Value::is_null);
    let body_still_same = text(&fixed_doc, "/body") == text(&malformed_doc2, "/body");
    j.guard(
        "a valid Raw clears fm_error and leaves the body untouched",
        fm_error_gone && body_still_same,
        json!({ "fm_error_gone": fm_error_gone, "body_unchanged": body_still_same }),
    );

    let uk_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: unknown.id.clone(),
            },
        )
        .await
        .expect("get_doc unknown-keys before raw edit");
    let uk_before_fm = opt_str_at(&uk_doc, "/frontmatter").unwrap_or_default();
    let raw_edit = uk_before_fm
        .replace("branch: feature/x", "branch: feature/y")
        .replace("# trailing comment", "# trailing comment, edited");
    j.call(
        &client,
        Request::SaveDocParts {
            id: unknown.id.clone(),
            body: text(&uk_doc, "/body"),
            frontmatter: FrontmatterEdit::Raw(raw_edit.clone()),
            base_mtime_ms: f64_at(&uk_doc, "/mtime_ms"),
        },
    )
    .await
    .expect("save_doc_parts Raw on unknown-keys.md");
    let uk_doc_after = j
        .call(
            &client,
            Request::GetDoc {
                id: unknown.id.clone(),
            },
        )
        .await
        .expect("get_doc unknown-keys after raw edit");
    let uk_after_fm = opt_str_at(&uk_doc_after, "/frontmatter").unwrap_or_default();
    let only_targeted_lines_changed = uk_after_fm.contains("branch: feature/y")
        && uk_after_fm.contains("# trailing comment, edited")
        && !uk_after_fm.contains("branch: feature/x");
    j.guard(
        "a Raw save on a well-formed doc changes only the targeted lines",
        only_targeted_lines_changed,
        json!({ "only_targeted_lines_changed": only_targeted_lines_changed }),
    );

    j.step("related values resolve by stem, path, URL and title");
    let related = find("related-forms");
    let related_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: related.id.clone(),
            },
        )
        .await
        .expect("get_doc related-forms");
    let related_values = strs_at(&related_doc, "/meta/fm/related");
    let resolved = j
        .call(
            &client,
            Request::ResolveRelatedDocs {
                id: related.id.clone(),
                values: related_values.clone(),
            },
        )
        .await
        .expect("resolve_related_docs");

    let link_for = |raw: &str| -> Option<Value> {
        resolved.as_array().and_then(|links| {
            links
                .iter()
                .find(|l| l.pointer("/raw").and_then(Value::as_str) == Some(raw))
                .cloned()
        })
    };
    let expect_doc = |raw: &str, expected_id: &str| -> (bool, Value) {
        let link = link_for(raw);
        let ok = link.as_ref().is_some_and(|l| {
            l.pointer("/target/kind").and_then(Value::as_str) == Some("doc")
                && l.pointer("/target/id").and_then(Value::as_str) == Some(expected_id)
        });
        (ok, link.unwrap_or(Value::Null))
    };
    let expect_kind = |raw: &str, kind: &str| -> (bool, Value) {
        let link = link_for(raw);
        let ok = link
            .as_ref()
            .is_some_and(|l| l.pointer("/target/kind").and_then(Value::as_str) == Some(kind));
        (ok, link.unwrap_or(Value::Null))
    };
    let expect_ambiguous = |raw: &str, ambiguous: bool| -> (bool, Value) {
        let link = link_for(raw);
        let ok = link.as_ref().is_some_and(|l| {
            l.pointer("/target/kind").and_then(Value::as_str) == Some("unresolved")
                && l.pointer("/target/ambiguous").and_then(Value::as_bool) == Some(ambiguous)
        });
        (ok, link.unwrap_or(Value::Null))
    };

    let plain_id = find("plain").id.clone();
    let checks: Vec<(&str, (bool, Value))> = vec![
        (
            "Anchor -> $ANCHOR by title",
            expect_doc("Anchor", &anchor_id),
        ),
        ("plain -> $PLAIN", expect_doc("plain", &plain_id)),
        (
            "thoughts/shared/notes/plain.md -> $PLAIN",
            expect_doc("thoughts/shared/notes/plain.md", &plain_id),
        ),
        (
            "https url -> External",
            expect_kind("https://example.com/spec", "external"),
        ),
        (
            "missing-stem -> Unresolved, not ambiguous",
            expect_ambiguous("missing-stem", false),
        ),
        (
            "2026-01-01-twin -> Unresolved, ambiguous",
            expect_ambiguous("2026-01-01-twin", true),
        ),
        (
            "2026-01-01-dup-stem -> demo's copy",
            expect_doc("2026-01-01-dup-stem", &demo_dup_id),
        ),
        ("[[plain]] -> $PLAIN", expect_doc("[[plain]]", &plain_id)),
        (
            "other's relPath -> $OTHER_DUP",
            expect_doc(
                &format!("repos/{mapped_other}/shared/notes/2026-01-01-dup-stem.md"),
                &other_dup_id,
            ),
        ),
    ];
    let all_ok = checks.iter().all(|(_, (ok, _))| *ok);
    let evidence: serde_json::Map<String, Value> = checks
        .into_iter()
        .map(|(name, (ok, link))| (name.to_string(), json!({ "ok": ok, "link": link })))
        .collect();
    j.guard(
        "each related value resolves as expected, matched by raw",
        all_ok,
        Value::Object(evidence),
    );

    let abs_resolved = j
        .call(
            &client,
            Request::ResolveRelatedDocs {
                id: related.id.clone(),
                values: vec![plain_id.clone()],
            },
        )
        .await
        .expect("resolve_related_docs (absolute path)");
    let abs_ok = abs_resolved
        .as_array()
        .and_then(|a| a.first())
        .is_some_and(|l| {
            l.pointer("/target/kind").and_then(Value::as_str) == Some("doc")
                && l.pointer("/target/id").and_then(Value::as_str) == Some(plain_id.as_str())
        });
    j.guard(
        "an absolute path value resolves to $PLAIN",
        abs_ok,
        json!({ "ok": abs_ok }),
    );

    let mixed = j
        .call(
            &client,
            Request::ResolveRelatedDocs {
                id: related.id.clone(),
                values: vec!["plain".into(), "".into(), "missing-stem".into()],
            },
        )
        .await
        .expect("resolve_related_docs (mixed with empty)");
    let mixed_ok = mixed.as_array().is_some_and(|a| {
        a.len() == 3
            && a[1].pointer("/target/kind").and_then(Value::as_str) == Some("unresolved")
            && a[1].pointer("/target/ambiguous").and_then(Value::as_bool) == Some(false)
    });
    j.guard(
        "an empty value in the list resolves as Unresolved, not ambiguous",
        mixed_ok,
        json!({ "ok": mixed_ok, "count": mixed.as_array().map(|a| a.len()) }),
    );

    j.step("a stale save is refused");
    let plain_doc = j
        .call(
            &client,
            Request::GetDoc {
                id: plain_id.clone(),
            },
        )
        .await
        .expect("get_doc plain");
    let stale_base = f64_at(&plain_doc, "/mtime_ms");
    tokio::time::sleep(Duration::from_millis(5)).await;
    j.call(
        &client,
        Request::SaveDocParts {
            id: plain_id.clone(),
            body: format!("{}\nnudge\n", text(&plain_doc, "/body")),
            frontmatter: no_op_patch(),
            base_mtime_ms: stale_base,
        },
    )
    .await
    .expect("save_doc_parts to advance plain.md's mtime");
    let stale = j
        .call(
            &client,
            Request::SaveDocParts {
                id: plain_id,
                body: "CLOBBER\n".into(),
                frontmatter: no_op_patch(),
                base_mtime_ms: stale_base,
            },
        )
        .await;
    let stale_evidence = match &stale {
        Err(e) => json!({ "code": e.code }),
        Ok(_) => json!({ "accepted": true }),
    };
    j.guard(
        "a save from a stale mtime is refused with doc_conflict",
        matches!(&stale, Err(e) if e.code == "doc_conflict"),
        stale_evidence,
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
