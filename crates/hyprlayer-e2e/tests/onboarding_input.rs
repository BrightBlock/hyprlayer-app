//! Onboarding input that would resolve against the daemon's working directory
//! is refused before anything is written.
#![cfg(unix)]

use hyprlayer_core::runtime::projects::ThoughtsSource;
use hyprlayer_e2e::{json, Journey, Request, Sandbox, Server};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn onboarding_refuses_input_that_escapes_the_profile() {
    let sandbox = Sandbox::new();
    let seed = sandbox.fixture_repo("seed-thoughts", &[("README.md", "# thoughts\n")]);
    let mut j = Journey::new("onboarding_input", &sandbox);
    let server = Server::start(&sandbox).await;
    let client = server.client();
    let cwd_before = sandbox.listing(&sandbox.cwd);
    let untouched = sandbox.home.join("untouched");

    j.step("refuse bad input");
    let cases = [
        (
            "a blank create path",
            "e2e",
            ThoughtsSource::Create {
                path: String::new(),
            },
        ),
        (
            "a whitespace create path",
            "e2e",
            ThoughtsSource::Create { path: "   ".into() },
        ),
        (
            "a relative create path",
            "e2e",
            ThoughtsSource::Create {
                path: "thoughts".into(),
            },
        ),
        (
            "a blank disk path",
            "e2e",
            ThoughtsSource::Disk {
                path: String::new(),
            },
        ),
        (
            "a relative disk path",
            "e2e",
            ThoughtsSource::Disk { path: ".".into() },
        ),
        (
            "a relative clone location",
            "e2e",
            ThoughtsSource::Clone {
                url: seed.display().to_string(),
                location: Some("cloned".into()),
            },
        ),
        (
            "a blank user",
            "  ",
            ThoughtsSource::Create {
                path: untouched.display().to_string(),
            },
        ),
    ];
    for (label, user, source) in cases {
        let outcome = j
            .call(
                &client,
                Request::InitThoughtsRepo {
                    user: user.into(),
                    source,
                },
            )
            .await;
        let evidence = match &outcome {
            Err(e) => json!({ "code": e.code, "message": e.message }),
            Ok(_) => json!({ "accepted": true }),
        };
        j.guard(
            &format!("onboarding refuses {label}"),
            outcome.is_err(),
            evidence,
        );
    }

    j.step("leave no trace");
    let cwd_after = sandbox.listing(&sandbox.cwd);
    j.guard(
        "the daemon's working directory is untouched",
        cwd_after == cwd_before,
        json!({ "before": cwd_before, "after": cwd_after }),
    );
    j.guard(
        "a refused request creates nothing at its path",
        !untouched.exists(),
        json!({ "exists": untouched.exists() }),
    );
    let thoughts = sandbox.read_json(&sandbox.cli_config())["thoughts"].clone();
    j.guard(
        "the CLI config gains no thoughts section",
        thoughts.is_null(),
        json!({ "thoughts": thoughts }),
    );

    j.step("accept good input");
    let accepted = j
        .call(
            &client,
            Request::InitThoughtsRepo {
                user: "e2e".into(),
                source: ThoughtsSource::Create {
                    path: "~/thoughts".into(),
                },
            },
        )
        .await;
    let repo = sandbox.home.join("thoughts");
    let config = sandbox.read_json(&sandbox.cli_config());
    let recorded = config["thoughts"]["backend"]["thoughtsRepo"].clone();
    j.guard(
        "a ~ path resolves under the profile's home",
        accepted.is_ok() && repo.join(".git").exists() && recorded == json!(repo),
        json!({
            "accepted": accepted.is_ok(),
            "thoughtsRepo": recorded,
            "user": config["thoughts"]["user"],
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
