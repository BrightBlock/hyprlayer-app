//! The mail-style Library's index contract: excerpts computed at index time,
//! paging, project spread, ticket search, the facet counts that label each
//! mailbox, server-side read state, and agent attribution from tool calls.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use hyprlayer_core::docs::frontmatter::FmPatch;
use hyprlayer_core::index::indexer::{DocMeta, SortKey, ThoughtQuery};
use hyprlayer_core::runtime::library::{CreateDocumentRequest, FrontmatterEdit};
use hyprlayer_core::{Harness, SessionEvent, Stage};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{json, text, Journey, Request, Sandbox, Server, Value};
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// `WRITE_DOC <abs>` writes the file and reports a successful `Write`;
/// `FAIL_DOC <abs>` writes it too but reports the tool call as failed.
const FAKE_CLAUDE: &str = r##"#!/bin/sh
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    --resume) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  err=""
  case "$line" in
    *WRITE_DOC*) err=false ;;
    *FAIL_DOC*) err=true ;;
  esac
  if [ -n "$err" ]; then
    path=$(printf '%s\n' "$line" | sed -n 's/.*_DOC \([^" ]*\).*/\1/p')
    mkdir -p "$(dirname "$path")"
    printf 'Written by the agent on turn %s.\n' "$turn" > "$path"
    printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_write_%s","name":"Write","input":{"file_path":"%s","content":"Written by the agent."}}]},"session_id":"%s"}\n' "$turn" "$path" "$sid"
    printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_write_%s","content":"File written.","is_error":%s}]},"session_id":"%s"}\n' "$turn" "$err" "$sid"
  fi
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7}}\n' "$turn" "$sid" "$turn"
done
"##;

/// The seeded codex session's id and its first row's timestamp (2026-01-01T00:00:00Z);
/// a backfilled attribution must carry the stored `created_at`, not the time it ran.
const CODEX_SESSION: &str = "e2e-codex-session";
const SEEDED_AT: f64 = 1_767_225_600_000.0;

const OG_PLAN: &str = "---\ntitle: OG plan\ntype: plan\nstatus: draft\nticket: POR-214\nauthor: jt\n---\n\n# OG plan\n\n## Overview\n\nThe **open graph** card renders from [the spec](https://example.com/og) and ships behind a flag.\n";

const AUDIT: &str = "---\ntitle: Audit\ntype: research\nstatus: active\n---\n\n```sh\ncargo audit\n```\n\nTwo advisories affect `time` and `chrono`; neither is reachable.\n";

const PAUSED: &str =
    "---\ntitle: Paused\ntype: handoff\nstatus: active\n---\n\n- [x] Index excerpts\n- [ ] Facet counts\n";

const LONG_SENTENCE: &str = "Lorem ipsum dolor sit amet. ";

const EMPTY: &str = "---\ntitle: Empty\ntype: pr\n---\n";

const OTHER_PLAN: &str =
    "---\ntitle: Other plan\ntype: plan\nstatus: implemented\n---\n\nShipped behind the flag.\n";

const ARCHIVED: &str = "---\ntitle: Archived\ntype: note\nstatus: archived\n---\n\nOld news.\n";

const SPEC: &str =
    "---\ntitle: Spec\ntype: spec\nstatus: wip\n---\n\nA spec in a type the vocabulary lacks.\n";

const OLD: &str = "---\ntitle: Old\ntype: note\n---\n\nWritten before the read baseline.\n";

/// 2020-01-01T00:00:00Z, long before any baseline.
const BEFORE_BASELINE_SECS: u64 = 1_577_836_800;

struct Fixture {
    name: &'static str,
    path: PathBuf,
    id: String,
    excerpt: String,
}

fn file_mtime_ms(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Writes through a non-`.md` sibling so the watcher sees one rename carrying
/// the final mtime, never the write and the `set_modified` as two events.
fn write_with_mtime(path: &Path, content: &str, mtime: SystemTime) {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, content).unwrap_or_else(|e| panic!("write {}: {e}", tmp.display()));
    std::fs::File::options()
        .write(true)
        .open(&tmp)
        .and_then(|f| f.set_modified(mtime))
        .unwrap_or_else(|e| panic!("set mtime {}: {e}", tmp.display()));
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("rename {}: {e}", path.display()));
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

fn item_ids(result: &Value) -> Vec<String> {
    result
        .pointer("/items")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|i| text(i, "/id")).collect())
        .unwrap_or_default()
}

fn mapped_name(created: &Value) -> String {
    text(created, "/relPath")
        .split('/')
        .nth(1)
        .expect("relPath starts with repos/<mapped>/")
        .to_string()
}

async fn create_doc(j: &mut Journey, client: &RpcClient, project: &str, title: &str) -> Value {
    j.call(
        client,
        Request::CreateDoc {
            input: CreateDocumentRequest {
                project_repo_path: Some(project.to_string()),
                scope: "shared".into(),
                doc_type: "note".into(),
                title: title.into(),
                date: "2026-01-01".into(),
                ticket: None,
                tags: vec![],
            },
        },
    )
    .await
    .expect("create_doc")
}

