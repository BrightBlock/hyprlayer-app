//! The CLI's agent bundle, fetched once per CLI version and copied into each
//! sandbox, so the CLI's first call there has nothing to download.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;
use sha2::{Digest, Sha256};

const RELEASES: &str = "https://api.github.com/repos/BrightBlock/hyprlayer-cli/releases/tags";
const KINDS: [&str; 2] = ["claude", "codex"];
/// Bounded, because a stalled fetch would hold the cache lock for every test process.
const CURL: [&str; 7] = [
    "-sSL",
    "--connect-timeout",
    "10",
    "--max-time",
    "120",
    "--retry",
    "2",
];

/// The installed CLI's version. Clap answers `--version` before the CLI's
/// startup checks, so this provisions and fetches nothing.
pub(crate) fn version(hyprlayer: &Path) -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let output = Command::new(hyprlayer)
            .arg("--version")
            .output()
            .expect("run hyprlayer --version");
        let stdout = String::from_utf8_lossy(&output.stdout);
        match stdout.split_whitespace().collect::<Vec<_>>()[..] {
            ["hyprlayer", version, sha] if sha.starts_with('(') && sha.ends_with(')') => {
                version.to_string()
            }
            _ => panic!("unexpected `hyprlayer --version` output: {stdout:?}"),
        }
    })
}

/// Copies the bundle into a sandbox's agent store; the CLI links it into the
/// profile's home at startup without the network.
pub(crate) fn seed(version: &str, dest: &Path) {
    let cache = cache(version);
    std::fs::create_dir_all(dest.parent().expect("an agent store dir"))
        .expect("create the agent store");
    // A real copy: the CLI rejects a symlinked generation. `-c` clones on APFS.
    let flags = if cfg!(target_os = "macos") {
        "-Rc"
    } else {
        "-R"
    };
    let status = Command::new("cp")
        .arg(flags)
        .arg(cache)
        .arg(dest)
        .status()
        .expect("run cp");
    assert!(status.success(), "copying {} failed", cache.display());
}

/// `target/e2e/cli-assets/<version>`, filled under a lock shared by every
/// test process and published by rename, so a reader never sees half of it.
fn cache(version: &str) -> &'static Path {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(|| {
        let root = crate::journey::target_dir().join("e2e").join("cli-assets");
        let dir = root.join(version);
        if dir.is_dir() {
            return dir;
        }
        std::fs::create_dir_all(&root).expect("create the CLI assets cache");
        let lock = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join(".lock"))
            .expect("open the CLI assets lock");
        lock.lock().expect("lock the CLI assets cache");
        if !dir.is_dir() {
            fill(version, &root, &dir);
        }
        dir
    })
}

fn fill(version: &str, root: &Path, dir: &Path) {
    // Under the lock no other fill runs, so any temp dir is a failed fill's.
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().contains(".tmp-") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let tmp = root.join(format!("{version}.tmp-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create the CLI assets temp dir");
    let release = release(version, root, &tmp);
    for kind in KINDS {
        let name = format!("hyprlayer-assets-{kind}-{version}.tar.gz");
        let asset = release["assets"]
            .as_array()
            .and_then(|assets| assets.iter().find(|a| a["name"] == name.as_str()))
            .unwrap_or_else(|| {
                panic!("the v{version} release has no {name}; install a CLI release that ships it")
            });
        let Some(expected) = asset["digest"].as_str() else {
            panic!("the v{version} release lists no digest for {name}, so it can't be verified");
        };
        let url = asset["browser_download_url"].as_str().unwrap_or_default();
        let tarball = tmp.join(&name);
        run(
            Command::new("curl")
                .args(CURL)
                .args(["-f", "-o"])
                .arg(&tarball)
                .arg(url),
            root,
        );
        let bytes = std::fs::read(&tarball).expect("read a downloaded bundle");
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        if digest != expected {
            fail(
                root,
                &format!("{name} is {digest}; the release says {expected}"),
            );
        }
        let unpacked = tmp.join(kind);
        std::fs::create_dir_all(&unpacked).expect("create a bundle dir");
        run(
            Command::new("tar")
                .arg("-xzf")
                .arg(&tarball)
                .arg("-C")
                .arg(&unpacked),
            root,
        );
        std::fs::remove_file(&tarball).expect("remove a downloaded bundle");
    }
    std::fs::rename(&tmp, dir).expect("publish the CLI assets cache");
}

/// The release's JSON, or a failure naming why GitHub withheld it.
fn release(version: &str, root: &Path, tmp: &Path) -> Value {
    // A file, not stdout: curl rewrites it on a retry but would append a retried body to stdout.
    let file = tmp.join("release.json");
    let status = run(
        Command::new("curl")
            .args(CURL)
            .args(["-w", "%{http_code}", "-o"])
            .arg(&file)
            .args(["-H", "Accept: application/vnd.github+json"])
            .arg(format!("{RELEASES}/v{version}")),
        root,
    );
    let body = std::fs::read_to_string(&file).unwrap_or_default();
    let _ = std::fs::remove_file(&file);
    let body = body.as_str();
    let message = || {
        serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_string))
            .unwrap_or_default()
    };
    match status.trim() {
        "200" => serde_json::from_str(body)
            .unwrap_or_else(|e| fail(root, &format!("the v{version} release is not JSON: {e}"))),
        // Not transient, so not `fail`: a re-run never finds a release that doesn't exist.
        "404" => panic!(
            "hyprlayer {version} has no GitHub release (a local build?); the sandbox seeds a \
             released CLI's agent bundle, so install a released CLI"
        ),
        code @ ("403" | "429") => fail(
            root,
            &format!("GitHub refused the v{version} lookup ({code}, likely its unauthenticated rate limit): {}", message()),
        ),
        code => fail(
            root,
            &format!("GitHub answered {code} for the v{version} release: {}", message()),
        ),
    }
}

fn run(command: &mut Command, root: &Path) -> String {
    let output = command
        .output()
        .unwrap_or_else(|e| fail(root, &format!("{command:?}: {e}")));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        fail(root, &format!("{command:?} failed: {}", stderr.trim()));
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn fail(root: &Path, what: &str) -> ! {
    panic!(
        "{what}\nfilling {} needs the network once per CLI version; re-run to retry",
        root.display()
    )
}
