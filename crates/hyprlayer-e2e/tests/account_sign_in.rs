//! Cloud sign-in end to end against stub Cognito and API servers. The test plays the
//! desktop (core's real loopback) and the browser; the server keeps the PKCE verifier,
//! the tokens and `account.json`, and only announced clients hear `AccountChanged`.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hyprlayer_core::account::loopback::Loopback;
use hyprlayer_e2e::{
    events_until, eventually, json, text, Journey, Request, RuntimeEvent, Sandbox, Server, Value,
};
use hyprlayer_proto::WireError;
use hyprlayer_server::client::RpcClient;
use hyprlayer_server::events::TARGETED_EVENTS;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

const IO: Duration = Duration::from_secs(10);
/// Long enough for a fanned-out event to land on every connection that would get it.
const QUIET: Duration = Duration::from_millis(500);
/// What released desktops 0.8.6–0.8.7 announce.
const SESSION_KINDS: [&str; 3] = ["SessionRevised", "SessionReset", "SessionViewers"];
const TOKEN_PREFIXES: [&str; 3] = ["e2e-access-", "e2e-refresh-", "e2e-id-"];
const CLIENT_ID: &str = "e2e-client";
const CODE: &str = "e2e-code-1";
/// Code exchanges and accepted refreshes the scenario makes, in order: sign-in, start-up
/// refresh, sign-in, refresh, sign-in, start-up refresh, sign-in.
const TOKENS_ISSUED: u32 = 7;
const SESSION: &str = r#"{"author":{"id":"octo@hyprlayer.invalid","name":"Octo Cat","email":"octo@hyprlayer.invalid","kind":"user"},"teamName":null,"capabilities":{}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn account_sign_in() {
    let stub = Stub::start().await;
    let mut sandbox = Sandbox::new();
    sandbox.set_env("HYPRLAYER_CLOUD_LOGIN_URL", stub.base.clone());
    sandbox.set_env("HYPRLAYER_CLOUD_API_URL", stub.base.clone());
    sandbox.set_env("HYPRLAYER_CLOUD_CLIENT_ID", CLIENT_ID);
    let account_file = sandbox.app_config.join("account.json");
    let mut j = Journey::new("account_sign_in", &sandbox);
    j.alias(stub.base.clone(), "$CLOUD");
    let mut answers = Vec::new();
    let server = Server::start(&sandbox).await;
    let a = server.client();
    let b = server.connect().await;
    let c = server.connect().await;
    let mut a_events = a.take_events().expect("A's runtime events");
    let mut b_events = b.take_events().expect("B's runtime events");
    let mut c_events = c.take_events().expect("C's runtime events");
    call(
        &mut j,
        &a,
        &mut answers,
        announce("client-a", &TARGETED_EVENTS),
    )
    .await
    .expect("A announces every targeted kind");
    call(
        &mut j,
        &b,
        &mut answers,
        announce("client-b", &SESSION_KINDS),
    )
    .await
    .expect("B announces the session kinds");
    let signed_in =
        json!({ "state": "signedIn", "name": "Octo Cat", "email": "octo@hyprlayer.invalid" });

    j.step("signed out before any sign-in");
    let status = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    j.guard(
        "status reads signed out and there is no account file",
        status.as_ref().ok() == Some(&json!({ "state": "signedOut" })) && !account_file.exists(),
        json!({ "status": outcome(&status), "fileExists": account_file.exists() }),
    );

    j.step("a file from another pool reads as signed out");
    let foreign = serde_json::to_vec_pretty(&json!({
        "clientId": "another-pool",
        "loginUrl": "https://login.another.invalid",
        "accessToken": "foreign-access",
        "refreshToken": "foreign-refresh",
        "expiresAt": 4_102_444_800u64,
        "name": "Someone Else",
        "email": null,
    }))
    .expect("encode a foreign account");
    std::fs::write(&account_file, &foreign).expect("write a foreign account");
    let status = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    let unchanged = std::fs::read(&account_file).is_ok_and(|bytes| bytes == foreign);
    j.guard(
        "it reads as signed out and is left byte for byte",
        status.as_ref().ok() == Some(&json!({ "state": "signedOut" })) && unchanged,
        json!({ "status": outcome(&status), "unchanged": unchanged }),
    );

    j.step("a port held on ::1 alone is not free");
    let held = std::net::TcpListener::bind("[::1]:0").expect("hold a port on ::1");
    let held_port = held.local_addr().expect("the held address").port();
    let skipped = Loopback::bind_first(&[held_port, 0])
        .await
        .is_ok_and(|other| other.port() != held_port);
    drop(held);
    j.guard(
        "the loopback passes over a port another listener has on ::1",
        skipped,
        json!({ "skipped": skipped }),
    );

    j.step("start keeps the verifier");
    let loopback = Loopback::bind_first(&[0]).await.expect("bind the loopback");
    let loopback_base = format!("http://localhost:{}", loopback.port());
    j.alias(loopback_base.clone(), "$LOOPBACK");
    let start = call(
        &mut j,
        &a,
        &mut answers,
        Request::AccountStartSignIn {
            port: loopback.port(),
        },
    )
    .await
    .expect("account_start_sign_in");
    let state = text(&start, "/state");
    j.alias(state.clone(), "$STATE");
    let authorize_url = text(&start, "/authorize_url");
    let logout_url = text(&start, "/logout_url");
    let mut authorize = query(&authorize_url);
    let challenge = authorize.remove("code_challenge").unwrap_or_default();
    let wanted_authorize: BTreeMap<String, String> = [
        ("client_id", CLIENT_ID.to_string()),
        ("response_type", "code".into()),
        ("scope", "openid email profile".into()),
        ("redirect_uri", format!("{loopback_base}/auth/callback")),
        ("code_challenge_method", "S256".into()),
        ("state", state.clone()),
        ("identity_provider", "GitHubOidc".into()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();
    j.guard(
        "authorize asks GitHub for a PKCE S256 code back at the loopback",
        location(&authorize_url) == format!("{}/oauth2/authorize", stub.base)
            && authorize == wanted_authorize
            && challenge.len() == 43,
        json!({
            "at": location(&authorize_url),
            "params": authorize,
            "challengeLength": challenge.len(),
        }),
    );
    let logout = query(&logout_url);
    j.guard(
        "logout returns to the loopback's loggedout path",
        location(&logout_url) == format!("{}/logout", stub.base)
            && logout.get("client_id").map(String::as_str) == Some(CLIENT_ID)
            && logout.get("logout_uri") == Some(&format!("{loopback_base}/auth/loggedout"))
            && logout.len() == 2,
        json!({ "at": location(&logout_url), "params": logout }),
    );
    let keys: Vec<String> = start
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    j.guard(
        "the start answer carries the state and the two URLs, and no verifier",
        keys == ["authorize_url", "logout_url", "state"],
        json!({ "keys": keys }),
    );

    j.step("the browser follows logout → loggedout → authorize → callback");
    let cancel = Notify::new();
    let walk = spawn_browse(&logout_url, &loopback_base);
    let mut callback = loopback
        .wait(IO, &state, &authorize_url, &cancel)
        .await
        .expect("the loopback hears the callback");
    drop(loopback);
    let code = callback.code.clone().unwrap_or_default();
    tokio::time::sleep(QUIET).await;
    let waiting = !walk.is_finished();
    j.guard(
        "the code arrives while the browser still waits on the callback",
        code == CODE && waiting,
        json!({ "codeArrived": code == CODE, "browserWaiting": waiting }),
    );

    j.step("finish rejects a foreign state and keeps the attempt");
    let foreign_finish = call(
        &mut j,
        &a,
        &mut answers,
        Request::AccountFinishSignIn {
            state: "not-the-state".into(),
            code: code.clone(),
        },
    )
    .await;
    let token_requests = stub.seen("POST", "/oauth2/token").len();
    j.guard(
        "a foreign state is stale and nothing is exchanged",
        code_of(&foreign_finish).as_deref() == Some("account_sign_in_stale") && token_requests == 0,
        json!({ "code": code_of(&foreign_finish), "tokenRequests": token_requests }),
    );

    j.step("finish exchanges with PKCE S256");
    let finished = call(
        &mut j,
        &a,
        &mut answers,
        Request::AccountFinishSignIn {
            state: state.clone(),
            code: code.clone(),
        },
    )
    .await;
    j.guard(
        "the real state still finishes, as the session's identity",
        finished.as_ref().ok() == Some(&signed_in),
        json!({ "result": outcome(&finished) }),
    );
    callback.reply(finished.is_ok()).await;
    let walk = walk.await.expect("the browser walk");
    let wanted_hops = [
        format!("{}/logout", stub.base),
        format!("{loopback_base}/auth/loggedout"),
        format!("{}/oauth2/authorize", stub.base),
        format!("{loopback_base}/auth/callback"),
        SUCCESS_PAGE.to_string(),
    ];
    j.guard(
        "after the exchange, the hops are logout, loggedout, authorize, callback, then the website",
        walk.hops == wanted_hops,
        json!({ "hops": walk.hops }),
    );
    let landed = walk.status == 302 && walk.page.is_empty();
    j.guard(
        "a forged callback and a stray probe are turned away, and the real one redirects",
        walk.forged == 400 && walk.favicon == 404 && landed,
        json!({
            "forgedCallback": walk.forged,
            "favicon": walk.favicon,
            "callbackStatus": walk.status,
            "redirectedToWebsite": landed,
        }),
    );
    let exchanges = stub.seen("POST", "/oauth2/token");
    let form = exchanges
        .first()
        .map(|seen| seen.form.clone())
        .unwrap_or_default();
    let field = |key: &str| form.get(key).cloned().unwrap_or_default();
    let authorized_redirect = stub
        .seen("GET", "/oauth2/authorize")
        .first()
        .and_then(|seen| seen.query.get("redirect_uri").cloned())
        .unwrap_or_default();
    let verifier = field("code_verifier");
    let s256 = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == challenge;
    j.guard(
        "the code is redeemed with the verifier its challenge was made from",
        exchanges.len() == 1
            && field("grant_type") == "authorization_code"
            && field("client_id") == CLIENT_ID
            && field("code") == CODE
            && field("redirect_uri") == authorized_redirect
            && s256,
        json!({
            "tokenRequests": exchanges.len(),
            "grantType": field("grant_type"),
            "clientId": field("client_id"),
            "redirectUri": field("redirect_uri"),
            "redirectUriMatchesAuthorize": field("redirect_uri") == authorized_redirect,
            "verifierLength": verifier.len(),
            "s256MatchesChallenge": s256,
        }),
    );
    let sessions = stub.seen("GET", "/v1/session");
    let bearer = sessions
        .first()
        .is_some_and(|seen| seen.authorization.as_deref() == Some("Bearer e2e-access-1"));
    j.guard(
        "the session is read with the issued access token",
        sessions.len() == 1 && bearer,
        json!({ "sessionRequests": sessions.len(), "bearerIsTheAccessToken": bearer }),
    );

    j.step("the state is one-shot");
    let again = call(
        &mut j,
        &a,
        &mut answers,
        Request::AccountFinishSignIn {
            state: state.clone(),
            code: code.clone(),
        },
    )
    .await;
    let token_requests = stub.seen("POST", "/oauth2/token").len();
    j.guard(
        "a second finish with the same state is stale",
        code_of(&again).as_deref() == Some("account_sign_in_stale") && token_requests == 1,
        json!({ "code": code_of(&again), "tokenRequests": token_requests }),
    );

    j.step("only clients that announced AccountChanged hear it");
    let mut heard_a = account_changes(&events_until(&mut a_events, IO, is_account_changed).await);
    heard_a += account_changes(&events_until(&mut a_events, QUIET, |_| false).await);
    let heard_b = account_changes(&events_until(&mut b_events, QUIET, |_| false).await);
    let heard_c = account_changes(&events_until(&mut c_events, QUIET, |_| false).await);
    j.guard(
        "A hears it once; B (session kinds only) and C (never announced) do not",
        heard_a == 1 && heard_b == 0 && heard_c == 0,
        json!({ "a": heard_a, "b": heard_b, "c": heard_c }),
    );

    j.step("account.json is private and names its pool");
    let mode = std::fs::metadata(&account_file)
        .map(|meta| meta.permissions().mode() & 0o777)
        .unwrap_or_default();
    let stored = sandbox.read_json(&account_file);
    let stored_keys: Vec<String> = stored
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    j.guard(
        "the file is owner-only and records the pool that issued it",
        mode == 0o600
            && text(&stored, "/clientId") == CLIENT_ID
            && text(&stored, "/loginUrl") == stub.base,
        json!({
            "mode": format!("{mode:o}"),
            "clientId": text(&stored, "/clientId"),
            "loginUrl": text(&stored, "/loginUrl"),
            "keys": stored_keys,
        }),
    );

    j.step("the account survives a restart");
    drop((b, c));
    let stopped = server.shutdown(&a).await;
    j.guard(
        "the first server stops cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code() }),
    );
    let server = Server::start(&sandbox).await;
    let a = server.client();
    let status = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    j.guard(
        "status reads the same identity from disk",
        status.as_ref().ok() == Some(&signed_in),
        json!({ "status": outcome(&status) }),
    );

    j.step("sign out");
    call(
        &mut j,
        &a,
        &mut answers,
        announce("client-a", &TARGETED_EVENTS),
    )
    .await
    .expect("A announces again");
    let mut a_events = a
        .take_events()
        .expect("A's runtime events after the restart");
    let signed_out = call(&mut j, &a, &mut answers, Request::AccountSignOut {}).await;
    let heard = account_changes(&events_until(&mut a_events, IO, is_account_changed).await);
    j.guard(
        "sign-out answers signed out, deletes the file, and A hears it",
        signed_out.as_ref().ok() == Some(&json!({ "state": "signedOut" }))
            && !account_file.exists()
            && heard == 1,
        json!({
            "result": outcome(&signed_out),
            "fileExists": account_file.exists(),
            "heard": heard,
        }),
    );

    j.step("a stale token refreshes on status, once, and emits nothing");
    stub.state().expires_in = 30;
    let signed = sign_in(&mut j, &a, &mut answers).await;
    stub.state().expires_in = 3600;
    drain_until_changed(&mut a_events).await;
    let before = sandbox.read_json(&account_file);
    let mark = stub.mark();
    let status = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    let grants = stub.since(mark, "POST", "/oauth2/token");
    let after = sandbox.read_json(&account_file);
    let heard = quiet_changes(&mut a_events).await;
    let grant_field = |key: &str| {
        grants
            .first()
            .and_then(|seen| seen.form.get(key).cloned())
            .unwrap_or_default()
    };
    j.guard(
        "one refresh grant carries the issued refresh token, and A hears nothing",
        signed.as_ref().ok() == Some(&signed_in)
            && status.as_ref().ok() == Some(&signed_in)
            && grants.len() == 1
            && grant_field("grant_type") == "refresh_token"
            && grant_field("refresh_token") == text(&before, "/refreshToken")
            && grant_field("client_id") == CLIENT_ID
            && heard == 0,
        json!({
            "status": outcome(&status),
            "refreshGrants": grants.len(),
            "grantType": grant_field("grant_type"),
            "sentTheIssuedRefreshToken": grant_field("refresh_token") == text(&before, "/refreshToken"),
            "clientId": grant_field("client_id"),
            "heard": heard,
        }),
    );
    j.guard(
        "the answer has no refresh token, so the stored one is carried forward",
        text(&after, "/refreshToken") == text(&before, "/refreshToken")
            && text(&after, "/accessToken") != text(&before, "/accessToken"),
        json!({
            "refreshTokenKept": text(&after, "/refreshToken") == text(&before, "/refreshToken"),
            "accessTokenReplaced": text(&after, "/accessToken") != text(&before, "/accessToken"),
        }),
    );

    j.step("invalid_grant signs out");
    age(&account_file);
    stub.state().refresh_invalid = true;
    let status = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    stub.state().refresh_invalid = false;
    let heard = account_changes(&events_until(&mut a_events, IO, is_account_changed).await);
    j.guard(
        "a dead refresh token signs out, deletes the file, and A hears it",
        status.as_ref().ok() == Some(&json!({ "state": "signedOut" }))
            && !account_file.exists()
            && heard == 1,
        json!({
            "status": outcome(&status),
            "fileExists": account_file.exists(),
            "heard": heard,
        }),
    );

    j.step("start refreshes a fresh token, then reads the session with the new one");
    let signed = sign_in(&mut j, &a, &mut answers).await;
    drain_until_changed(&mut a_events).await;
    let before = sandbox.read_json(&account_file);
    let mark = stub.mark();
    let stopped = server.shutdown(&a).await;
    let server = Server::start(&sandbox).await;
    let a = server.client();
    call(
        &mut j,
        &a,
        &mut answers,
        announce("client-a", &TARGETED_EVENTS),
    )
    .await
    .expect("A announces after the second restart");
    let mut a_events = a
        .take_events()
        .expect("A's runtime events after the second restart");
    let settled = eventually(IO, || stub.trail(mark).len() >= 2).await;
    let status = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    let trail = stub.trail(mark);
    let after = sandbox.read_json(&account_file);
    let sessions = stub.since(mark, "GET", "/v1/session");
    let used_new_token = sessions.first().and_then(|seen| seen.authorization.clone())
        == Some(format!("Bearer {}", text(&after, "/accessToken")));
    let heard = quiet_changes(&mut a_events).await;
    j.guard(
        "one refresh, then session 200 with the new token, unprompted",
        signed.as_ref().ok() == Some(&signed_in)
            && stopped.status.success()
            && settled
            && trail == ["POST /oauth2/token", "GET /v1/session"]
            && used_new_token
            && text(&after, "/accessToken") != text(&before, "/accessToken")
            && status.as_ref().ok() == Some(&signed_in)
            && heard == 0,
        json!({
            "trail": trail,
            "sessionUsedTheNewToken": used_new_token,
            "status": outcome(&status),
            "heard": heard,
        }),
    );

    j.step("a transient failure keeps the session and backs off");
    age(&account_file);
    stub.state().refresh_unavailable = true;
    let bytes = std::fs::read(&account_file).unwrap_or_default();
    let mark = stub.mark();
    let first = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    let second = call(&mut j, &a, &mut answers, Request::AccountStatus {}).await;
    stub.state().refresh_unavailable = false;
    let requests = stub.since(mark, "POST", "/oauth2/token").len();
    let unchanged = std::fs::read(&account_file).is_ok_and(|now| now == bytes);
    let heard = quiet_changes(&mut a_events).await;
    j.guard(
        "a 503 keeps the session, is tried once, and leaves the file and A alone",
        first.as_ref().ok() == Some(&signed_in)
            && second.as_ref().ok() == Some(&signed_in)
            && requests == 1
            && unchanged
            && heard == 0,
        json!({
            "first": outcome(&first),
            "second": outcome(&second),
            "tokenRequests": requests,
            "fileUnchanged": unchanged,
            "heard": heard,
        }),
    );

    j.step("sign-out revokes the refresh token");
    let refresh = text(&sandbox.read_json(&account_file), "/refreshToken");
    let mark = stub.mark();
    let out = call(&mut j, &a, &mut answers, Request::AccountSignOut {}).await;
    let revokes = stub.since(mark, "POST", "/oauth2/revoke");
    let revoked = |key: &str| {
        revokes
            .first()
            .and_then(|seen| seen.form.get(key).cloned())
            .unwrap_or_default()
    };
    drain_until_changed(&mut a_events).await;
    j.guard(
        "the stored refresh token is revoked for the client, and the file is gone",
        out.as_ref().ok() == Some(&json!({ "state": "signedOut" }))
            && revokes.len() == 1
            && revoked("token") == refresh
            && revoked("client_id") == CLIENT_ID
            && !account_file.exists(),
        json!({
            "result": outcome(&out),
            "revokeRequests": revokes.len(),
            "revokedTheStoredToken": revoked("token") == refresh,
            "clientId": revoked("client_id"),
            "fileExists": account_file.exists(),
        }),
    );

    j.step("a failed revoke still signs out");
    let signed = sign_in(&mut j, &a, &mut answers).await;
    drain_until_changed(&mut a_events).await;
    stub.state().revoke_unavailable = true;
    let mark = stub.mark();
    let out = call(&mut j, &a, &mut answers, Request::AccountSignOut {}).await;
    let revokes = stub.since(mark, "POST", "/oauth2/revoke").len();
    j.guard(
        "a 503 from revoke is only logged: signed out, file gone",
        signed.as_ref().ok() == Some(&signed_in)
            && out.as_ref().ok() == Some(&json!({ "state": "signedOut" }))
            && revokes == 1
            && !account_file.exists(),
        json!({
            "result": outcome(&out),
            "revokeRequests": revokes,
            "fileExists": account_file.exists(),
        }),
    );

    j.step("a failed exchange never sends the browser to the website");
    let (refused, walk) = sign_in_with(&mut j, &a, &mut answers, Some("e2e-code-wrong")).await;
    let failed_page = walk.page.contains("Sign-in failed");
    let landed = walk.hops.iter().any(|hop| hop == SUCCESS_PAGE);
    j.guard(
        "the browser gets the failure page from the callback, and no file is written",
        code_of(&refused).as_deref() == Some("account_sign_in_failed")
            && walk.status == 200
            && failed_page
            && !landed
            && !account_file.exists(),
        json!({
            "code": code_of(&refused),
            "callbackStatus": walk.status,
            "failurePage": failed_page,
            "reachedWebsite": landed,
            "fileExists": account_file.exists(),
        }),
    );

    j.step("no token left the server");
    let issued = stub.state().issued;
    let in_answers = answers.iter().any(|answer| mentions_token(answer));
    let (logs_read, in_logs) = scan_logs(&sandbox.logs);
    j.guard(
        "no issued token is in an RPC answer or a server log",
        issued == TOKENS_ISSUED && !in_answers && logs_read && !in_logs,
        json!({
            "tokensIssued": issued,
            "inAnswers": in_answers,
            "logsRead": logs_read,
            "inLogs": in_logs,
        }),
    );

    j.step("shutdown");
    let stopped = server.shutdown(&a).await;
    j.guard(
        "shutdown stops the server cleanly",
        stopped.status.success() && !stopped.still_listening,
        json!({ "exit": stopped.status.code(), "listening": stopped.still_listening }),
    );

    j.finish();
}

async fn sign_in(
    j: &mut Journey,
    client: &RpcClient,
    answers: &mut Vec<String>,
) -> Result<Value, WireError> {
    sign_in_with(j, client, answers, None).await.0
}

/// The Phase 1 browser flow again: the desktop's loopback and the browser, then finish
/// with the callback's code or `code`, answering the browser as the desktop does.
async fn sign_in_with(
    j: &mut Journey,
    client: &RpcClient,
    answers: &mut Vec<String>,
    code: Option<&str>,
) -> (Result<Value, WireError>, Walk) {
    let loopback = Loopback::bind_first(&[0]).await.expect("bind the loopback");
    let base = format!("http://localhost:{}", loopback.port());
    let start = call(
        j,
        client,
        answers,
        Request::AccountStartSignIn {
            port: loopback.port(),
        },
    )
    .await
    .expect("account_start_sign_in");
    let state = text(&start, "/state");
    let authorize_url = text(&start, "/authorize_url");
    let logout_url = text(&start, "/logout_url");
    let cancel = Notify::new();
    let walk = spawn_browse(&logout_url, &base);
    let mut callback = loopback
        .wait(IO, &state, &authorize_url, &cancel)
        .await
        .expect("the loopback hears the callback");
    let code = match code {
        Some(code) => code.to_string(),
        None => callback.code.clone().expect("the callback carries a code"),
    };
    let finished = call(
        j,
        client,
        answers,
        Request::AccountFinishSignIn { state, code },
    )
    .await;
    callback.reply(finished.is_ok()).await;
    (finished, walk.await.expect("the browser walk"))
}

fn spawn_browse(start: &str, loopback: &str) -> tokio::task::JoinHandle<Walk> {
    let browser = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .expect("build the browser");
    let (start, loopback) = (start.to_string(), loopback.to_string());
    tokio::spawn(async move { browse(&browser, &start, &loopback).await })
}

/// Backdates the stored token so the next read finds it stale.
fn age(account_file: &Path) {
    let mut account: Value =
        serde_json::from_slice(&std::fs::read(account_file).expect("read account.json"))
            .expect("parse account.json");
    account["expiresAt"] = json!(0);
    std::fs::write(
        account_file,
        serde_json::to_vec_pretty(&account).expect("encode account.json"),
    )
    .expect("write account.json");
}

/// Waits out the `AccountChanged` a sign-in sends, then any stragglers.
async fn drain_until_changed(events: &mut tokio::sync::mpsc::Receiver<RuntimeEvent>) {
    events_until(events, IO, is_account_changed).await;
    events_until(events, QUIET, |_| false).await;
}

async fn quiet_changes(events: &mut tokio::sync::mpsc::Receiver<RuntimeEvent>) -> usize {
    account_changes(&events_until(events, QUIET, |_| false).await)
}

/// Records the answer for the token scan as well as in the transcript.
async fn call(
    j: &mut Journey,
    client: &RpcClient,
    answers: &mut Vec<String>,
    request: Request,
) -> Result<Value, WireError> {
    let reply = j.call(client, request).await;
    answers.push(match &reply {
        Ok(value) => value.to_string(),
        Err(error) => serde_json::to_string(error).unwrap_or_default(),
    });
    reply
}

fn announce(client_id: &str, kinds: &[&str]) -> Request {
    Request::AnnounceClient {
        client_id: client_id.into(),
        device: format!("{client_id}-device"),
        capabilities: kinds.iter().map(|kind| kind.to_string()).collect(),
    }
}

fn outcome(reply: &Result<Value, WireError>) -> Value {
    match reply {
        Ok(value) => value.clone(),
        Err(error) => json!({ "code": error.code }),
    }
}

fn code_of(reply: &Result<Value, WireError>) -> Option<String> {
    reply.as_ref().err().map(|error| error.code.clone())
}

fn is_account_changed(event: &RuntimeEvent) -> bool {
    matches!(event, RuntimeEvent::AccountChanged)
}

fn account_changes(events: &[RuntimeEvent]) -> usize {
    events.iter().filter(|e| is_account_changed(e)).count()
}

fn mentions_token(text: &str) -> bool {
    TOKEN_PREFIXES.iter().any(|prefix| text.contains(prefix))
}

/// Whether any log file was read, and whether one mentions a token.
fn scan_logs(dir: &Path) -> (bool, bool) {
    let logs: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| std::fs::read(entry.path()).ok())
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .collect()
        })
        .unwrap_or_default();
    (!logs.is_empty(), logs.iter().any(|log| mentions_token(log)))
}

