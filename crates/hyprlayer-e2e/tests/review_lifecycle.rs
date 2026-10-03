//! The review workspace's server surface, one block of steps per server phase:
//! the branch-vs-base diff and the two sides of a file, then the persisted
//! review: viewed files, comments and the lines they follow, then the notes an
//! implement session leaves in its sidecar file, then submitting the review and
//! the request-changes round trip.
//!
//! The harness is a stub speaking claude's stream-json; it logs the argv it was
//! spawned with, so the instruction's route into the session is observable.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyprlayer_core::models::review::{ReviewSide, ReviewVerdict};
use hyprlayer_core::pipeline::{review_notes_instruction, review_notes_path};
use hyprlayer_core::runtime::reviews::{
    AddReviewCommentRequest, SetReviewFileViewedRequest, SubmitReviewRequest,
};
use hyprlayer_core::{Harness, SessionEvent, Stage, WorktreeMode};
use hyprlayer_e2e::flows::{create_workstream, onboard_and_import};
use hyprlayer_e2e::{
    events_until, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::{EventStream, RpcClient};
use hyprlayer_server::streams::StreamStartOpts;

const IO: Duration = Duration::from_secs(20);

/// Replies to every user turn and logs what it was given: each spawn's argv to
/// `<sandbox>/argv.log` behind a `--spawn` marker, each stdin line to `harness.log`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
printf '%s\n' "--spawn" "$@" >> "$(dirname "$0")/../argv.log"
sid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) sid="$2"; shift 2 ;;
    *) shift ;;
  esac
done
log="$(dirname "$0")/../harness.log"
printf '{"type":"system","subtype":"init","session_id":"%s","model":"claude-e2e","tools":[]}\n' "$sid"
turn=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  case "$line" in
    *control_response*) continue ;;
  esac
  turn=$((turn + 1))
  printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"E2E_REPLY_%s"}]},"session_id":"%s"}\n' "$turn" "$sid"
  printf '{"type":"result","subtype":"success","is_error":false,"result":"E2E_REPLY_%s","session_id":"%s","num_turns":%s,"total_cost_usd":0.01,"usage":{"input_tokens":11,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":7},"modelUsage":{"claude-e2e":{"contextWindow":200000}}}\n' "$turn" "$sid" "$turn"
done
"#;

const REVIEW_RULE: &str = "/.hyprlayer/review/";

// Lines 1-3 of the sidecar are accepted; `NOTE_PARTIAL` arrives later, unterminated.
const NOTE_NEEDS_CALL: &str = r#"{"path":"src/edit.rs","line":6,"kind":"needs_your_call","body":"Why `b` is 20 is a guess; confirm the constant.","task":"Phase 1: edit"}"#;
const NOTE_OUT_OF_SCOPE: &str = r#"{"path":"src/new.rs","line":2,"kind":"outside_plan_scope","body":"This constant was not in the plan.","task":"Phase 2: new"}"#;
const NOTE_PLAIN: &str =
    r#"{"path":"src/edit.rs","line":7,"kind":"note","body":"Prints the sum."}"#;
const NOTE_PARTIAL: &str = r#"{"path":"src/new.rs","line":1,"kind":"note","body":"Added after the first commit.","task":"Phase 2: new"}"#;
const NOTE_SHORT: &str = r#"{"path":"src/new.rs","line":2,"kind":"note","body":"Short."}"#;
const UNUSABLE_NOTES: [&str; 9] = [
    r#"{"path":"src/edit.rs","line":5,"kind":"note","body":"#,
    r#"{"path":"src/edit.rs","line":5,"kind":"shout","body":"Loud."}"#,
    r#"{"path":"../escape","line":1,"kind":"note","body":"Outside the worktree."}"#,
    r#"{"path":"/etc/passwd","line":1,"kind":"note","body":"Absolute."}"#,
    r#"{"path":"src/edit.rs","line":0,"kind":"note","body":"Lines start at 1."}"#,
    r#"{"path":"src/edit.rs","line":"5","kind":"note","body":"A string line."}"#,
    r#"{"path":"src/edit.rs","line":5,"kind":"note","body":""}"#,
    r#"{"path":"","line":5,"kind":"note","body":"No path."}"#,
    r#"{"path":".hyprlayer/settings.toml","line":1,"kind":"note","body":"Never part of a review."}"#,
];

const EDIT_BASE: &str =
    "fn edit() {\n    let a = 1;\n    let b = 2;\n    println!(\"{}\", a + b);\n}\n";
const EDIT_BRANCH: &str =
    "fn edit() {\n    let a = 1;\n    let b = 20;\n    let c = 3;\n    println!(\"{}\", a + b + c);\n}\n";
const OLD_NAME: &str = "pub fn renamed() -> u32 {\n    let a = 40;\n    let b = 1;\n    let c = 1;\n    let d = 0;\n    let e = 0;\n    a + b + c + d + e\n}\n";
const NEW_NAME: &str = "pub fn renamed() -> u32 {\n    let a = 40;\n    let b = 1;\n    let c = 2;\n    let d = 0;\n    let e = 0;\n    a + b + c + d + e\n}\n";
// Gone and new share no lines with each other or the renamed file, so no detector can pair them.
const GONE: &str =
    "pub fn gone() -> &'static str {\n    \"removed on the branch, uncommitted\"\n}\n";
const NEW: &str =
    "// Added after the first commit, never staged.\npub const ADDED: &str = \"untracked\";\n";
// Moved with a plain `mv`, never staged, then one line edited.
const MV_FROM: &str = "pub struct Span {\n    pub left: i64,\n    pub right: i64,\n}\n\nimpl Span {\n    pub fn width(&self) -> i64 {\n        self.right - self.left\n    }\n}\n";
const MV_TO: &str = "pub struct Span {\n    pub left: i64,\n    pub right: i64,\n}\n\nimpl Span {\n    pub fn width(&self) -> i64 {\n        (self.right - self.left).abs()\n    }\n}\n";
const UPSTREAM: &str = "pub const UPSTREAM: u32 = 1;\n";
const BLOB: &[u8] = b"\x00\x01\x02\xff\x00binary";
// EDIT_BRANCH with three lines above `let c = 3;`, then without that line.
const EDIT_SHIFTED: &str = "fn edit() {\n    // One.\n    // Two.\n    // Three.\n    let a = 1;\n    let b = 20;\n    let c = 3;\n    println!(\"{}\", a + b + c);\n}\n";
const EDIT_DROPPED: &str = "fn edit() {\n    // One.\n    // Two.\n    // Three.\n    let a = 1;\n    let b = 20;\n    println!(\"{}\", a + b);\n}\n";

// The review's last edit: the line below the pending comment's, inside its context.
const EDIT_FIXED: &str = "// Header.\nfn edit() {\n    // One.\n    // Two.\n    // Three.\n    let a = 1;\n    let b = 20;\n    println!(\"{}\", a * b);\n}\n";
const PENDING_BODY: &str = "Name this constant.\n\nIt also needs a doc comment.";
const REQUEST_SUMMARY: &str = "Tighten the constants before this ships.";
// The resolved root and its reply are outdated, so they carry the line they were left on.
const REVIEW_MESSAGE: &str = concat!(
    "Review · Changes requested · 2 / 7 viewed · 6 comments\n",
    "\n",
    "Tighten the constants before this ships.\n",
    "\n",
    "- `src/edit.rs:4` Why 3, and not 30?\n",
    "- `src/edit.rs:4` It balances the sum.\n",
    "- `src/gone.rs:2 (removed line)` Why is this here?\n",
    "- `src/gone.rs:2` Why is this here?\n",
    "- `src/new_name.rs:4 (removed line)` Why is this here?\n",
    "- `src/edit.rs:7` Name this constant.\n",
    "  \n",
    "  It also needs a doc comment.\n",
    "\n",
    "Address each comment, then stop. Files you change go back to unviewed in the review.",
);