/// `<root>/repos/<mapped>/shared/`, from a created doc's resolved path.
async fn shared_dir(j: &mut Journey, client: &RpcClient, id: &str) -> PathBuf {
    let resolved = j
        .call(client, Request::ResolveDocPath { id: id.to_string() })
        .await
        .expect("resolve_doc_path");
    Path::new(&text(&resolved, ""))
        .ancestors()
        .nth(2)
        .expect("<shared>/<type>/<file>")
        .to_path_buf()
}

async fn query(j: &mut Journey, client: &RpcClient, query: ThoughtQuery) -> Value {
    j.call(client, Request::QueryDocs { query })
        .await
        .expect("query_docs")
}

/// Doc name → its `unread` flag in `result`, for every named doc.
fn unread_by_name(result: &Value, named: &[(String, String)]) -> serde_json::Map<String, Value> {
    named
        .iter()
        .map(|(name, id)| {
            let unread = result
                .pointer("/items")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().find(|i| text(i, "/id") == *id))
                .and_then(|i| i.get("unread").cloned())
                .unwrap_or(Value::Null);
            (name.clone(), unread)
        })
        .collect()
}

/// Polls `QueryDocs` off the transcript until the doc is listed and `ready` holds.
async fn await_doc(
    client: &RpcClient,
    id: &str,
    within: Duration,
    ready: impl Fn(&DocMeta) -> bool,
) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if let Ok(result) = client.query_docs(ThoughtQuery::default()).await {
            if result.items.iter().any(|item| item.id == id && ready(item)) {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn item(result: &Value, id: &str) -> Value {
    result
        .pointer("/items")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find(|i| text(i, "/id") == id))
        .cloned()
        .unwrap_or(Value::Null)
}

async fn recv_until(
    events: &mut EventStream<SessionEvent>,
    stop: impl Fn(&SessionEvent) -> bool,
) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(IO, async {
        while let Some(event) = events.recv().await {
            let done = stop(&event);
            seen.push(event);
            if done {
                return;
            }
        }
    })
    .await;
    seen
}

async fn drain_turn(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    recv_until(events, |e| matches!(e, SessionEvent::TurnDone { .. })).await
}

fn profile_db(sandbox: &Sandbox) -> rusqlite::Connection {
    let db = rusqlite::Connection::open(sandbox.app_data.join("hyprlayer.db"))
        .expect("open the profile database");
    db.busy_timeout(Duration::from_secs(5))
        .expect("set a busy timeout");
    db
}

/// `(doc_id, session_id, harness, workstream_id, first_at, last_at)`, ordered.
type AgentRow = (String, String, String, String, f64, f64);

fn doc_agent_rows(sandbox: &Sandbox) -> Vec<AgentRow> {
    let db = profile_db(sandbox);
    let mut stmt = db
        .prepare(
            "SELECT doc_id, session_id, harness, workstream_id, first_at, last_at
               FROM doc_agents ORDER BY doc_id, session_id",
        )
        .expect("prepare the doc_agents read");
    stmt.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })
    .expect("read doc_agents")
    .collect::<rusqlite::Result<_>>()
    .expect("decode doc_agents")
}

fn backfill_mark(sandbox: &Sandbox) -> Option<f64> {
    profile_db(sandbox)
        .query_row(
            "SELECT value FROM library_state WHERE key = 'agent_backfill_rowid'",
            [],
            |row| row.get(0),
        )
        .ok()
}

/// A codex session in `workstream_id`'s worktree whose successful `MultiEdit`
/// touched `relative` (through the worktree's `thoughts/` link) and `aliased`
/// (through a symlinked root), then wrote `outside`, which is no thoughts doc.
fn seed_codex_history(
    sandbox: &Sandbox,
    workstream_id: &str,
    worktree: &Path,
    relative: &str,
    aliased: &Path,
    outside: &Path,
) {
    let db = profile_db(sandbox);
    db.execute(
        "INSERT INTO sessions (id, workstream_id, stage, harness, cwd, state, started_at)
         VALUES (?1, ?2, 'general', 'codex', ?3, 'idle', ?4)",
        rusqlite::params![
            CODEX_SESSION,
            workstream_id,
            worktree.display().to_string(),
            SEEDED_AT
        ],
    )
    .expect("seed the codex session");
    let ok = |id: &str| SessionEvent::ToolResult {
        tool_use_id: id.into(),
        content: "Applied.".into(),
        is_error: false,
        parent_id: None,
    };
    let events = [
        SessionEvent::ToolUse {
            id: "call_multi".into(),
            name: "MultiEdit".into(),
            input: json!({
                "file_path": relative,
                "edits": [
                    { "file_path": relative, "old_string": "", "new_string": "Seeded." },
                    { "file_path": aliased, "old_string": "", "new_string": "Seeded note." },
                ],
            })
            .to_string(),
            parent_id: None,
        },
        ok("call_multi"),
        SessionEvent::ToolUse {
            id: "call_readme".into(),
            name: "Write".into(),
            input: json!({ "file_path": outside, "content": "# demo" }).to_string(),
            parent_id: None,
        },
        ok("call_readme"),
    ];
    for (seq, event) in events.iter().enumerate() {
        db.execute(
            "INSERT INTO session_events (session_id, seq, kind, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                CODEX_SESSION,
                seq as i64,
                event.kind(),
                serde_json::to_string(event).expect("encode a seeded event"),
                SEEDED_AT + seq as f64 * 1000.0
            ],
        )
        .expect("seed a session event");
    }
}