/// Decoded, so a percent-encoded port never reaches the evidence unscrubbed.
fn query(url: &str) -> BTreeMap<String, String> {
    reqwest::Url::parse(url)
        .map(|url| url.query_pairs().into_owned().collect())
        .unwrap_or_default()
}

/// Origin and path: the part of a hop that `$CLOUD` and `$LOOPBACK` scrub.
fn location(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| format!("{}{}", url.origin().ascii_serialization(), url.path()))
        .unwrap_or_default()
}

const SUCCESS_PAGE: &str = "https://hyprlayer.dev/signin/success";

struct Walk {
    hops: Vec<String>,
    forged: u16,
    favicon: u16,
    status: u16,
    page: String,
}

/// Follows one redirect at a time, as a browser does; before the real callback it tries
/// a forged one and a stray probe on the same listener.
async fn browse(browser: &reqwest::Client, start: &str, loopback: &str) -> Walk {
    let mut walk = Walk {
        hops: Vec::new(),
        forged: 0,
        favicon: 0,
        status: 0,
        page: String::new(),
    };
    let mut url = start.to_string();
    for _ in 0..7 {
        walk.hops.push(location(&url));
        // The browser leaves for the website here; the test never goes online.
        if location(&url) == SUCCESS_PAGE {
            break;
        }
        if location(&url) == format!("{loopback}/auth/callback") {
            let forged = format!("{loopback}/auth/callback?code=forged&state=wrong");
            walk.forged = get(browser, &forged).await.0;
            walk.favicon = get(browser, &format!("{loopback}/favicon.ico")).await.0;
        }
        let (status, next, body) = get(browser, &url).await;
        walk.status = status;
        walk.page = body;
        match next {
            Some(next) => url = next,
            None => break,
        }
    }
    walk
}

