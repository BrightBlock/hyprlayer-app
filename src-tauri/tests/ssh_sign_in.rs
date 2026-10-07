//! Signing in to a host the way the desktop does: the real `ssh`, a
//! password-only sshd on an x86_64 Linux host, this app's own binary as the
//! askpass helper, and the relay. A responder stands in for the window.
#![cfg(unix)]

use std::collections::HashSet;
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hyprlayer_app_lib::hosts::{Host, Target};
use hyprlayer_app_lib::provision;
use hyprlayer_app_lib::ssh::{Ssh, SshPrompt, SshPromptKind, SshSetup};
use hyprlayer_app_lib::supervisor::{self, HostProbe, Outcome};
use hyprlayer_e2e::containers::{HostOptions, Platform, RemoteHost};
use hyprlayer_e2e::{json, Journey, Sandbox, Value};

const PASSWORD: &str = "correct-horse-battery";
const WRONG: &str = "battery-staple-horse";
const TRIPLE: &str = "x86_64-unknown-linux-gnu";
/// Past the 15s connect deadline, which a pending prompt must pause.
const SLOW_ANSWER: Duration = Duration::from_secs(17);
const WITHDRAWN: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
enum Reply {
    Trust,
    Type(&'static str),
    Cancel,
    Late(Duration, &'static str),
    Ignore,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs Docker: cargo test -p hyprlayer-app --test ssh_sign_in -- --ignored"]
async fn ssh_sign_in() {
    let sandbox = Sandbox::new();
    capture_logs(&sandbox.logs.join("app.log"));
    let mut j = Journey::new("ssh_sign_in", &sandbox);
    let host = RemoteHost::start_opts(
        &sandbox,
        HostOptions {
            platform: Platform::Amd64,
            password: Some(PASSWORD),
            install: false,
            ..HostOptions::default()
        },
    );
    let port = host.port().to_string();
    let fingerprint =
        host.exec("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub | awk '{print $2}'");
    j.alias(&fingerprint, "$FINGERPRINT");
    j.alias(format!("[127.0.0.1]:{port}"), "[127.0.0.1]:$PORT");

    // Short paths: a control socket must fit in 104 bytes with ssh's suffix.
    let short = sandbox.given.clone();
    let desktop = Desktop::write(
        &sandbox.root.join("desktop"),
        &short,
        &port,
        host.identity(),
    );
    let ssh = Ssh::new(desktop.setup(&short.join("askpass"), Duration::from_secs(95)));
    let pw = Host {
        destination: "e2e-pw".into(),
        label: "Password host".into(),
    };
    let data = sandbox.app_data.clone();
    let probe = |ssh: &Ssh, host: &Host| {
        let (ssh, host, data) = (ssh.clone(), host.clone(), data.clone());
        async move { supervisor::probe_remote(&host, &data, &ssh).await }
    };
    let connect = |ssh: &Ssh| {
        let (ssh, target) = (ssh.clone(), Target::Remote(pw.clone()));
        let (config, data, logs) = (
            sandbox.app_config.clone(),
            sandbox.app_data.clone(),
            sandbox.logs.clone(),
        );
        async move { supervisor::connect(&target, &config, &data, &logs, &ssh).await }
    };

    j.step("launch never asks");
    let (outcome, asked) = drive(&ssh, connect(&ssh.batch()), &[]).await;
    j.guard(
        "a launch connect to an unknown host reports sign-in needed",
        blocked_code(&outcome) == Some("remote_sign_in_required"),
        json!({ "outcome": describe(&outcome) }),
    );
    j.guard(
        "it asks nothing",
        asked.is_empty(),
        json!({ "asked": kinds(&asked) }),
    );
    j.guard(
        "it trusts nothing on its own",
        !desktop.knows(&port),
        json!({ "known": desktop.knows(&port) }),
    );

    j.step("decline the host key");
    let (result, asked) = drive(&ssh, probe(&ssh, &pw), &[Reply::Cancel]).await;
    j.guard(
        "an unknown host asks to trust its key first",
        asked.first().map(|p| p.kind) == Some(SshPromptKind::HostKey),
        json!({ "asked": kinds(&asked) }),
    );
    let offered = asked.first().and_then(|p| p.fingerprint.clone());
    j.guard(
        "the fingerprint offered is the one the host presents",
        offered.as_deref() == Some(fingerprint.as_str()),
        json!({ "offered": offered, "key_type": asked.first().and_then(|p| p.key_type.clone()) }),
    );
    let named = asked.first().and_then(|p| p.key_host.clone());
    j.guard(
        "the prompt names the host whose key it is",
        named.as_deref() == Some(format!("[127.0.0.1]:{port}").as_str()),
        json!({ "key_host": named }),
    );
    j.guard(
        "declining ends the attempt there",
        probe_code(&result) == Some("remote_sign_in_cancelled") && asked.len() == 1,
        json!({ "code": probe_code(&result), "asked": asked.len() }),
    );
    j.guard(
        "a declined key is not written",
        !desktop.knows(&port),
        json!({ "known": desktop.knows(&port) }),
    );
    let left = ssh.prompts().borrow().len();
    j.guard(
        "nothing waits once it ends",
        left == 0,
        json!({ "pending": left }),
    );

    j.step("cancel the password");
    let before = Auth::read(&host);
    let (result, asked) = drive(&ssh, probe(&ssh, &pw), &[Reply::Trust, Reply::Cancel]).await;
    let after = Auth::read(&host);
    j.guard(
        "trusting the key writes it to known_hosts",
        desktop.knows(&port),
        json!({ "known": desktop.knows(&port) }),
    );
    j.guard(
        "a password-only host asks for its password next",
        asked
            .get(1)
            .is_some_and(|p| p.kind == SshPromptKind::Secret && p.message.contains("password")),
        json!({ "asked": kinds(&asked) }),
    );
    j.guard(
        "cancelling stops ssh before it sends an empty password",
        after.failed == before.failed && after.accepted == before.accepted,
        json!({
            "failed_passwords": after.failed - before.failed,
            "logins": after.accepted - before.accepted,
        }),
    );
    j.guard(
        "the attempt ends as cancelled",
        probe_code(&result) == Some("remote_sign_in_cancelled") && asked.len() == 2,
        json!({ "code": probe_code(&result), "asked": asked.len() }),
    );

    j.step("a refused password is asked again");
    let before = Auth::read(&host);
    let (result, asked) = drive(
        &ssh,
        probe(&ssh, &pw),
        &[Reply::Type(WRONG), Reply::Type(PASSWORD)],
    )
    .await;
    let after = Auth::read(&host);
    j.guard(
        "a trusted host does not ask about its key again",
        asked.first().map(|p| p.kind) == Some(SshPromptKind::Secret),
        json!({ "asked": kinds(&asked) }),
    );
    j.guard(
        "a refused password comes back marked as a retry",
        asked.len() == 2 && !asked[0].retry && asked[1].retry,
        json!({ "retries": asked.iter().map(|p| p.retry).collect::<Vec<_>>() }),
    );
    j.guard(
        "the right password reaches the host",
        probe_code(&result) == Some("remote_cli_missing"),
        json!({ "code": probe_code(&result) }),
    );
    j.guard(
        "sshd saw exactly the one wrong password",
        after.failed - before.failed == 1 && after.accepted - before.accepted == 1,
        json!({ "failed": after.failed - before.failed, "accepted": after.accepted - before.accepted }),
    );

    j.step("install over the shared connection");
    let staged = provision::cache_path(&sandbox.app_data, TRIPLE);
    std::fs::create_dir_all(staged.parent().expect("cache dir")).expect("create the cache dir");
    std::fs::copy(host.server_binary(), &staged).expect("stage the x86_64 server");
    let before = Auth::read(&host);
    let installed = {
        let (ssh, host, data) = (ssh.clone(), pw.clone(), data.clone());
        async move {
            let install = provision::provision(&host, &data, |_| {}, &ssh).await;
            (install, supervisor::probe_remote(&host, &data, &ssh).await)
        }
    };
    let ((install, result), asked) = drive(&ssh, installed, &[]).await;
    let after = Auth::read(&host);
    j.guard(
        "installing and connecting ask nothing more",
        asked.is_empty(),
        json!({ "asked": kinds(&asked) }),
    );
    j.guard(
        "the installed server answers",
        install.is_ok()
            && result.ok
            && result.server_version == Some(hyprlayer_proto::PROTOCOL_VERSION),
        json!({
            "install": install.as_ref().err().map(|e| &e.code),
            "ok": result.ok,
            "code": probe_code(&result),
        }),
    );
    j.guard(
        "it all rode the one sign-in",
        after.accepted == before.accepted && after.failed == before.failed,
        json!({ "logins": after.accepted - before.accepted }),
    );
    j.guard(
        "the shared connection is Hyprlayer's own",
        sockets(&desktop.control) == 1,
        json!({ "control_sockets": sockets(&desktop.control) }),
    );

    j.step("an established connection asks nothing at launch");
    let (outcome, asked) = drive(&ssh, connect(&ssh.batch()), &[]).await;
    j.guard(
        "a launch connect over the live shared connection connects",
        matches!(outcome, Outcome::Connected { .. }),
        json!({ "outcome": describe(&outcome) }),
    );
    j.guard(
        "it asks nothing",
        asked.is_empty(),
        json!({ "asked": kinds(&asked) }),
    );
    drop(outcome);

    j.step("launch after the shared connection closes");
    desktop.close_masters();
    let before = Auth::read(&host);
    let (outcome, asked) = drive(&ssh, connect(&ssh.batch()), &[]).await;
    let after = Auth::read(&host);
    j.guard(
        "launch reports sign-in needed, not a failed login",
        blocked_code(&outcome) == Some("remote_sign_in_required") && asked.is_empty(),
        json!({ "outcome": describe(&outcome), "asked": kinds(&asked) }),
    );
    j.guard(
        "it sends no password",
        after.failed == before.failed && after.accepted == before.accepted,
        json!({ "failed": after.failed - before.failed, "accepted": after.accepted - before.accepted }),
    );

    j.step("a slow answer outlasts the connect deadline");
    let started = Instant::now();
    let (outcome, asked) = drive(&ssh, connect(&ssh), &[Reply::Late(SLOW_ANSWER, PASSWORD)]).await;
    let waited = started.elapsed() >= SLOW_ANSWER;
    j.guard(
        "an answer after the 15s deadline still connects",
        matches!(outcome, Outcome::Connected { .. }) && waited,
        json!({ "outcome": describe(&outcome), "waited_past_deadline": waited }),
    );
    j.guard(
        "the connect asked once",
        kinds(&asked) == vec!["secret"],
        json!({ "asked": kinds(&asked) }),
    );
    drop(outcome);

    j.step("a dropped attempt withdraws its prompt");
    desktop.close_masters();
    let before = Auth::read(&host);
    let mut rx = ssh.prompts();
    let attempt = tokio::spawn(probe(&ssh, &pw));
    let appeared = tokio::time::timeout(WITHDRAWN, rx.wait_for(|p| !p.is_empty()))
        .await
        .is_ok();
    attempt.abort();
    let withdrawn = tokio::time::timeout(WITHDRAWN, rx.wait_for(|p| p.is_empty()))
        .await
        .is_ok();
    let after = Auth::read(&host);
    j.guard(
        "the prompt is withdrawn when its attempt goes away",
        appeared && withdrawn,
        json!({ "appeared": appeared, "withdrawn": withdrawn }),
    );
    j.guard(
        "no password was sent",
        after.failed == before.failed && after.accepted == before.accepted,
        json!({ "failed": after.failed - before.failed, "accepted": after.accepted - before.accepted }),
    );

    j.step("an unanswered prompt gives up");
    let quick = Ssh::new(desktop.setup(&short.join("askpass-quick"), Duration::from_secs(3)));
    let before = Auth::read(&host);
    let started = Instant::now();
    let (result, asked) = drive(&quick, probe(&quick, &pw), &[Reply::Ignore]).await;
    let after = Auth::read(&host);
    let gave_up = result
        .error
        .as_ref()
        .is_some_and(|e| e.code == "remote_sign_in_cancelled" && e.message.contains("timed out"));
    j.guard(
        "an unanswered prompt ends as timed out",
        gave_up && asked.len() == 1,
        json!({ "code": probe_code(&result), "asked": kinds(&asked) }),
    );
    j.guard(
        "well before sshd's login grace time",
        started.elapsed() < Duration::from_secs(60),
        json!({ "under_a_minute": started.elapsed() < Duration::from_secs(60) }),
    );
    j.guard(
        "no password was sent",
        after.failed == before.failed && after.accepted == before.accepted,
        json!({ "failed": after.failed - before.failed, "accepted": after.accepted - before.accepted }),
    );

    j.step("a key you already use asks nothing");
    desktop.close_masters();
    let key = Host {
        destination: "e2e-key".into(),
        label: "Key host".into(),
    };
    let (result, asked) = drive(&ssh, probe(&ssh, &key), &[]).await;
    j.guard(
        "a host your key opens connects without a prompt",
        result.ok && asked.is_empty(),
        json!({ "ok": result.ok, "code": probe_code(&result), "asked": kinds(&asked) }),
    );
    j.guard(
        "Hyprlayer shares the connection for keys too",
        sockets(&desktop.control) == 1,
        json!({ "control_sockets": sockets(&desktop.control) }),
    );

    j.step("your ControlPath wins");
    desktop.close_masters();
    let own = Host {
        destination: "e2e-own-cm".into(),
        label: "Own master".into(),
    };
    let (result, asked) = drive(&ssh, probe(&ssh, &own), &[]).await;
    j.guard(
        "your ControlPath carries the connection",
        result.ok && asked.is_empty() && sockets(&desktop.user_control) == 1,
        json!({
            "ok": result.ok,
            "code": probe_code(&result),
            "user_sockets": sockets(&desktop.user_control),
        }),
    );
    j.guard(
        "Hyprlayer adds no master of its own",
        sockets(&desktop.control) == 0,
        json!({ "control_sockets": sockets(&desktop.control) }),
    );
    desktop.close_masters();

    j.step("keyboard-interactive asks the same way");
    let kbd = Host {
        destination: "e2e-kbd".into(),
        label: "PAM host".into(),
    };
    let before = Auth::read(&host);
    let (cancelled, asked_first) = drive(&ssh, probe(&ssh, &kbd), &[Reply::Cancel]).await;
    let between = Auth::read(&host);
    let (result, asked) = drive(&ssh, probe(&ssh, &kbd), &[Reply::Type(PASSWORD)]).await;
    let after = Auth::read(&host);
    j.guard(
        "a PAM password prompt is asked as a secret",
        kinds(&asked_first) == vec!["secret"] && kinds(&asked) == vec!["secret"],
        json!({ "asked": [kinds(&asked_first), kinds(&asked)] }),
    );
    j.guard(
        "cancelling it sends PAM nothing",
        probe_code(&cancelled) == Some("remote_sign_in_cancelled")
            && between.pam_failed == before.pam_failed
            && between.pam_accepted == before.pam_accepted,
        json!({
            "code": probe_code(&cancelled),
            "pam_failures": between.pam_failed - before.pam_failed,
            "pam_logins": between.pam_accepted - before.pam_accepted,
        }),
    );
    j.guard(
        "answering it signs in",
        result.ok && after.pam_accepted - between.pam_accepted == 1,
        json!({ "ok": result.ok, "code": probe_code(&result), "pam_logins": after.pam_accepted - between.pam_accepted }),
    );
    desktop.close_masters();

    j.step("a changed host key is refused without asking");
    desktop.forge_host_key(&port);
    let before = Auth::read(&host);
    let (result, asked) = drive(&ssh, probe(&ssh, &pw), &[]).await;
    let after = Auth::read(&host);
    j.guard(
        "a changed key is never offered for trust",
        asked.is_empty(),
        json!({ "asked": kinds(&asked) }),
    );
    j.guard(
        "it is reported as a changed key",
        probe_code(&result) == Some("remote_host_key_changed"),
        json!({ "code": probe_code(&result) }),
    );
    let remedy = result.error.as_ref().and_then(|e| e.remedy.clone());
    let command = format!("ssh-keygen -R '[127.0.0.1]:{port}'");
    j.guard(
        "the remedy names the key to remove",
        remedy.as_deref().is_some_and(|r| r.contains(&command)),
        json!({ "remedy": remedy }),
    );
    j.guard(
        "no login was attempted",
        after.failed == before.failed && after.accepted == before.accepted,
        json!({ "failed": after.failed - before.failed, "accepted": after.accepted - before.accepted }),
    );

    j.step("secrets stay out of the logs");
    let leaks = leaked(&sandbox.logs, &[PASSWORD, WRONG]);
    j.guard(
        "neither password appears in any log",
        leaks.is_empty(),
        json!({ "files": leaks }),
    );

    desktop.close_masters();
    host.save_server_logs();
    std::fs::write(sandbox.logs.join("sshd.log"), host.sshd_log()).expect("save the sshd log");
    j.finish();
}

/// Runs `attempt` on its own task, answering each new prompt with the next reply.
async fn drive<T: Send + 'static>(
    ssh: &Ssh,
    attempt: impl Future<Output = T> + Send + 'static,
    replies: &[Reply],
) -> (T, Vec<SshPrompt>) {
    let mut rx = ssh.prompts();
    let mut task = tokio::spawn(attempt);
    let mut seen = HashSet::new();
    let mut asked = Vec::new();
    let mut replies = replies.iter().copied();
    loop {
        let fresh: Vec<SshPrompt> = rx
            .borrow_and_update()
            .iter()
            .filter(|p| !seen.contains(&p.id))
            .cloned()
            .collect();
        for prompt in fresh {
            seen.insert(prompt.id.clone());
            match replies.next() {
                Some(Reply::Trust) => {
                    ssh.answer(&prompt.id, Some("yes".into()));
                }
                Some(Reply::Type(text)) => {
                    ssh.answer(&prompt.id, Some(text.into()));
                }
                Some(Reply::Late(wait, text)) => {
                    let (ssh, id) = (ssh.clone(), prompt.id.clone());
                    tokio::spawn(async move {
                        tokio::time::sleep(wait).await;
                        ssh.answer(&id, Some(text.into()));
                    });
                }
                Some(Reply::Ignore) => {}
                Some(Reply::Cancel) | None => {
                    ssh.answer(&prompt.id, None);
                }
            }
            asked.push(prompt);
        }
        tokio::select! {
            out = &mut task => return (out.expect("the attempt panicked"), asked),
            _ = rx.changed() => {}
        }
    }
}

/// The sandbox's stand-in for `~/.ssh`: its own config, known_hosts and
/// control dirs, reached through an `ssh` wrapper that passes `-F`.
struct Desktop {
    program: PathBuf,
    known_hosts: PathBuf,
    control: PathBuf,
    user_control: PathBuf,
}

impl Desktop {
    fn write(dir: &Path, short: &Path, port: &str, identity: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).expect("create the desktop ssh dir");
        let known_hosts = dir.join("known_hosts");
        let control = short.join("cm");
        let user_control = short.join("ucm");
        for d in [&control, &user_control] {
            std::fs::create_dir_all(d).expect("create a control dir");
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))
                .expect("narrow a control dir");
        }
        let common = format!(
            "  HostName 127.0.0.1\n  Port {port}\n  User e2e\n  IdentityAgent none\n  \
             UserKnownHostsFile {known}\n  ConnectTimeout 10\n",
            known = known_hosts.display()
        );
        let key = format!(
            "  IdentityFile {}\n  IdentitiesOnly yes\n  PasswordAuthentication no\n  \
             KbdInteractiveAuthentication no\n",
            identity.display()
        );
        let config = dir.join("config");
        std::fs::write(
            &config,
            format!(
                "Host e2e-pw\n{common}  PubkeyAuthentication no\n  PreferredAuthentications password\n\n\
                 Host e2e-kbd\n{common}  PubkeyAuthentication no\n  \
                 PreferredAuthentications keyboard-interactive\n\n\
                 Host e2e-key\n{common}{key}\n\
                 Host e2e-own-cm\n{common}{key}  ControlMaster auto\n  ControlPath {ucm}/%p\n  \
                 ControlPersist 30\n",
                ucm = user_control.display()
            ),
        )
        .expect("write the desktop ssh config");
        let program = dir.join("ssh");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nexec /usr/bin/ssh -F '{}' \"$@\"\n",
                config.display()
            ),
        )
        .expect("write the ssh wrapper");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("make the wrapper executable");
        Self {
            program,
            known_hosts,
            control,
            user_control,
        }
    }

    fn setup(&self, socket_dir: &Path, prompt_timeout: Duration) -> SshSetup {
        SshSetup {
            program: self.program.clone(),
            askpass: Some(PathBuf::from(env!("CARGO_BIN_EXE_Hyprlayer"))),
            socket_dir: socket_dir.to_path_buf(),
            control_dir: Some(self.control.clone()),
            prompt_timeout,
        }
    }

    fn knows(&self, port: &str) -> bool {
        std::fs::read_to_string(&self.known_hosts)
            .is_ok_and(|known| known.contains(&format!("[127.0.0.1]:{port}")))
    }

    /// Swaps the trusted key for one the host does not hold.
    fn forge_host_key(&self, port: &str) {
        let key = self.known_hosts.with_file_name("forged");
        let _ = std::fs::remove_file(&key);
        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-q", "-f"])
            .arg(&key)
            .status()
            .expect("run ssh-keygen");
        assert!(status.success(), "ssh-keygen failed");
        let public =
            std::fs::read_to_string(key.with_extension("pub")).expect("read the forged key");
        let fields: Vec<&str> = public.split_whitespace().take(2).collect();
        std::fs::write(
            &self.known_hosts,
            format!("[127.0.0.1]:{port} {}\n", fields.join(" ")),
        )
        .expect("forge known_hosts");
    }

    /// `-O exit` on every master either control dir holds.
    fn close_masters(&self) {
        for dir in [&self.control, &self.user_control] {
            for socket in socket_paths(dir) {
                let _ = Command::new(&self.program)
                    .arg("-o")
                    .arg(format!("ControlPath={}", socket.display()))
                    .args(["-O", "exit", "e2e-pw"])
                    .output();
            }
        }
        let gone = Instant::now() + Duration::from_secs(5);
        while (sockets(&self.control) + sockets(&self.user_control)) > 0 && Instant::now() < gone {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        self.close_masters();
    }
}