/// 49,152 lines of 64 bytes: exactly 3 MiB, past the 2 MiB diff limit.
fn big_file() -> String {
    (0..49_152).map(|n| format!("{n:063}\n")).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_lifecycle() {
    let sandbox = Sandbox::new();
    sandbox.stage_tool("claude", FAKE_CLAUDE);
    let repo = sandbox.fixture_repo(
        "demo",
        &[
            ("README.md", "# demo\n"),
            ("src/edit.rs", EDIT_BASE),
            ("src/gone.rs", GONE),
            ("src/old_name.rs", OLD_NAME),
            ("src/mv_from.rs", MV_FROM),
        ],
    );
    let mut j = Journey::new("review_lifecycle", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let project = onboard_and_import(&mut j, &client, &sandbox, &repo).await;
    let view = create_workstream(&mut j, &client, &project, "Review work").await;
    let id = text(&view, "/item/id");
    let worktree = PathBuf::from(text(&view, "/item/worktreePath"));
    let merge_base = sandbox.git(&worktree, &["rev-parse", "main"]);
    j.alias(&merge_base, "$MERGE_BASE");

    j.step("a fresh branch has nothing to review");
    let fresh = changed_files(&mut j, &client, &id).await;
    j.guard(
        "no files, against the commit the branch started from",
        fresh["files"] == json!([])
            && fresh["baseResolved"] == json!(true)
            && fresh["mergeBase"] == json!(merge_base),
        json!({
            "files": fresh["files"],
            "baseResolved": fresh["baseResolved"],
            "mergeBase": fresh["mergeBase"],
        }),
    );

    j.step("review the branch against its base");
    write(&worktree, "src/edit.rs", EDIT_BRANCH);
    sandbox.git(&worktree, &["add", "src/edit.rs"]);
    sandbox.git(&worktree, &["commit", "--quiet", "--message", "Edit"]);
    sandbox.git(&worktree, &["mv", "src/old_name.rs", "src/new_name.rs"]);
    write(&worktree, "src/new_name.rs", NEW_NAME);
    sandbox.git(&worktree, &["add", "src/new_name.rs"]);
    sandbox.git(&worktree, &["commit", "--quiet", "--message", "Rename"]);
    sandbox.git(&worktree, &["rm", "--quiet", "src/gone.rs"]);
    std::fs::rename(
        worktree.join("src/mv_from.rs"),
        worktree.join("src/mv_to.rs"),
    )
    .expect("move a file");
    write(&worktree, "src/mv_to.rs", MV_TO);
    write(&worktree, "src/new.rs", NEW);
    write(&worktree, "assets/blob.bin", BLOB);
    write(&worktree, "big.txt", big_file());
    write(&worktree, ".hyprlayer/review/x", "scratch\n");

    let list = changed_files(&mut j, &client, &id).await;
    let git_merge_base = sandbox.git(&worktree, &["merge-base", "main", "HEAD"]);
    j.guard(
        "the list names its base and the commit the branch started from",
        list["baseResolved"] == json!(true)
            && list["base"] == json!("main")
            && list["mergeBase"] == json!(git_merge_base)
            && git_merge_base == merge_base,
        json!({
            "base": list["base"],
            "baseResolved": list["baseResolved"],
            "mergeBase": list["mergeBase"],
            "git_merge_base_matches": list["mergeBase"] == json!(git_merge_base),
        }),
    );

    let kinds: Vec<Value> = files(&list)
        .iter()
        .map(|f| json!({ "path": f["path"], "change": f["change"], "oldPath": f["oldPath"] }))
        .collect();
    let expected_kinds = vec![
        json!({ "path": "assets/blob.bin", "change": "added", "oldPath": null }),
        json!({ "path": "big.txt", "change": "added", "oldPath": null }),
        json!({ "path": "src/edit.rs", "change": "modified", "oldPath": null }),
        json!({ "path": "src/gone.rs", "change": "deleted", "oldPath": null }),
        json!({ "path": "src/mv_to.rs", "change": "renamed", "oldPath": "src/mv_from.rs" }),
        json!({ "path": "src/new.rs", "change": "added", "oldPath": null }),
        json!({ "path": "src/new_name.rs", "change": "renamed", "oldPath": "src/old_name.rs" }),
    ];
    j.guard(
        "kinds and paths cover commits and uncommitted work, with each rename's old path",
        kinds == expected_kinds,
        json!({ "files": kinds }),
    );

    let status_before = sandbox.git(&worktree, &["status", "--porcelain"]);
    let untracked = sandbox.git(
        &worktree,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    );
    let untracked: Vec<&str> = untracked.split('\0').filter(|p| !p.is_empty()).collect();
    let unstaged_deletes = sandbox.git(&worktree, &["ls-files", "--deleted", "-z"]);
    let unstaged_deletes = unstaged_deletes.split('\0').filter(|p| !p.is_empty());
    sandbox.git(&worktree, &["add", "-N", "."]);
    let git_numstat =
        numstat(&sandbox.git(&worktree, &["diff", "--numstat", "-z", "-M", &merge_base]));
    // `add -N .` stages unstaged deletes too. Reset only what it touched; `git rm` stays staged.
    let mut restore = vec!["reset", "--quiet", "--"];
    restore.extend(untracked.iter().copied());
    restore.extend(unstaged_deletes);
    sandbox.git(&worktree, &restore);
    let status_after = sandbox.git(&worktree, &["status", "--porcelain"]);

    let leaked: Vec<String> = paths(&list).into_iter().filter(|p| internal(p)).collect();
    j.guard(
        "`.hyprlayer/` stays out of the list, though git would list it",
        leaked.is_empty() && git_numstat.contains_key(".hyprlayer/review/x"),
        json!({
            "leaked": leaked,
            "git_lists_it": git_numstat.contains_key(".hyprlayer/review/x"),
        }),
    );

    let git_text: BTreeMap<String, (u64, u64)> = git_numstat
        .into_iter()
        .filter(|(path, _)| !internal(path))
        .collect();
    let review_text = text_counts(&list);
    j.guard(
        "text-file counts equal git's numstat against the merge-base",
        review_text == git_text,
        json!({ "review": review_text, "git": git_text }),
    );
    j.guard(
        "the numstat check leaves the index as it found it",
        status_before == status_after,
        json!({ "status": status_after.lines().collect::<Vec<_>>() }),
    );

    let mut oids = BTreeMap::new();
    let mut oids_match = true;
    for f in files(&list) {
        let path = text(f, "/path");
        let oid = text(f, "/contentOid");
        let on_disk = worktree.join(&path);
        let want = if !on_disk.exists() {
            "deleted".to_string()
        } else if on_disk.is_file() {
            sandbox.git(&worktree, &["hash-object", &path])
        } else {
            "(not a file)".to_string()
        };
        oids_match &= oid == want;
        oids.insert(path, oid);
    }
    j.guard(
        "contentOid is git's blob hash, and a deleted file says deleted",
        oids_match,
        json!({ "oids": oids }),
    );

    let binary: Vec<String> = files(&list)
        .iter()
        .filter(|f| f["binary"] == json!(true))
        .map(|f| text(f, "/path"))
        .collect();
    let bin = file(&list, "assets/blob.bin");
    j.guard(
        "only the binary file is flagged, with no line counts",
        binary == ["assets/blob.bin"]
            && bin.is_some_and(|f| f["additions"] == json!(0) && f["deletions"] == json!(0)),
        json!({ "binary": binary }),
    );

    let (additions, deletions) = text_counts(&list)
        .values()
        .fold((0, 0), |(a, d), (fa, fd)| (a + fa, d + fd));
    j.guard(
        "the list totals sum the per-file counts",
        list["additions"] == json!(additions) && list["deletions"] == json!(deletions),
        json!({ "additions": list["additions"], "deletions": list["deletions"] }),
    );

    j.step("read both sides of a file");
    let edit = file_sides(&mut j, &client, &id, "src/edit.rs").await;
    j.guard(
        "an edited file returns the base text and the working text",
        edit["original"] == json!(EDIT_BASE)
            && edit["modified"] == json!(EDIT_BRANCH)
            && edit["originalExists"] == json!(true)
            && edit["modifiedExists"] == json!(true)
            && edit["originalPath"].is_null()
            && edit["binary"] == json!(false)
            && edit["tooLarge"] == json!(false)
            && edit["contentOid"] == json!(oids["src/edit.rs"]),
        json!({
            "original_is_base": edit["original"] == json!(EDIT_BASE),
            "modified_is_working": edit["modified"] == json!(EDIT_BRANCH),
            "contentOid": edit["contentOid"],
        }),
    );

    let renamed = file_sides(&mut j, &client, &id, "src/new_name.rs").await;
    j.guard(
        "a rename reads its original from the old path",
        renamed["original"] == json!(OLD_NAME)
            && renamed["modified"] == json!(NEW_NAME)
            && renamed["originalPath"] == json!("src/old_name.rs"),
        json!({ "originalPath": renamed["originalPath"] }),
    );

    let unstaged_mv = file_sides(&mut j, &client, &id, "src/mv_to.rs").await;
    j.guard(
        "an unstaged mv is a rename too, read from its old path",
        unstaged_mv["original"] == json!(MV_FROM)
            && unstaged_mv["modified"] == json!(MV_TO)
            && unstaged_mv["originalPath"] == json!("src/mv_from.rs"),
        json!({ "originalPath": unstaged_mv["originalPath"] }),
    );

    let added = file_sides(&mut j, &client, &id, "src/new.rs").await;
    let deleted = file_sides(&mut j, &client, &id, "src/gone.rs").await;
    j.guard(
        "an added file has no original and a deleted file has no working copy",
        added["originalExists"] == json!(false)
            && added["modifiedExists"] == json!(true)
            && added["modified"] == json!(NEW)
            && deleted["originalExists"] == json!(true)
            && deleted["original"] == json!(GONE)
            && deleted["modifiedExists"] == json!(false)
            && deleted["contentOid"] == json!("deleted"),
        json!({
            "added": { "originalExists": added["originalExists"], "modifiedExists": added["modifiedExists"] },
            "deleted": {
                "originalExists": deleted["originalExists"],
                "modifiedExists": deleted["modifiedExists"],
                "contentOid": deleted["contentOid"],
            },
        }),
    );

    let big = file_sides(&mut j, &client, &id, "big.txt").await;
    j.guard(
        "a file over 2 MiB is flagged too large and returns no content",
        big["tooLarge"] == json!(true)
            && big["modifiedExists"] == json!(true)
            && big["original"] == json!("")
            && big["modified"] == json!(""),
        json!({ "tooLarge": big["tooLarge"], "modifiedExists": big["modifiedExists"] }),
    );

    let blob = file_sides(&mut j, &client, &id, "assets/blob.bin").await;
    j.guard(
        "a binary file is flagged and returns no content",
        blob["binary"] == json!(true)
            && blob["modified"] == json!("")
            && blob["tooLarge"] == json!(false),
        json!({ "binary": blob["binary"], "tooLarge": blob["tooLarge"] }),
    );

    let escape = j
        .call(
            &client,
            Request::ReviewFileSides {
                workstream_id: id.clone(),
                path: "../outside".into(),
            },
        )
        .await;
    j.guard(
        "a path outside the worktree is refused",
        escape.is_err(),
        json!({ "refused": escape.is_err() }),
    );

    j.step("the base moves on after the branch started");
    write(&repo, "src/upstream.rs", UPSTREAM);
    sandbox.git(&repo, &["add", "src/upstream.rs"]);
    sandbox.git(&repo, &["commit", "--quiet", "--message", "Upstream"]);
    let moved = changed_files(&mut j, &client, &id).await;
    j.guard(
        "work that landed on the base after the branch point is not in the review",
        moved["mergeBase"] == json!(merge_base) && paths(&moved) == paths(&list),
        json!({ "mergeBase": moved["mergeBase"], "files": paths(&moved) }),
    );

    j.step("resolve the base the way the branch names it");
    set_base(&mut j, &client, &id, "origin/main").await;
    let remote_named = changed_files(&mut j, &client, &id).await;
    j.guard(
        "an origin/ base that is not fetched falls back to the local branch",
        remote_named["baseResolved"] == json!(true)
            && remote_named["base"] == json!("origin/main")
            && remote_named["mergeBase"] == json!(merge_base)
            && paths(&remote_named) == paths(&list),
        json!({ "base": remote_named["base"], "baseResolved": remote_named["baseResolved"] }),
    );

    set_base(&mut j, &client, &id, "no-such-base").await;
    let unresolved = changed_files(&mut j, &client, &id).await;
    j.guard(
        "an unresolvable base is reported and the diff falls back to HEAD",
        unresolved["baseResolved"] == json!(false)
            && unresolved["mergeBase"].is_null()
            && unresolved["base"] == json!("no-such-base")
            && paths(&unresolved)
                == [
                    "assets/blob.bin",
                    "big.txt",
                    "src/gone.rs",
                    "src/mv_to.rs",
                    "src/new.rs",
                ],
        json!({
            "baseResolved": unresolved["baseResolved"],
            "mergeBase": unresolved["mergeBase"],
            "files": paths(&unresolved),
        }),
    );
    set_base(&mut j, &client, &id, "main").await;

    j.step("open the review");
    let mut events = client.take_events().expect("the client's runtime events");
    let opened = review(&mut j, &client, &id).await;
    let review_id = text(&opened, "/id");
    j.alias(&review_id, "$REVIEW");
    let reopened = review(&mut j, &client, &id).await;
    j.guard(
        "the first read creates the review and the next returns the same one",
        !review_id.is_empty()
            && text(&reopened, "/id") == review_id
            && opened["workstreamId"] == json!(id),
        json!({
            "id": opened["id"],
            "again": reopened["id"],
            "workstreamId": opened["workstreamId"],
        }),
    );
    j.guard(
        "a new review lists the branch's files and holds nothing else",
        paths(&opened["files"]) == paths(&list)
            && opened["status"] == json!("in_progress")
            && opened["viewed"] == json!([])
            && opened["comments"] == json!([])
            && opened["notes"] == json!([])
            && opened["rounds"] == json!([])
            && opened["changedSinceRound"] == json!([]),
        json!({
            "status": opened["status"],
            "files": paths(&opened["files"]),
            "viewed": opened["viewed"],
            "comments": opened["comments"],
            "notes": opened["notes"],
            "rounds": opened["rounds"],
            "changedSinceRound": opened["changedSinceRound"],
        }),
    );

    j.step("mark files viewed");
    let edit_oid = content_oid(&opened, "src/edit.rs");
    set_viewed(&mut j, &client, &id, "src/edit.rs", &edit_oid, true).await;
    let announced = events_until(&mut events, Duration::from_secs(20), |e| {
        review_changed(e, &id)
    })
    .await;
    // Delivery is in order, so a read that had announced would already be here.
    let trailing = events_until(&mut events, Duration::from_secs(1), |_| false).await;
    let announcements = announced
        .iter()
        .chain(&trailing)
        .filter(|e| review_changed(e, &id))
        .count();
    j.guard(
        "a change announces ReviewChanged once, and the reads before it announced nothing",
        announcements == 1,
        json!({ "review_changed": announcements }),
    );
    set_viewed(
        &mut j,
        &client,
        &id,
        "src/new.rs",
        &content_oid(&opened, "src/new.rs"),
        true,
    )
    .await;
    set_viewed(
        &mut j,
        &client,
        &id,
        "big.txt",
        &content_oid(&opened, "big.txt"),
        true,
    )
    .await;
    set_viewed(
        &mut j,
        &client,
        &id,
        "big.txt",
        &content_oid(&opened, "big.txt"),
        false,
    )
    .await;
    let viewed = review(&mut j, &client, &id).await;
    j.guard(
        "files marked viewed read back as viewed, and unmarking one clears it",
        viewed["viewed"] == json!(["src/edit.rs", "src/new.rs"]),
        json!({ "viewed": viewed["viewed"] }),
    );

    j.step("comment on a line, reply, edit and resolve");
    let comment = add_comment(
        &mut j,
        &client,
        comment_on(
            &id,
            "src/edit.rs",
            ReviewSide::Modified,
            4,
            "    let c = 3;",
        ),
    )
    .await
    .expect("add_review_comment");
    let comment_id = text(&comment, "/id");
    j.alias(&comment_id, "$COMMENT");
    let reply = add_comment(
        &mut j,
        &client,
        AddReviewCommentRequest {
            parent_id: Some(comment_id.clone()),
            body: "It balances the sum.".into(),
            ..comment_on(&id, "README.md", ReviewSide::Original, 1, "# demo")
        },
    )
    .await
    .expect("add_review_comment reply");
    let reply_id = text(&reply, "/id");
    j.alias(&reply_id, "$REPLY");
    j.call(
        &client,
        Request::EditReviewComment {
            workstream_id: id.clone(),
            comment_id: comment_id.clone(),
            body: "Why 3, and not 30?".into(),
        },
    )
    .await
    .expect("edit_review_comment");
    j.call(
        &client,
        Request::ResolveReviewComment {
            workstream_id: id.clone(),
            comment_id: comment_id.clone(),
            resolved: true,
        },
    )
    .await
    .expect("resolve_review_comment");
    let threaded = review(&mut j, &client, &id).await;
    let root = comment_in(&threaded, &comment_id);
    let answer = comment_in(&threaded, &reply_id);
    j.guard(
        "a comment reads back on its line, edited, resolved and still pending",
        root["path"] == json!("src/edit.rs")
            && root["side"] == json!("modified")
            && root["line"] == json!(4)
            && root["currentLine"] == json!(4)
            && root["body"] == json!("Why 3, and not 30?")
            && root["resolved"] == json!(true)
            && root["state"] == json!("pending")
            && root["parentId"].is_null(),
        summary(&root),
    );
    j.guard(
        "a reply takes its parent's place, whatever the request said",
        answer["parentId"] == json!(comment_id)
            && answer["path"] == root["path"]
            && answer["side"] == root["side"]
            && answer["line"] == root["line"]
            && answer["anchorText"] == root["anchorText"]
            && answer["resolved"] == json!(false)
            && reply["path"] == root["path"],
        summary(&answer),
    );
    let zero = add_comment(
        &mut j,
        &client,
        comment_on(&id, "src/edit.rs", ReviewSide::Modified, 0, ""),
    )
    .await;
    let orphan = add_comment(
        &mut j,
        &client,
        AddReviewCommentRequest {
            parent_id: Some("no-such-comment".into()),
            ..comment_on(
                &id,
                "src/edit.rs",
                ReviewSide::Modified,
                4,
                "    let c = 3;",
            )
        },
    )
    .await;
    let escape = add_comment(
        &mut j,
        &client,
        comment_on(&id, "../outside", ReviewSide::Modified, 1, ""),
    )
    .await;
    j.guard(
        "line 0, a missing parent and a path outside the worktree are refused",
        code(&zero).is_some() && code(&orphan).is_some() && code(&escape).is_some(),
        json!({ "line_0": code(&zero), "missing_parent": code(&orphan), "outside": code(&escape) }),
    );

    j.step("comment on the base side");
    let mut base_side = BTreeMap::new();
    for (label, path, side, line, anchor) in [
        (
            "deleted file, base side",
            "src/gone.rs",
            ReviewSide::Original,
            2,
            "    \"removed on the branch, uncommitted\"",
        ),
        (
            "deleted file, working side",
            "src/gone.rs",
            ReviewSide::Modified,
            2,
            "    \"removed on the branch, uncommitted\"",
        ),
        (
            "renamed file, base side",
            "src/new_name.rs",
            ReviewSide::Original,
            4,
            "    let c = 1;",
        ),
        (
            "added file, base side",
            "src/new.rs",
            ReviewSide::Original,
            1,
            "// Added after the first commit, never staged.",
        ),
    ] {
        let added = add_comment(&mut j, &client, comment_on(&id, path, side, line, anchor))
            .await
            .expect("add_review_comment on the base side");
        let added_id = text(&added, "/id");
        j.alias(&added_id, format!("$COMMENT_{}", base_side.len() + 1));
        base_side.insert(label, added_id);
    }
    let based = review(&mut j, &client, &id).await;
    let lines: BTreeMap<&str, Value> = base_side
        .iter()
        .map(|(label, cid)| (*label, comment_in(&based, cid)["currentLine"].clone()))
        .collect();
    j.guard(
        "the base side reads the merge-base, at a rename's old path; a side with no file has no line",
        lines["deleted file, base side"] == json!(2)
            && lines["deleted file, working side"].is_null()
            && lines["renamed file, base side"] == json!(4)
            && lines["added file, base side"].is_null(),
        json!({ "currentLine": lines }),
    );

    j.step("the commented file changes");
    write(&worktree, "src/edit.rs", EDIT_SHIFTED);
    let shifted = review(&mut j, &client, &id).await;
    let moved_root = comment_in(&shifted, &comment_id);
    let moved_reply = comment_in(&shifted, &reply_id);
    j.guard(
        "three lines inserted above move the comment and its reply to line 7",
        moved_root["currentLine"] == json!(7)
            && moved_root["line"] == json!(4)
            && moved_reply["currentLine"] == json!(7),
        json!({
            "comment": { "line": moved_root["line"], "currentLine": moved_root["currentLine"] },
            "reply": { "line": moved_reply["line"], "currentLine": moved_reply["currentLine"] },
        }),
    );
    j.guard(
        "the rewritten file is no longer viewed and the untouched one still is",
        shifted["viewed"] == json!(["src/new.rs"]),
        json!({ "viewed": shifted["viewed"] }),
    );
    write(&worktree, "src/edit.rs", EDIT_DROPPED);
    let dropped = review(&mut j, &client, &id).await;
    let lost_root = comment_in(&dropped, &comment_id);
    let lost_reply = comment_in(&dropped, &reply_id);
    j.guard(
        "deleting the commented line leaves the comment and its reply with no line",
        lost_root["currentLine"].is_null()
            && lost_reply["currentLine"].is_null()
            && lost_root["line"] == json!(4),
        json!({
            "comment": lost_root["currentLine"],
            "reply": lost_reply["currentLine"],
        }),
    );

    j.step("delete a comment, and stay within the workstream");
    let doomed = base_side["added file, base side"].clone();
    j.call(
        &client,
        Request::DeleteReviewComment {
            workstream_id: id.clone(),
            comment_id: doomed.clone(),
        },
    )
    .await
    .expect("delete_review_comment");
    let pruned = review(&mut j, &client, &id).await;
    j.guard(
        "a deleted comment is gone and the rest remain",
        pruned["comments"]
            .as_array()
            .is_some_and(|all| all.len() == 5 && all.iter().all(|c| text(c, "/id") != doomed)),
        json!({ "comments": comment_ids(&pruned) }),
    );

    let other = j
        .call(
            &client,
            Request::CreateWorkstream {
                project_path: project.clone(),
                title: "Other work".into(),
                base: None,
                mode: WorktreeMode::Isolated,
                branch: None,
                branch_prefix: None,
            },
        )
        .await
        .expect("create_workstream");
    let other_id = text(&other, "/item/id");
    j.alias(&other_id, "$WS2");
    let other_slug = text(&other, "/item/slug");
    if let Some((stem, _)) = other_slug.rsplit_once('-') {
        j.alias(&other_slug, format!("{stem}-$SUFFIX"));
    }
    let foreign_edit = j
        .call(
            &client,
            Request::EditReviewComment {
                workstream_id: other_id.clone(),
                comment_id: comment_id.clone(),
                body: "Not yours.".into(),
            },
        )
        .await;
    let foreign_resolve = j
        .call(
            &client,
            Request::ResolveReviewComment {
                workstream_id: other_id.clone(),
                comment_id: comment_id.clone(),
                resolved: false,
            },
        )
        .await;
    let foreign_delete = j
        .call(
            &client,
            Request::DeleteReviewComment {
                workstream_id: other_id.clone(),
                comment_id: comment_id.clone(),
            },
        )
        .await;
    let foreign_reply = add_comment(
        &mut j,
        &client,
        AddReviewCommentRequest {
            parent_id: Some(comment_id.clone()),
            ..comment_on(&other_id, "src/edit.rs", ReviewSide::Modified, 1, "")
        },
    )
    .await;
    let missing_note = j
        .call(
            &client,
            Request::ResolveReviewNote {
                workstream_id: id.clone(),
                note_id: "no-such-note".into(),
                resolved: true,
            },
        )
        .await;
    let untouched = review(&mut j, &client, &id).await;
    let kept = comment_in(&untouched, &comment_id);
    j.guard(
        "another workstream cannot edit, resolve, delete or reply to this one's comment",
        code(&foreign_edit).as_deref() == Some("review_comment_not_found")
            && code(&foreign_resolve).as_deref() == Some("review_comment_not_found")
            && code(&foreign_delete).as_deref() == Some("review_comment_not_found")
            && code(&foreign_reply).as_deref() == Some("review_comment_not_found")
            && kept["body"] == json!("Why 3, and not 30?")
            && kept["resolved"] == json!(true),
        json!({
            "edit": code(&foreign_edit),
            "resolve": code(&foreign_resolve),
            "delete": code(&foreign_delete),
            "reply": code(&foreign_reply),
            "comment": summary(&kept),
        }),
    );
    j.guard(
        "resolving a note that is not there is refused",
        code(&missing_note).as_deref() == Some("review_note_not_found"),
        json!({ "code": code(&missing_note) }),
    );

    j.step("exclude the sidecar directory");
    let exclude = PathBuf::from(sandbox.git(
        &worktree,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .join("info/exclude");
    let rules = hyprlayer_rules(&exclude);
    j.guard(
        "reading a review leaves the rule in the common git dir, and never one for .hyprlayer/ itself",
        rules == [REVIEW_RULE],
        json!({ "rules": rules }),
    );
    let kept_rules: String = std::fs::read_to_string(&exclude)
        .unwrap_or_default()
        .lines()
        .filter(|rule| !matches!(rule.trim(), REVIEW_RULE | "# hyprlayer: agent review notes"))
        .map(|rule| format!("{rule}\n"))
        .collect();
    std::fs::write(&exclude, kept_rules).expect("drop the rule");
    let listed = sandbox.git(&worktree, &["status", "--porcelain", "-uall"]);
    j.guard(
        "without the rule git lists the sidecar directory, so the check below can fail",
        hyprlayer_rules(&exclude).is_empty() && listed.contains(".hyprlayer/review/x"),
        json!({ "rules": hyprlayer_rules(&exclude), "git_lists_it": listed.contains(".hyprlayer/review/x") }),
    );

    j.step("an implement session is told to leave notes");
    let instruction = review_notes_instruction(&id);
    let sidecar_path = review_notes_path(&id);
    let (implement, mut implement_events) = launch_session(
        &mut j,
        &client,
        &id,
        Stage::Implement,
        "thoughts/shared/plans/e2e.md",
    )
    .await;
    j.alias(&implement, "$IMPLEMENT");
    let rules = hyprlayer_rules(&exclude);
    j.guard(
        "starting the session writes the rule before any review is read",
        rules == [REVIEW_RULE],
        json!({ "rules": rules }),
    );
    let appended = spawn_of(&sandbox, &implement).and_then(|args| {
        let flag = args.iter().position(|a| a == "--append-system-prompt")?;
        args.get(flag + 1).cloned()
    });
    j.guard(
        "claude is spawned with the instruction as its appended system prompt, naming the sidecar",
        appended.as_deref() == Some(instruction.as_str()) && instruction.contains(&sidecar_path),
        json!({
            "flag_present": appended.is_some(),
            "is_the_instruction": appended.as_deref() == Some(instruction.as_str()),
            "sidecar": sidecar_path,
        }),
    );
    let turn = first_user_turn(&sandbox, &implement);
    j.guard(
        "the first user turn is still the bare skill invocation",
        turn.starts_with("/implement_plan")
            && !turn.contains(&instruction)
            && !turn.contains(".hyprlayer/review"),
        json!({ "turn": turn }),
    );
    let (general, mut general_events) =
        launch_session(&mut j, &client, &id, Stage::General, "E2E_PROMPT_GENERAL").await;
    j.alias(&general, "$GENERAL");
    let other = spawn_of(&sandbox, &general);
    j.guard(
        "a session for any other stage gets no instruction",
        other
            .as_ref()
            .is_some_and(|args| !args.iter().any(|a| a == "--append-system-prompt")),
        json!({ "spawned": other.is_some() }),
    );
    let status = sandbox.git(&worktree, &["status", "--porcelain", "-uall"]);
    j.guard(
        "git status does not list the sidecar directory",
        !status.contains(".hyprlayer/review"),
        json!({ "lists_it": status.contains(".hyprlayer/review") }),
    );

    j.step("read the notes the agent left");
    let sidecar = worktree.join(&sidecar_path);
    let mut lines = vec![
        NOTE_NEEDS_CALL,
        UNUSABLE_NOTES[0],
        NOTE_OUT_OF_SCOPE,
        UNUSABLE_NOTES[1],
        UNUSABLE_NOTES[2],
        NOTE_PLAIN,
        UNUSABLE_NOTES[3],
        UNUSABLE_NOTES[4],
        UNUSABLE_NOTES[5],
        UNUSABLE_NOTES[6],
        UNUSABLE_NOTES[7],
        UNUSABLE_NOTES[8],
        NOTE_NEEDS_CALL,
    ]
    .join("\n");
    lines.push('\n');
    lines.push_str(NOTE_PARTIAL);
    write(&worktree, &sidecar_path, &lines);
    let first = review(&mut j, &client, &id).await;
    let expected = vec![
        json!({ "path": "src/edit.rs", "line": 6, "currentLine": 6, "kind": "needs_your_call",
            "body": "Why `b` is 20 is a guess; confirm the constant.", "task": "Phase 1: edit", "resolved": false }),
        json!({ "path": "src/new.rs", "line": 2, "currentLine": 2, "kind": "outside_plan_scope",
            "body": "This constant was not in the plan.", "task": "Phase 2: new", "resolved": false }),
        json!({ "path": "src/edit.rs", "line": 7, "currentLine": 7, "kind": "note",
            "body": "Prints the sum.", "task": null, "resolved": false }),
    ];
    j.guard(
        "three valid lines become notes; malformed, unknown-kind, escaping, empty, line 0 and excluded ones, a duplicate and an unterminated line do not",
        notes(&first) == expected,
        json!({ "notes": notes(&first) }),
    );

    let mut tail = std::fs::OpenOptions::new()
        .append(true)
        .open(&sidecar)
        .expect("open the sidecar");
    std::io::Write::write_all(&mut tail, b"\n").expect("finish the line");
    let finished = review(&mut j, &client, &id).await;
    let mut with_partial = expected.clone();
    with_partial.push(
        json!({ "path": "src/new.rs", "line": 1, "currentLine": 1, "kind": "note",
        "body": "Added after the first commit.", "task": "Phase 2: new", "resolved": false }),
    );
    j.guard(
        "finishing the line adds that note",
        notes(&finished) == with_partial,
        json!({ "notes": notes(&finished).len(), "last": notes(&finished).last() }),
    );
    let again = review(&mut j, &client, &id).await;
    j.guard(
        "reading again adds nothing",
        notes(&again) == with_partial,
        json!({ "notes": notes(&again).len() }),
    );

    write(
        &worktree,
        "src/edit.rs",
        format!("// Header.\n{EDIT_DROPPED}"),
    );
    let shifted = review(&mut j, &client, &id).await;
    let shifted_lines: Vec<Value> = shifted["notes"]
        .as_array()
        .map(|all| all.iter().map(|n| n["currentLine"].clone()).collect())
        .unwrap_or_default();
    j.guard(
        "a note follows its line like a comment does",
        shifted_lines == [json!(7), json!(2), json!(8), json!(1)],
        json!({ "currentLine": shifted_lines }),
    );

    let before_len = std::fs::metadata(&sidecar).map(|m| m.len()).unwrap_or(0);
    write(&worktree, &sidecar_path, format!("{NOTE_SHORT}\n"));
    let after_len = std::fs::metadata(&sidecar).map(|m| m.len()).unwrap_or(0);
    let recreated = review(&mut j, &client, &id).await;
    let replayed = review(&mut j, &client, &id).await;
    j.guard(
        "a recreated, shorter file is read from its start",
        after_len < before_len
            && notes(&recreated).len() == 5
            && notes(&recreated)
                .last()
                .is_some_and(|n| n["body"] == json!("Short."))
            && notes(&replayed).len() == 5,
        json!({
            "shorter": after_len < before_len,
            "notes": notes(&recreated).len(),
            "last": notes(&recreated).last(),
            "after_reading_again": notes(&replayed).len(),
        }),
    );

    kill_session(&mut j, &client, &implement, &mut implement_events).await;
    kill_session(&mut j, &client, &general, &mut general_events).await;

    j.step("restart the server");
    let before = review(&mut j, &client, &id).await;
    drop(events);
    let stopped = server.shutdown(&client).await;
    j.guard(
        "the first server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let after = review(&mut j, &client, &id).await;
    let kept_comments: Vec<Value> = comments(&after).into_iter().map(summary).collect();
    let had_comments: Vec<Value> = comments(&before).into_iter().map(summary).collect();
    j.guard(
        "comments and viewed files survive a restart",
        text(&after, "/id") == review_id
            && after["viewed"] == before["viewed"]
            && kept_comments == had_comments
            && kept_comments.len() == 5,
        json!({ "id": after["id"], "viewed": after["viewed"], "comments": kept_comments }),
    );

    j.step("notes survive a restart");
    let reread = review(&mut j, &client, &id).await;
    let reread_again = review(&mut j, &client, &id).await;
    j.guard(
        "the stored offset keeps the file from being ingested twice",
        notes(&reread) == notes(&before)
            && notes(&reread_again) == notes(&before)
            && notes(&before).len() == 5,
        json!({ "before": notes(&before).len(), "after": notes(&reread).len(), "again": notes(&reread_again).len() }),
    );

    j.step("save the comments with a Comment verdict");
    let saved = submit(&mut j, &client, &id, ReviewVerdict::Comment, None)
        .await
        .expect("submit_review comment");
    let after_comment = review(&mut j, &client, &id).await;
    let states: Vec<Value> = comments(&after_comment)
        .into_iter()
        .map(|c| json!({ "id": c["id"], "state": c["state"], "round": c["round"] }))
        .collect();
    j.guard(
        "a Comment verdict saves every pending comment on round 1 and sends nothing",
        saved["message"].is_null()
            && saved["round"]["number"] == json!(1)
            && saved["round"]["verdict"] == json!("comment")
            && saved["round"]["commentCount"] == json!(5)
            && states.len() == 5
            && comments(&after_comment)
                .iter()
                .all(|c| c["state"] == json!("saved") && c["round"] == json!(1))
            && after_comment["status"] == json!("in_progress"),
        json!({ "round": round_summary(&saved["round"]), "comments": states, "status": after_comment["status"] }),
    );

    j.step("request changes");
    let pending = add_comment(
        &mut j,
        &client,
        AddReviewCommentRequest {
            body: PENDING_BODY.into(),
            ..comment_on(
                &id,
                "src/edit.rs",
                ReviewSide::Modified,
                7,
                "    let b = 20;",
            )
        },
    )
    .await
    .expect("add_review_comment before requesting changes");
    let pending_id = text(&pending, "/id");
    j.alias(&pending_id, "$PENDING");
    set_viewed(
        &mut j,
        &client,
        &id,
        "src/edit.rs",
        &content_oid(&after_comment, "src/edit.rs"),
        true,
    )
    .await;
    let requested = submit(
        &mut j,
        &client,
        &id,
        ReviewVerdict::RequestChanges,
        Some(REQUEST_SUMMARY),
    )
    .await
    .expect("submit_review request_changes");
    j.guard(
        "the message follows the review grammar",
        requested["message"] == json!(REVIEW_MESSAGE),
        json!({ "message": requested["message"] }),
    );
    let sending = review(&mut j, &client, &id).await;
    let states: Vec<Value> = comments(&sending)
        .into_iter()
        .map(|c| json!({ "id": c["id"], "state": c["state"], "round": c["round"] }))
        .collect();
    let latest = sending["rounds"]
        .as_array()
        .and_then(|r| r.last())
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "saved and pending comments are sending on round 2, undelivered, and the review is not yet changes requested",
        comments(&sending)
            .iter()
            .all(|c| c["state"] == json!("sending") && c["round"] == json!(2))
            && states.len() == 6
            && latest["number"] == json!(2)
            && latest["verdict"] == json!("request_changes")
            && latest["delivered"] == json!(false)
            && latest["message"] == json!(REVIEW_MESSAGE)
            && latest["commentCount"] == json!(6)
            && sending["status"] == json!("in_progress"),
        json!({ "round": round_summary(&latest), "comments": states, "status": sending["status"] }),
    );
    let refused = submit(&mut j, &client, &id, ReviewVerdict::Approve, None).await;
    j.guard(
        "a round still waiting on delivery refuses the next submit",
        code(&refused).as_deref() == Some("review_round_undelivered"),
        json!({ "code": code(&refused) }),
    );

    j.step("deliver the review to the implement session");
    let (fixer, mut fixer_events) = launch_session(
        &mut j,
        &client,
        &id,
        Stage::Implement,
        "thoughts/shared/plans/e2e.md",
    )
    .await;
    j.alias(&fixer, "$IMPLEMENT_2");
    j.call(
        &client,
        Request::SendUserMessage {
            session_id: fixer.clone(),
            text: REVIEW_MESSAGE.into(),
            attached_docs: vec![],
            prompt_name: Some("hyprlayer:review".into()),
            client_message_id: None,
            base_rev: None,
        },
    )
    .await
    .expect("send_user_message");
    drain_turn(&mut fixer_events).await;
    let turns = user_turns(&sandbox, &fixer);
    j.guard(
        "the harness receives the message as the session's next turn",
        turns.last().map(String::as_str) == Some(REVIEW_MESSAGE),
        json!({ "turns": turns.len(), "last_is_the_message": turns.last().map(String::as_str) == Some(REVIEW_MESSAGE) }),
    );
    mark_delivered(&mut j, &client, &id, 2)
        .await
        .expect("mark_review_round_delivered");
    let again = mark_delivered(&mut j, &client, &id, 2).await;
    let delivered = review(&mut j, &client, &id).await;
    let latest = delivered["rounds"]
        .as_array()
        .and_then(|r| r.last())
        .cloned()
        .unwrap_or(Value::Null);
    j.guard(
        "marking the round delivered sends its comments and requests changes, and marking it again is harmless",
        again.is_ok()
            && comments(&delivered)
                .iter()
                .all(|c| c["state"] == json!("sent"))
            && latest["delivered"] == json!(true)
            && delivered["status"] == json!("changes_requested"),
        json!({
            "again": again.is_ok(),
            "states": comments(&delivered).iter().map(|c| c["state"].clone()).collect::<Vec<_>>(),
            "delivered": latest["delivered"],
            "status": delivered["status"],
        }),
    );
    let missing = mark_delivered(&mut j, &client, &id, 9).await;
    let not_requested = mark_delivered(&mut j, &client, &id, 1).await;
    j.guard(
        "only a change request round that exists can be marked delivered",
        code(&missing).as_deref() == Some("review_round_not_found")
            && code(&not_requested).as_deref() == Some("review_round_not_found"),
        json!({ "missing": code(&missing), "comment_round": code(&not_requested) }),
    );
    j.guard(
        "nothing has changed since the review yet",
        delivered["changedSinceRound"] == json!([])
            && comments(&delivered)
                .iter()
                .all(|c| c["addressed"] == json!(false)),
        json!({ "changedSinceRound": delivered["changedSinceRound"] }),
    );

    j.step("the agent fixes the commented code");
    write(&worktree, "src/edit.rs", EDIT_FIXED);
    let fixed = review(&mut j, &client, &id).await;
    let addressed: BTreeMap<String, Value> = comments(&fixed)
        .into_iter()
        .map(|c| (text(c, "/id"), c["addressed"].clone()))
        .collect();
    let fixed_pending = comment_in(&fixed, &pending_id);
    j.guard(
        "the touched file is changed since the review and no longer viewed",
        fixed["changedSinceRound"] == json!(["src/edit.rs"])
            && !fixed["viewed"]
                .as_array()
                .is_some_and(|v| v.contains(&json!("src/edit.rs"))),
        json!({ "changedSinceRound": fixed["changedSinceRound"], "viewed": fixed["viewed"] }),
    );
    j.guard(
        "comments in the touched file are addressed, whether their lines vanished or changed around them",
        fixed_pending["currentLine"] == json!(7)
            && addressed[&pending_id] == json!(true)
            && addressed[&comment_id] == json!(true)
            && addressed[&reply_id] == json!(true)
            && addressed[&base_side["deleted file, base side"]] == json!(false)
            && addressed[&base_side["deleted file, working side"]] == json!(false)
            && addressed[&base_side["renamed file, base side"]] == json!(false),
        json!({ "addressed": addressed, "pendingLine": fixed_pending["currentLine"] }),
    );

    j.step("approve");
    let approved = submit(&mut j, &client, &id, ReviewVerdict::Approve, None)
        .await
        .expect("submit_review approve");
    let done = review(&mut j, &client, &id).await;
    j.guard(
        "approving after a delivered round marks the review approved",
        approved["round"]["number"] == json!(3)
            && approved["round"]["verdict"] == json!("approve")
            && approved["message"].is_null()
            && done["status"] == json!("approved"),
        json!({ "round": round_summary(&approved["round"]), "status": done["status"] }),
    );
    kill_session(&mut j, &client, &fixer, &mut fixer_events).await;

    j.step("shutdown");
    let stopped = server.shutdown(&client).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

async fn changed_files(j: &mut Journey, client: &RpcClient, id: &str) -> Value {
    j.call(
        client,
        Request::ReviewChangedFiles {
            workstream_id: id.to_string(),
        },
    )
    .await
    .expect("review_changed_files")
}

async fn file_sides(j: &mut Journey, client: &RpcClient, id: &str, path: &str) -> Value {
    j.call(
        client,
        Request::ReviewFileSides {
            workstream_id: id.to_string(),
            path: path.to_string(),
        },
    )
    .await
    .expect("review_file_sides")
}

async fn review(j: &mut Journey, client: &RpcClient, id: &str) -> Value {
    j.call(
        client,
        Request::GetReview {
            workstream_id: id.to_string(),
        },
    )
    .await
    .expect("get_review")
}

async fn set_viewed(
    j: &mut Journey,
    client: &RpcClient,
    id: &str,
    path: &str,
    content_oid: &str,
    viewed: bool,
) {
    j.call(
        client,
        Request::SetReviewFileViewed {
            req: SetReviewFileViewedRequest {
                workstream_id: id.to_string(),
                path: path.to_string(),
                content_oid: content_oid.to_string(),
                viewed,
            },
        },
    )
    .await
    .expect("set_review_file_viewed");
}

async fn add_comment(
    j: &mut Journey,
    client: &RpcClient,
    req: AddReviewCommentRequest,
) -> Result<Value, WireError> {
    j.call(client, Request::AddReviewComment { req }).await
}

async fn submit(
    j: &mut Journey,
    client: &RpcClient,
    id: &str,
    verdict: ReviewVerdict,
    summary: Option<&str>,
) -> Result<Value, WireError> {
    j.call(
        client,
        Request::SubmitReview {
            req: SubmitReviewRequest {
                workstream_id: id.to_string(),
                verdict,
                summary: summary.map(str::to_string),
            },
        },
    )
    .await
}

async fn mark_delivered(
    j: &mut Journey,
    client: &RpcClient,
    id: &str,
    round: u32,
) -> Result<Value, WireError> {
    j.call(
        client,
        Request::MarkReviewRoundDelivered {
            workstream_id: id.to_string(),
            round,
        },
    )
    .await
}

/// A round without its timestamp and message.
fn round_summary(round: &Value) -> Value {
    json!({
        "number": round["number"],
        "verdict": round["verdict"],
        "summary": round["summary"],
        "viewedCount": round["viewedCount"],
        "fileCount": round["fileCount"],
        "commentCount": round["commentCount"],
        "delivered": round["delivered"],
    })
}

fn comment_on(
    id: &str,
    path: &str,
    side: ReviewSide,
    line: u32,
    anchor: &str,
) -> AddReviewCommentRequest {
    AddReviewCommentRequest {
        workstream_id: id.to_string(),
        path: path.to_string(),
        side,
        line,
        anchor_text: anchor.to_string(),
        body: "Why is this here?".to_string(),
        parent_id: None,
    }
}

fn review_changed(event: &RuntimeEvent, id: &str) -> bool {
    matches!(event, RuntimeEvent::ReviewChanged { workstream_id } if workstream_id == id)
}

fn content_oid(review: &Value, path: &str) -> String {
    file(&review["files"], path)
        .map(|f| text(f, "/contentOid"))
        .unwrap_or_default()
}

fn comments(review: &Value) -> Vec<&Value> {
    review["comments"]
        .as_array()
        .map(|all| all.iter().collect())
        .unwrap_or_default()
}

fn comment_in(review: &Value, id: &str) -> Value {
    comments(review)
        .into_iter()
        .find(|c| text(c, "/id") == id)
        .cloned()
        .unwrap_or(Value::Null)
}

fn comment_ids(review: &Value) -> Vec<String> {
    comments(review)
        .into_iter()
        .map(|c| text(c, "/id"))
        .collect()
}

/// A comment without its timestamps.
fn summary(comment: &Value) -> Value {
    json!({
        "id": comment["id"],
        "parentId": comment["parentId"],
        "path": comment["path"],
        "side": comment["side"],
        "line": comment["line"],
        "currentLine": comment["currentLine"],
        "body": comment["body"],
        "state": comment["state"],
        "resolved": comment["resolved"],
    })
}

fn code(result: &Result<Value, WireError>) -> Option<String> {
    result.as_ref().err().map(|e| e.code.clone())
}

async fn set_base(j: &mut Journey, client: &RpcClient, id: &str, base: &str) {
    j.call(
        client,
        Request::SetWorkstreamBaseBranch {
            id: id.to_string(),
            base: base.to_string(),
        },
    )
    .await
    .expect("set_workstream_base_branch");
}

fn write(worktree: &Path, rel: &str, content: impl AsRef<[u8]>) {
    let path = worktree.join(rel);
    std::fs::create_dir_all(path.parent().expect("a file has a parent")).expect("create a dir");
    std::fs::write(path, content).expect("write a file");
}

fn files(list: &Value) -> Vec<&Value> {
    list["files"]
        .as_array()
        .map(|files| files.iter().collect())
        .unwrap_or_default()
}

fn file<'a>(list: &'a Value, path: &str) -> Option<&'a Value> {
    files(list).into_iter().find(|f| text(f, "/path") == path)
}

fn paths(list: &Value) -> Vec<String> {
    files(list).iter().map(|f| text(f, "/path")).collect()
}

/// Additions and deletions per text file; binary files carry none.
fn text_counts(list: &Value) -> BTreeMap<String, (u64, u64)> {
    files(list)
        .into_iter()
        .filter(|f| f["binary"] != json!(true))
        .map(|f| {
            let count = |key: &str| f[key].as_u64().unwrap_or(0);
            (text(f, "/path"), (count("additions"), count("deletions")))
        })
        .collect()
}

fn internal(path: &str) -> bool {
    [".hyprlayer", "thoughts"]
        .iter()
        .any(|dir| path == *dir || path.starts_with(&format!("{dir}/")))
}

/// `git diff --numstat -z`: `add\tdel\tpath\0`, or `add\tdel\t\0old\0new\0` for
/// a rename. Binary rows count `-`, and are left out.
fn numstat(raw: &str) -> BTreeMap<String, (u64, u64)> {
    let mut fields = raw.split('\0');
    let mut rows = BTreeMap::new();
    while let Some(record) = fields.next() {
        let mut parts = record.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            fields.next();
            fields.next().unwrap_or_default()
        } else {
            path
        };
        if let (Ok(added), Ok(deleted)) = (added.parse(), deleted.parse()) {
            rows.insert(path.to_string(), (added, deleted));
        }
    }
    rows
}

/// Everything up to and including the turn's `TurnDone`.
async fn drain_turn(events: &mut EventStream<SessionEvent>) -> Vec<SessionEvent> {
    recv_until(events, |e| matches!(e, SessionEvent::TurnDone { .. })).await
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

/// A launched stage session, its first turn drained.
async fn launch_session(
    j: &mut Journey,
    client: &RpcClient,
    id: &str,
    stage: Stage,
    input: &str,
) -> (String, EventStream<SessionEvent>) {
    let (session, _prompt, _stream_id, mut events) = client
        .start_stage_session_stream(
            id.to_string(),
            stage,
            input.to_string(),
            vec![],
            StreamStartOpts {
                harness: Some(Harness::Claude),
                ..Default::default()
            },
        )
        .await
        .expect("start_stage_session");
    j.call(
        client,
        Request::LaunchAndConverse {
            session_id: session.id.clone(),
        },
    )
    .await
    .expect("launch_and_converse");
    drain_turn(&mut events).await;
    (session.id, events)
}

async fn kill_session(
    j: &mut Journey,
    client: &RpcClient,
    session_id: &str,
    events: &mut EventStream<SessionEvent>,
) {
    j.call(
        client,
        Request::KillSession {
            session_id: session_id.to_string(),
        },
    )
    .await
    .expect("kill_session");
    recv_until(events, |e| matches!(e, SessionEvent::Exited { .. })).await;
}

/// The argv a session's harness was spawned with, found by its `--session-id`.
fn spawn_of(sandbox: &Sandbox, session_id: &str) -> Option<Vec<String>> {
    let raw = std::fs::read_to_string(sandbox.root.join("argv.log")).unwrap_or_default();
    raw.split("--spawn\n")
        .map(|spawn| spawn.lines().map(str::to_string).collect::<Vec<_>>())
        .find(|args| {
            args.windows(2)
                .any(|w| w[0] == "--session-id" && w[1] == session_id)
        })
}

/// The text of the first user turn the harness received for a session.
fn first_user_turn(sandbox: &Sandbox, session_id: &str) -> String {
    std::fs::read_to_string(sandbox.root.join("harness.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["type"] == json!("user") && line["session_id"] == json!(session_id))
        .map(|line| text(&line, "/message/content"))
        .unwrap_or_default()
}

/// Every user turn the harness received for a session, in order.
fn user_turns(sandbox: &Sandbox, session_id: &str) -> Vec<String> {
    std::fs::read_to_string(sandbox.root.join("harness.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|line| line["type"] == json!("user") && line["session_id"] == json!(session_id))
        .map(|line| text(&line, "/message/content"))
        .collect()
}

/// The `.hyprlayer` rules in the exclude file, comments aside.
fn hyprlayer_rules(exclude: &Path) -> Vec<String> {
    std::fs::read_to_string(exclude)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|rule| !rule.starts_with('#') && rule.contains(".hyprlayer"))
        .map(str::to_string)
        .collect()
}

/// A note without its id and timestamp.
fn note_summary(note: &Value) -> Value {
    json!({
        "path": note["path"],
        "line": note["line"],
        "currentLine": note["currentLine"],
        "kind": note["kind"],
        "body": note["body"],
        "task": note["task"],
        "resolved": note["resolved"],
    })
}

fn notes(review: &Value) -> Vec<Value> {
    review["notes"]
        .as_array()
        .map(|all| all.iter().map(note_summary).collect())
        .unwrap_or_default()
}