async fn get(browser: &reqwest::Client, url: &str) -> (u16, Option<String>, String) {
    let Ok(response) = browser.get(url).send().await else {
        return (0, None, String::new());
    };
    let status = response.status().as_u16();
    let next = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    (status, next, response.text().await.unwrap_or_default())
}

#[derive(Clone)]
struct Seen {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    form: BTreeMap<String, String>,
    authorization: Option<String>,
}

#[derive(Default)]
struct StubState {
    seen: Vec<Seen>,
    challenge: Option<String>,
    redirect_uri: Option<String>,
    issued: u32,
    access: Option<String>,
    refresh: Option<String>,
    expires_in: u64,
    refresh_unavailable: bool,
    refresh_invalid: bool,
    revoke_unavailable: bool,
}

/// Cognito's hosted UI and token endpoint plus the Cloud API's `/v1/session`, on one
/// loopback port, recording every request.
struct Stub {
    base: String,
    state: Arc<Mutex<StubState>>,
}

impl Stub {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the stub");
        let base = format!("http://{}", listener.local_addr().expect("stub address"));
        let state = Arc::new(Mutex::new(StubState {
            expires_in: 3600,
            ..StubState::default()
        }));
        let shared = state.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, shared.clone()));
            }
        });
        Self { base, state }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, StubState> {
        self.state.lock().expect("stub state")
    }

    /// Where the request log stands now, for `since` and `trail`.
    fn mark(&self) -> usize {
        self.state().seen.len()
    }

    fn since(&self, mark: usize, method: &str, path: &str) -> Vec<Seen> {
        self.state()
            .seen
            .iter()
            .skip(mark)
            .filter(|seen| seen.method == method && seen.path == path)
            .cloned()
            .collect()
    }

    /// The session and token requests after `mark`, in order.
    fn trail(&self, mark: usize) -> Vec<String> {
        self.state()
            .seen
            .iter()
            .skip(mark)
            .filter(|seen| matches!(seen.path.as_str(), "/v1/session" | "/oauth2/token"))
            .map(|seen| format!("{} {}", seen.method, seen.path))
            .collect()
    }

    fn seen(&self, method: &str, path: &str) -> Vec<Seen> {
        self.state()
            .seen
            .iter()
            .filter(|seen| seen.method == method && seen.path == path)
            .cloned()
            .collect()
    }
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<StubState>>) {
    let Some(seen) = read_request(&mut stream).await else {
        return;
    };
    let response = route(&seen, &state);
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let target = request_line.next()?.to_string();
    let mut content_length = 0;
    let mut authorization = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => content_length = value.trim().parse().unwrap_or(0),
            "authorization" => authorization = Some(value.trim().to_string()),
            _ => {}
        }
    }
    while bytes.len() < head_end + content_length {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let body = String::from_utf8_lossy(&bytes[head_end..]).into_owned();
    let url = reqwest::Url::parse(&format!("http://stub{target}")).ok()?;
    Some(Seen {
        method,
        path: url.path().to_string(),
        query: url.query_pairs().into_owned().collect(),
        form: query(&format!("http://stub/?{body}")),
        authorization,
    })
}