async fn counts(j: &mut Journey, client: &RpcClient, project: Option<&str>) -> Value {
    filtered_counts(j, client, project, None).await
}

async fn filtered_counts(
    j: &mut Journey,
    client: &RpcClient,
    project: Option<&str>,
    filters: Option<ThoughtQuery>,
) -> Value {
    j.call(
        client,
        Request::CountDocs {
            project: project.map(str::to_string),
            filters,
        },
    )
    .await
    .expect("count_docs")
}

/// Each mailbox as the frontend's `boxQuery` builds it: (box, type, status, unread only).
/// A box replaces the list filter on its own dimension.
const BOXES: [(&str, Option<&str>, Option<&str>, bool); 10] = [
    ("all", None, None, false),
    ("new", None, None, true),
    ("drafts", None, Some("draft"), false),
    ("plans", Some("plan"), None, false),
    ("research", Some("research"), None, false),
    ("handoffs", Some("handoff"), None, false),
    ("notes", Some("note"), None, false),
    ("prs", Some("pr"), None, false),
    ("implemented", None, Some("implemented"), false),
    ("archived", None, Some("archived"), false),
];

/// Every box's count beside the total of the list it opens, off the transcript.
async fn box_agreement(
    client: &RpcClient,
    project: Option<&str>,
    filters: &ThoughtQuery,
) -> (bool, Value) {
    let counted = client
        .count_docs(project.map(str::to_string), Some(filters.clone()))
        .await
        .expect("count_docs");
    let mut agree = true;
    let mut evidence = serde_json::Map::new();
    for (name, doc_type, status, unread_only) in BOXES {
        let mut list = ThoughtQuery {
            projects: project.map(str::to_string).into_iter().collect(),
            unread_only,
            ..filters.clone()
        };
        if let Some(t) = doc_type {
            list.types = vec![t.into()];
        }
        if let Some(s) = status {
            list.statuses = vec![s.into()];
        }
        let listed = client.query_docs(list).await.expect("query_docs").total;
        let count = match (doc_type, status) {
            (Some(t), _) => counted
                .types
                .iter()
                .find(|c| c.doc_type.as_str() == t)
                .map(|c| c.count),
            (_, Some(s)) => counted
                .statuses
                .iter()
                .find(|c| c.status.as_str() == s)
                .map(|c| c.count),
            _ if unread_only => Some(counted.unread),
            _ => Some(counted.total),
        };
        agree &= count == Some(listed);
        evidence.insert(name.into(), json!({ "count": count, "listed": listed }));
    }
    (agree, Value::Object(evidence))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn library_mail() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let repo = sandbox.fixture_repo("demo", &[("README.md", "# demo\n")]);
    let mut j = Journey::new("library_mail", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;

    j.step("set up the fixtures");
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

    let demo_seed = create_doc(&mut j, &client, &project, "Demo seed").await;
    let demo_seed_id = text(&demo_seed, "/id");
    j.alias(&demo_seed_id, "$DEMO_SEED");
    let other_seed = create_doc(&mut j, &client, &other_project, "Other seed").await;
    let other_seed_id = text(&other_seed, "/id");
    j.alias(&other_seed_id, "$OTHER_SEED");
    let demo_mapped = mapped_name(&demo_seed);
    let other_mapped = mapped_name(&other_seed);
    let demo_shared = shared_dir(&mut j, &client, &demo_seed_id).await;
    let other_shared = shared_dir(&mut j, &client, &other_seed_id).await;

    let long_body = LONG_SENTENCE.repeat(22).trim_end().to_string();
    let long_excerpt = format!("{}…", LONG_SENTENCE.repeat(7).trim_end());
    let raw: Vec<(&'static str, &Path, &str, String, &str)> = vec![
        (
            "og-plan",
            &demo_shared,
            "plans/og-plan.md",
            OG_PLAN.to_string(),
            "The open graph card renders from the spec and ships behind a flag.",
        ),
        (
            "audit",
            &demo_shared,
            "research/audit.md",
            AUDIT.to_string(),
            "Two advisories affect time and chrono; neither is reachable.",
        ),
        (
            "paused",
            &demo_shared,
            "handoffs/paused.md",
            PAUSED.to_string(),
            "Index excerpts Facet counts",
        ),
        (
            "long",
            &demo_shared,
            "notes/long.md",
            long_body,
            &long_excerpt,
        ),
        ("empty", &demo_shared, "prs/empty.md", EMPTY.to_string(), ""),
        (
            "other-plan",
            &other_shared,
            "plans/other-plan.md",
            OTHER_PLAN.to_string(),
            "Shipped behind the flag.",
        ),
        (
            "archived",
            &other_shared,
            "notes/archived.md",
            ARCHIVED.to_string(),
            "Old news.",
        ),
        (
            "spec",
            &other_shared,
            "notes/spec.md",
            SPEC.to_string(),
            "A spec in a type the vocabulary lacks.",
        ),
    ];
    for (_, shared, rel, _, _) in &raw {
        let dir = shared.join(rel).parent().expect("type dir").to_path_buf();
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("mkdir {}: {e}", dir.display()));
    }
    // Distinct mtimes (ties would break by id), all after the read baseline.
    let now = SystemTime::now();
    let fixtures: Vec<Fixture> = raw
        .into_iter()
        .enumerate()
        .map(|(i, (name, shared, rel, content, excerpt))| {
            let path = shared.join(rel);
            let mtime = now + Duration::from_secs(60 * (i as u64 + 1));
            write_with_mtime(&path, &content, mtime);
            Fixture {
                name,
                id: path.display().to_string(),
                path,
                excerpt: excerpt.to_string(),
            }
        })
        .collect();
    for f in &fixtures {
        j.alias(
            &f.id,
            format!("${}", f.name.replace('-', "_").to_uppercase()),
        );
    }
    let name_of = |id: &str| -> String {
        fixtures
            .iter()
            .find(|f| f.id == id)
            .map(|f| f.name.to_string())
            .unwrap_or_else(|| id.to_string())
    };

    let indexed = await_indexed(&client, &fixtures, Duration::from_secs(20)).await;
    assert!(indexed, "fixtures did not get indexed in time");

    j.step("excerpts are computed at index time");
    let all = query(&mut j, &client, ThoughtQuery::default()).await;
    let excerpt_of = |id: &str| -> Option<String> {
        all.pointer("/items")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|i| text(i, "/id") == id))
            .map(|i| text(i, "/excerpt"))
    };
    let mut excerpts_ok = true;
    let mut excerpts = serde_json::Map::new();
    for f in &fixtures {
        let got = excerpt_of(&f.id);
        excerpts_ok &= got.as_deref() == Some(f.excerpt.as_str());
        excerpts.insert(f.name.to_string(), json!(got));
    }
    j.guard(
        "each excerpt equals its expected literal (heading and fence skipped, markers stripped, inline markup dropped)",
        excerpts_ok,
        Value::Object(excerpts),
    );
    let long = excerpt_of(&fixtures[3].id).unwrap_or_default();
    let long_chars = long.chars().count();
    j.guard(
        "long.md is cut at a word boundary within 201 chars and ends with …",
        long_chars <= 201 && long.ends_with('…') && !long.ends_with(" …"),
        json!({ "chars": long_chars, "endsWithEllipsis": long.ends_with('…') }),
    );

    j.step("paging splits the mtime order without overlap");
    let mtime_query = |offset: Option<u32>, limit: Option<u32>| ThoughtQuery {
        sort: SortKey::Mtime,
        offset,
        limit,
        ..Default::default()
    };
    let unpaged = query(&mut j, &client, mtime_query(None, None)).await;
    let first = query(&mut j, &client, mtime_query(Some(0), Some(3))).await;
    let second = query(&mut j, &client, mtime_query(Some(3), Some(3))).await;
    let (first_ids, second_ids) = (item_ids(&first), item_ids(&second));
    let totals = [&unpaged, &first, &second].map(|r| r.pointer("/total").cloned());
    let disjoint = first_ids.iter().all(|id| !second_ids.contains(id));
    let union: Vec<String> = first_ids.iter().chain(&second_ids).cloned().collect();
    let first_six: Vec<String> = item_ids(&unpaged).into_iter().take(6).collect();
    let names = |ids: &[String]| ids.iter().map(|id| name_of(id)).collect::<Vec<_>>();
    j.guard(
        "both pages report the unpaged total, share no id, and together are the first six",
        totals.iter().all(|t| *t == totals[0])
            && first_ids.len() == 3
            && disjoint
            && union == first_six,
        json!({
            "totals": totals,
            "firstPage": names(&first_ids),
            "secondPage": names(&second_ids),
            "disjoint": disjoint,
            "unionIsFirstSix": union == first_six,
        }),
    );

    j.step("the unscoped result spans both projects");
    let projects = all.pointer("/projects").cloned().unwrap_or(Value::Null);
    j.guard(
        "QueryResult.projects counts demo and other",
        projects == json!(2),
        json!({ "projects": projects }),
    );

    j.step("a ticket id finds its doc");
    let by_ticket = query(
        &mut j,
        &client,
        ThoughtQuery {
            text: Some("POR-214".into()),
            ..Default::default()
        },
    )
    .await;
    let ticket_hits = names(&item_ids(&by_ticket));
    j.guard(
        "searching POR-214 returns only og-plan, whose title and body never mention it",
        ticket_hits == ["og-plan"],
        json!({ "hits": ticket_hits }),
    );

    j.step("facet counts label every mailbox");
    let unscoped = counts(&mut j, &client, None).await;
    let both_projects = json!([
        { "name": demo_mapped, "count": 6 },
        { "name": other_mapped, "count": 4 },
    ]);
    let expected_unscoped = json!({
        "total": 10,
        "types": [
            { "docType": "plan", "count": 2 },
            { "docType": "research", "count": 1 },
            { "docType": "handoff", "count": 1 },
            { "docType": "note", "count": 4 },
            { "docType": "pr", "count": 1 },
        ],
        "statuses": [
            { "status": "draft", "count": 3 },
            { "status": "active", "count": 2 },
            { "status": "implemented", "count": 1 },
            { "status": "superseded", "count": 0 },
            { "status": "archived", "count": 1 },
        ],
        "projects": both_projects,
        "authors": [
            { "name": "e2e", "count": 2 },
            { "name": "jt", "count": 1 },
        ],
        "unread": 8,
    });
    j.guard(
        "unscoped: every type and status listed (spec and wip uncounted), both projects, authors jt and the CLI user",
        unscoped == expected_unscoped,
        unscoped.clone(),
    );
    let scoped = counts(&mut j, &client, Some(&demo_mapped)).await;
    let expected_scoped = json!({
        "total": 6,
        "types": [
            { "docType": "plan", "count": 1 },
            { "docType": "research", "count": 1 },
            { "docType": "handoff", "count": 1 },
            { "docType": "note", "count": 2 },
            { "docType": "pr", "count": 1 },
        ],
        "statuses": [
            { "status": "draft", "count": 2 },
            { "status": "active", "count": 2 },
            { "status": "implemented", "count": 0 },
            { "status": "superseded", "count": 0 },
            { "status": "archived", "count": 0 },
        ],
        "projects": both_projects,
        "authors": [
            { "name": "e2e", "count": 1 },
            { "name": "jt", "count": 1 },
        ],
        "unread": 5,
    });
    j.guard(
        "scoped to demo: totals narrow while projects still lists both",
        scoped == expected_scoped,
        scoped.clone(),
    );

    j.step("under a list filter, each box counts the list it opens");
    let plans_only = ThoughtQuery {
        types: vec!["plan".into()],
        ..Default::default()
    };
    let filtered = filtered_counts(&mut j, &client, None, Some(plans_only.clone())).await;
    let expected_filtered = json!({
        "total": 2,
        "types": expected_unscoped["types"],
        "statuses": [
            { "status": "draft", "count": 1 },
            { "status": "active", "count": 0 },
            { "status": "implemented", "count": 1 },
            { "status": "superseded", "count": 0 },
            { "status": "archived", "count": 0 },
        ],
        "projects": both_projects,
        "authors": expected_unscoped["authors"],
        "unread": 2,
    });
    j.guard(
        "a type filter narrows every box but the type boxes, which replace it; projects and authors ignore it",
        filtered == expected_filtered,
        filtered.clone(),
    );
    let active_only = ThoughtQuery {
        statuses: vec!["active".into()],
        ..Default::default()
    };
    for (project, filters, guard) in [
        (
            None,
            &plans_only,
            "type filter, unscoped: every box's count is its list's total",
        ),
        (
            Some(demo_mapped.as_str()),
            &active_only,
            "status filter, scoped to demo: every box's count is its list's total",
        ),
    ] {
        let (agree, evidence) = box_agreement(&client, project, filters).await;
        j.guard(guard, agree, evidence);
    }

    j.step("read state starts from the baseline");
    let old_path = demo_shared.join("notes/old.md");
    write_with_mtime(
        &old_path,
        OLD,
        SystemTime::UNIX_EPOCH + Duration::from_secs(BEFORE_BASELINE_SECS),
    );
    let old = Fixture {
        name: "old",
        id: old_path.display().to_string(),
        path: old_path,
        excerpt: String::new(),
    };
    j.alias(&old.id, "$OLD");
    assert!(
        await_indexed(&client, std::slice::from_ref(&old), Duration::from_secs(20)).await,
        "old.md did not get indexed in time"
    );
    let mut named: Vec<(String, String)> = fixtures
        .iter()
        .map(|f| (f.name.to_string(), f.id.clone()))
        .collect();
    named.push(("old".into(), old.id.clone()));
    named.push(("demo-seed".into(), demo_seed_id.clone()));
    named.push(("other-seed".into(), other_seed_id.clone()));
    let fresh: Vec<&str> = fixtures.iter().map(|f| f.name).collect();
    let expected_state = |unread: &[&str]| -> serde_json::Map<String, Value> {
        named
            .iter()
            .map(|(name, _)| (name.clone(), json!(unread.contains(&name.as_str()))))
            .collect()
    };
    let all = query(&mut j, &client, ThoughtQuery::default()).await;
    let state = unread_by_name(&all, &named);
    j.guard(
        "old.md, modified before the read baseline, is not unread",
        state.get("old") == Some(&json!(false)),
        json!({ "old": state.get("old") }),
    );
    j.guard(
        "every fixture written after the baseline is unread; docs created through the Library are not",
        state == expected_state(&fresh),
        Value::Object(state.clone()),
    );

    j.step("the unread filter and counts agree");
    let unread_only = query(
        &mut j,
        &client,
        ThoughtQuery {
            unread_only: true,
            ..Default::default()
        },
    )
    .await;
    let mut unread_names = names(&item_ids(&unread_only));
    unread_names.sort();
    let mut fresh_sorted: Vec<String> = fresh.iter().map(|n| n.to_string()).collect();
    fresh_sorted.sort();
    let unscoped = counts(&mut j, &client, None).await;
    let scoped = counts(&mut j, &client, Some(&demo_mapped)).await;
    let evidence = json!({
        "unreadOnly": unread_names,
        "unreadOnlyTotal": unread_only.pointer("/total"),
        "unreadOnlyUnread": unread_only.pointer("/unread"),
        "allTotal": all.pointer("/total"),
        "allUnread": all.pointer("/unread"),
        "countsUnread": unscoped.pointer("/unread"),
        "demoCountsUnread": scoped.pointer("/unread"),
    });
    j.guard(
        "unreadOnly returns exactly the fresh fixtures; QueryResult.unread and CountDocs.unread match",
        unread_names == fresh_sorted
            && evidence["unreadOnlyTotal"] == json!(8)
            && evidence["unreadOnlyUnread"] == json!(8)
            && evidence["allTotal"] == json!(11)
            && evidence["allUnread"] == json!(8)
            && evidence["countsUnread"] == json!(8)
            && evidence["demoCountsUnread"] == json!(5),
        evidence,
    );

    j.step("marking read, then the user's own save");
    let og = &fixtures[0];
    let unread_of = |result: &Value, id: &str| -> Value {
        result
            .pointer("/items")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|i| text(i, "/id") == id))
            .and_then(|i| i.get("unread").cloned())
            .unwrap_or(Value::Null)
    };
    let opened = j
        .call(&client, Request::GetDoc { id: og.id.clone() })
        .await
        .expect("get_doc");
    j.call(&client, Request::MarkDocRead { id: og.id.clone() })
        .await
        .expect("mark_doc_read");
    let after_mark = query(&mut j, &client, ThoughtQuery::default()).await;
    let reopened = j
        .call(&client, Request::GetDoc { id: og.id.clone() })
        .await
        .expect("get_doc");
    j.guard(
        "GetDoc reports og-plan unread until MarkDocRead clears it",
        opened.pointer("/meta/unread") == Some(&json!(true))
            && unread_of(&after_mark, &og.id) == json!(false)
            && reopened.pointer("/meta/unread") == Some(&json!(false)),
        json!({
            "getBefore": opened.pointer("/meta/unread"),
            "queryAfter": unread_of(&after_mark, &og.id),
            "getAfter": reopened.pointer("/meta/unread"),
        }),
    );
    let saved = j
        .call(
            &client,
            Request::SaveDocParts {
                id: og.id.clone(),
                body: format!("{}\nRevised after reading.\n", text(&reopened, "/body")),
                frontmatter: FrontmatterEdit::Patch(FmPatch::default()),
                base_mtime_ms: reopened
                    .pointer("/mtime_ms")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
            },
        )
        .await
        .expect("save_doc_parts");
    let after_save = query(&mut j, &client, ThoughtQuery::default()).await;
    let revised = std::fs::read_to_string(&og.path).unwrap_or_default();
    j.guard(
        "a save that changes og-plan's body leaves it read",
        revised.contains("Revised after reading.")
            && saved.pointer("/unread") == Some(&json!(false))
            && unread_of(&after_save, &og.id) == json!(false),
        json!({
            "bodyChanged": revised.contains("Revised after reading."),
            "savedUnread": saved.pointer("/unread"),
            "queryUnread": unread_of(&after_save, &og.id),
        }),
    );

    j.step("a sync rewrite of the same bytes stays read");
    let rebased_at = SystemTime::now() + Duration::from_secs(60 * 10);
    write_with_mtime(&og.path, &revised, rebased_at);
    assert!(
        await_indexed(&client, std::slice::from_ref(og), Duration::from_secs(20)).await,
        "the rebased og-plan did not get indexed in time"
    );
    let after_rebase = query(&mut j, &client, ThoughtQuery::default()).await;
    j.guard(
        "identical bytes under a later mtime keep og-plan read",
        unread_of(&after_rebase, &og.id) == json!(false),
        json!({ "unread": unread_of(&after_rebase, &og.id) }),
    );

    j.step("an external edit makes it new again");
    write_with_mtime(
        &og.path,
        &format!("{revised}\nEdited outside the app.\n"),
        rebased_at + Duration::from_secs(2),
    );
    assert!(
        await_indexed(&client, std::slice::from_ref(og), Duration::from_secs(20)).await,
        "the edited og-plan did not get indexed in time"
    );
    let after_edit = query(&mut j, &client, ThoughtQuery::default()).await;
    j.guard(
        "new bytes under a later mtime make og-plan unread",
        unread_of(&after_edit, &og.id) == json!(true),
        json!({ "unread": unread_of(&after_edit, &og.id) }),
    );

    j.step("marking unread sticks");
    let audit = &fixtures[1];
    j.call(
        &client,
        Request::MarkDocRead {
            id: audit.id.clone(),
        },
    )
    .await
    .expect("mark_doc_read");
    let read = query(&mut j, &client, ThoughtQuery::default()).await;
    j.call(
        &client,
        Request::MarkDocUnread {
            id: audit.id.clone(),
        },
    )
    .await
    .expect("mark_doc_unread");
    let unread = query(&mut j, &client, ThoughtQuery::default()).await;
    let mtime_in = |result: &Value| -> Option<f64> {
        result
            .pointer("/items")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|i| text(i, "/id") == audit.id))
            .and_then(|i| i.pointer("/mtimeMs"))
            .and_then(Value::as_f64)
    };
    let same_mtime = mtime_in(&read).is_some() && mtime_in(&read) == mtime_in(&unread);
    j.guard(
        "MarkDocUnread after MarkDocRead makes audit unread at the same mtime",
        unread_of(&read, &audit.id) == json!(false)
            && unread_of(&unread, &audit.id) == json!(true)
            && same_mtime,
        json!({
            "afterRead": unread_of(&read, &audit.id),
            "afterUnread": unread_of(&unread, &audit.id),
            "sameMtime": same_mtime,
        }),
    );

    j.step("read state survives a restart");
    // Only a read fixture would differ if the table were lost.
    let paused = &fixtures[2];
    j.call(
        &client,
        Request::MarkDocRead {
            id: paused.id.clone(),
        },
    )
    .await
    .expect("mark_doc_read");
    let before = query(&mut j, &client, ThoughtQuery::default()).await;
    let before_state = unread_by_name(&before, &named);
    let before_counts = counts(&mut j, &client, None).await;
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the first server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let reindexed = await_indexed(&client, &fixtures, Duration::from_secs(20)).await
        && await_indexed(&client, std::slice::from_ref(&old), Duration::from_secs(20)).await;
    assert!(
        reindexed,
        "the restarted server did not index the fixtures in time"
    );
    let after = query(&mut j, &client, ThoughtQuery::default()).await;
    let after_state = unread_by_name(&after, &named);
    let after_counts = counts(&mut j, &client, None).await;
    let expected = expected_state(&[
        "og-plan",
        "audit",
        "long",
        "empty",
        "other-plan",
        "archived",
        "spec",
    ]);
    j.guard(
        "every doc's unread flag and CountDocs.unread are unchanged after a restart",
        before_state == expected
            && after_state == before_state
            && after_counts.pointer("/unread") == before_counts.pointer("/unread")
            && after_counts.pointer("/unread") == Some(&json!(7)),
        json!({
            "unread": Value::Object(after_state),
            "countsUnread": after_counts.pointer("/unread"),
        }),
    );

    j.step("a live agent write is attributed");
    let view = create_workstream(&mut j, &client, &project, "Agent writes").await;
    let ws = text(&view, "/item/id");
    let worktree = PathBuf::from(text(&view, "/item/worktreePath"));
    let live_id = demo_shared.join("plans/live-plan.md").display().to_string();
    let failed_id = demo_shared.join("plans/failed.md").display().to_string();
    j.alias(&live_id, "$LIVE_PLAN");
    j.alias(&failed_id, "$FAILED");
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            ws.clone(),
            Stage::General,
            format!("WRITE_DOC {live_id}"),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.alias(&session.id, "$SESSION");
    j.call(
        &client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    drain_turn(&mut events).await;
    assert!(
        await_doc(&client, &live_id, IO, |d| d.agent.is_some()).await,
        "live-plan.md was not attributed in time"
    );
    let listed = query(&mut j, &client, ThoughtQuery::default()).await;
    let live = item(&listed, &live_id);
    let agent = |doc: &Value| {
        json!({
            "harness": doc.pointer("/agent/harness"),
            "sessionId": doc.pointer("/agent/sessionId"),
            "workstreamId": doc.pointer("/agent/workstreamId"),
            "projectPath": doc.pointer("/agent/projectPath"),
        })
    };
    j.guard(
        "the claude session's Write credits live-plan to it, its workstream and project; the doc is new",
        agent(&live)
            == json!({
                "harness": "claude",
                "sessionId": session.id,
                "workstreamId": ws,
                "projectPath": project,
            })
            && live.get("unread") == Some(&json!(true)),
        json!({ "agent": agent(&live), "unread": live.get("unread") }),
    );

    j.step("a failed write is not");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: session.id.clone(),
            text: format!("FAIL_DOC {failed_id}"),
            attached_docs: vec![],
            prompt_name: None,
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    drain_turn(&mut events).await;
    // The watcher's debounce outlasts the attribution task a success would have spawned.
    assert!(
        await_doc(&client, &failed_id, IO, |_| true).await,
        "failed.md did not get indexed in time"
    );
    let listed = query(&mut j, &client, ThoughtQuery::default()).await;
    let failed = item(&listed, &failed_id);
    j.guard(
        "failed.md is indexed but carries no agent",
        !failed.is_null() && failed.get("agent").is_none_or(Value::is_null),
        json!({ "indexed": !failed.is_null(), "agent": failed.get("agent") }),
    );

    j.step("stored history is backfilled on start");
    let worktree_shared = worktree.join("thoughts/shared");
    j.guard(
        "the isolated worktree links thoughts/shared",
        worktree_shared.is_dir(),
        json!({ "linked": worktree_shared.is_dir() }),
    );
    drop(events);
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the second server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );
    let thoughts_root = demo_shared
        .ancestors()
        .nth(3)
        .expect("<root>/repos/<mapped>/shared")
        .to_path_buf();
    let alias_root = sandbox.root.join("alias-thoughts");
    std::os::unix::fs::symlink(&thoughts_root, &alias_root).expect("link the alias root");
    let seeded = demo_shared.join("research/seeded.md");
    let seeded_note = demo_shared.join("notes/seeded-note.md");
    std::fs::write(&seeded, "Seeded.\n").expect("write seeded.md");
    std::fs::write(&seeded_note, "Seeded note.\n").expect("write seeded-note.md");
    let (seeded_id, seeded_note_id) = (
        seeded.display().to_string(),
        seeded_note.display().to_string(),
    );
    j.alias(&seeded_id, "$SEEDED");
    j.alias(&seeded_note_id, "$SEEDED_NOTE");
    let aliased = alias_root
        .join("repos")
        .join(&demo_mapped)
        .join("shared/notes/seeded-note.md");
    seed_codex_history(
        &sandbox,
        &ws,
        &worktree,
        "thoughts/shared/research/seeded.md",
        &aliased,
        &worktree.join("README.md"),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let backfilled = await_doc(&client, &seeded_id, IO, |d| d.agent.is_some()).await
        && await_doc(&client, &seeded_note_id, IO, |d| d.agent.is_some()).await;
    assert!(backfilled, "the seeded docs were not backfilled in time");
    let listed = query(&mut j, &client, ThoughtQuery::default()).await;
    let harness_of = |id: &str| item(&listed, id).pointer("/agent/harness").cloned();
    let session_of = |id: &str| item(&listed, id).pointer("/agent/sessionId").cloned();
    j.guard(
        "the relative path (via the worktree link) and the absolute one (via a symlinked root) both credit codex",
        harness_of(&seeded_id) == Some(json!("codex"))
            && harness_of(&seeded_note_id) == Some(json!("codex"))
            && session_of(&seeded_id) == Some(json!(CODEX_SESSION))
            && session_of(&seeded_note_id) == Some(json!(CODEX_SESSION))
            && harness_of(&live_id) == Some(json!("claude")),
        json!({
            "seeded": harness_of(&seeded_id),
            "seededNote": harness_of(&seeded_note_id),
            "livePlan": harness_of(&live_id),
        }),
    );
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the third server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );
    let rows = doc_agent_rows(&sandbox);
    let pairs: Vec<(String, String)> = rows.iter().map(|r| (r.0.clone(), r.1.clone())).collect();
    let mut expected_pairs = vec![
        (live_id.clone(), session.id.clone()),
        (seeded_id.clone(), CODEX_SESSION.to_string()),
        (seeded_note_id.clone(), CODEX_SESSION.to_string()),
    ];
    expected_pairs.sort();
    let seeded_span: Vec<(f64, f64)> = rows
        .iter()
        .filter(|r| r.1 == CODEX_SESSION)
        .map(|r| (r.4, r.5))
        .collect();
    j.guard(
        "doc_agents holds exactly live-plan and the two seeded docs; nothing for README.md or failed.md",
        pairs == expected_pairs,
        json!({
            "rows": rows
                .iter()
                .map(|r| json!([r.0, r.1, r.2, r.3]))
                .collect::<Vec<_>>(),
        }),
    );
    j.guard(
        "backfilled rows carry the stored tool use's created_at, not the time the backfill ran",
        seeded_span == [(SEEDED_AT, SEEDED_AT), (SEEDED_AT, SEEDED_AT)],
        json!({ "seeded": seeded_span }),
    );

    j.step("a forced full re-backfill changes nothing");
    let mark = backfill_mark(&sandbox);
    profile_db(&sandbox)
        .execute(
            "DELETE FROM library_state WHERE key = 'agent_backfill_rowid'",
            [],
        )
        .expect("drop the backfill mark");
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let deadline = tokio::time::Instant::now() + IO;
    let mut restored = backfill_mark(&sandbox);
    while restored.is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        restored = backfill_mark(&sandbox);
    }
    let rerun = doc_agent_rows(&sandbox);
    j.guard(
        "the walk from rowid 0 reaches the old mark and leaves every row and its first_at/last_at as they were",
        mark.is_some() && restored == mark && rerun == rows,
        json!({
            "markRestored": mark.is_some() && restored == mark,
            "rows": rerun.len(),
            "unchanged": rerun == rows,
        }),
    );

    j.step("archiving the workstream unlinks the session");
    j.call(
        &client,
        Request::ArchiveWorkstream {
            id: ws.clone(),
            skip_scripts: true,
        },
    )
    .await
    .expect("archive_workstream");
    let listed = query(&mut j, &client, ThoughtQuery::default()).await;
    let attribution = |id: &str| {
        let doc = item(&listed, id);
        json!({
            "harness": doc.pointer("/agent/harness"),
            "projectPath": doc.pointer("/agent/projectPath"),
        })
    };
    j.guard(
        "agent.projectPath becomes null while the harness stays",
        attribution(&live_id) == json!({ "harness": "claude", "projectPath": null })
            && attribution(&seeded_id) == json!({ "harness": "codex", "projectPath": null }),
        json!({ "livePlan": attribution(&live_id), "seeded": attribution(&seeded_id) }),
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