/// What sshd logged about logins so far; PAM's keyboard-interactive counts apart.
struct Auth {
    failed: usize,
    accepted: usize,
    pam_failed: usize,
    pam_accepted: usize,
}

impl Auth {
    fn read(host: &RemoteHost) -> Self {
        let log = host.sshd_log();
        Self {
            failed: log.matches("Failed password for e2e").count(),
            accepted: log.matches("Accepted password for e2e").count(),
            // Debian's sshd logs the first per declined answer; other builds the second.
            pam_failed: log.matches("Authentication failure for e2e").count()
                + log
                    .matches("Failed keyboard-interactive/pam for e2e")
                    .count(),
            pam_accepted: log
                .matches("Accepted keyboard-interactive/pam for e2e")
                .count(),
        }
    }
}

fn socket_paths(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

fn sockets(dir: &Path) -> usize {
    socket_paths(dir).len()
}

fn kinds(prompts: &[SshPrompt]) -> Vec<&'static str> {
    prompts
        .iter()
        .map(|p| match p.kind {
            SshPromptKind::HostKey => "hostKey",
            SshPromptKind::Secret => "secret",
            SshPromptKind::Confirm => "confirm",
            SshPromptKind::Notice => "notice",
        })
        .collect()
}

fn probe_code(probe: &HostProbe) -> Option<&str> {
    probe.error.as_ref().map(|e| e.code.as_str())
}

fn blocked_code(outcome: &Outcome) -> Option<&str> {
    match outcome {
        Outcome::Blocked { error, .. } => Some(error.code.as_str()),
        Outcome::Connected { .. } => None,
    }
}

fn describe(outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Connected { client, .. } => json!({ "connected": client.server_version() }),
        Outcome::Blocked { error, .. } => json!({ "blocked": error.code }),
    }
}

/// Files under `dir` that contain any of `secrets`.
fn leaked(dir: &Path, secrets: &[&str]) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    std::fs::read_to_string(e.path())
                        .is_ok_and(|text| secrets.iter().any(|s| text.contains(s)))
                })
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The app's `log` records, for the leak check.
fn capture_logs(path: &Path) {
    struct FileLog(Mutex<std::fs::File>);
    impl log::Log for FileLog {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            if let Ok(mut file) = self.0.lock() {
                let _ = writeln!(
                    file,
                    "{} {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                );
            }
        }
        fn flush(&self) {}
    }
    let file = std::fs::File::create(path).expect("create the app log");
    if log::set_boxed_logger(Box::new(FileLog(Mutex::new(file)))).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
}