fn route(seen: &Seen, state: &Mutex<StubState>) -> String {
    let mut stub = state.lock().expect("stub state");
    stub.seen.push(seen.clone());
    let param = |key: &str| seen.query.get(key).cloned().unwrap_or_default();
    let field = |key: &str| seen.form.get(key).cloned().unwrap_or_default();
    match (seen.method.as_str(), seen.path.as_str()) {
        ("GET", "/logout") if param("client_id") == CLIENT_ID => redirect(&param("logout_uri")),
        ("GET", "/oauth2/authorize") => {
            stub.challenge = Some(param("code_challenge"));
            stub.redirect_uri = Some(param("redirect_uri"));
            let back = reqwest::Url::parse_with_params(
                &param("redirect_uri"),
                [("code", CODE.to_string()), ("state", param("state"))],
            )
            .map(String::from)
            .unwrap_or_default();
            redirect(&back)
        }
        ("POST", "/oauth2/token") if field("grant_type") == "refresh_token" => {
            if stub.refresh_unavailable {
                return respond(503, r#"{"message":"unavailable"}"#);
            }
            let known = stub.refresh.as_deref() == Some(field("refresh_token").as_str());
            if stub.refresh_invalid || field("client_id") != CLIENT_ID || !known {
                return respond(400, r#"{"error":"invalid_grant"}"#);
            }
            stub.issued += 1;
            let n = stub.issued;
            stub.access = Some(format!("e2e-access-{n}"));
            // Cognito answers a refresh without a refresh token.
            let tokens = json!({
                "id_token": format!("e2e-id-{n}"),
                "access_token": format!("e2e-access-{n}"),
                "expires_in": stub.expires_in,
                "token_type": "Bearer",
            });
            respond(200, &tokens.to_string())
        }
        ("POST", "/oauth2/revoke") => {
            if stub.revoke_unavailable {
                respond(503, r#"{"message":"unavailable"}"#)
            } else {
                respond(200, "{}")
            }
        }
        ("POST", "/oauth2/token") => {
            let s256 = URL_SAFE_NO_PAD.encode(Sha256::digest(field("code_verifier").as_bytes()));
            let valid = field("grant_type") == "authorization_code"
                && field("client_id") == CLIENT_ID
                && field("code") == CODE
                && stub.redirect_uri.as_deref() == Some(field("redirect_uri").as_str())
                && stub.challenge.as_deref() == Some(s256.as_str());
            if !valid {
                return respond(400, r#"{"error":"invalid_grant"}"#);
            }
            stub.issued += 1;
            let n = stub.issued;
            stub.access = Some(format!("e2e-access-{n}"));
            stub.refresh = Some(format!("e2e-refresh-{n}"));
            let tokens = json!({
                "id_token": format!("e2e-id-{n}"),
                "access_token": format!("e2e-access-{n}"),
                "refresh_token": format!("e2e-refresh-{n}"),
                "expires_in": stub.expires_in,
                "token_type": "Bearer",
            });
            respond(200, &tokens.to_string())
        }
        ("GET", "/v1/session") => {
            let current = stub
                .access
                .as_ref()
                .map(|access| format!("Bearer {access}"));
            if current.is_some() && seen.authorization == current {
                respond(200, SESSION)
            } else {
                respond(401, r#"{"message":"Unauthorized"}"#)
            }
        }
        _ => respond(404, "{}"),
    }
}

fn respond(status: u16, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        503 => "Service Unavailable",
        _ => "Not Found",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn redirect(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}
