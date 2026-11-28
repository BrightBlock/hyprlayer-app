//! Which profile this process is: its identifier, dirs, announced identity and server target.

use std::collections::BTreeMap;
use std::path::PathBuf;

use hyprlayer_client::client_identity::{self, ClientIdentity};
use hyprlayer_client::hosts::{self, Target};
use hyprlayer_client::paths::AppPaths;
use hyprlayer_client::ssh::{Ssh, SshSetup};
use hyprlayer_server::APP_IDENTIFIER;
use hyprlayer_ui::theme::Mode;

/// The Tauri dev app's identifier: both dev clients share one profile until G14 (ADR-0013).
pub fn default_identifier() -> String {
    format!("{APP_IDENTIFIER}.dev")
}

/// How this process was launched. `HYPRLAYER_APP_*_DIR` are not read: a shell may export the
/// release profile's, and the supervisor would then evict that profile's server.
#[derive(Debug, Default)]
pub struct Launch {
    pub identifier: Option<String>,
    pub profile_dir: Option<PathBuf>,
    pub attach: Option<PathBuf>,
}

pub const USAGE: &str = "usage: hyprlayer-native [--identifier <id>] [--profile-dir <dir>] [--attach <server-data-dir>]
  --identifier   profile identifier (default ai.brightblock.hyprlayer.dev)
  --profile-dir  keep config, data and logs under <dir>/{config,data,logs} instead of the identifier's dirs
  --attach       attach to the server already running on that data dir; never spawn or replace one";

impl Launch {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut launch = Self::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
            match flag.as_str() {
                "--identifier" => launch.identifier = Some(value()?),
                "--profile-dir" => launch.profile_dir = Some(value()?.into()),
                "--attach" => launch.attach = Some(value()?.into()),
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(launch)
    }

    pub fn identifier(&self) -> String {
        self.identifier.clone().unwrap_or_else(default_identifier)
    }

    pub fn paths(&self) -> Option<AppPaths> {
        match &self.profile_dir {
            Some(dir) => Some(AppPaths {
                config: dir.join("config"),
                data: dir.join("data"),
                log: dir.join("logs"),
            }),
            None => AppPaths::resolve(&self.identifier()),
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub identifier: String,
    pub paths: AppPaths,
    pub identity: ClientIdentity,
    pub target: Target,
    /// Batch only: nothing here can answer an ssh prompt yet.
    pub ssh: Ssh,
}

impl Config {
    /// `target` overrides `hosts.json`, which is read once: a later host switch needs a relaunch.
    pub fn new(
        identifier: String,
        paths: AppPaths,
        target: Option<Target>,
    ) -> std::io::Result<Self> {
        for dir in [&paths.config, &paths.data, &paths.log] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(Self {
            identity: client_identity::load_or_create(&paths.config),
            target: target.unwrap_or_else(|| hosts::target(&paths.config)),
            ssh: Ssh::new(SshSetup::for_app(&paths.data)).batch(),
            identifier,
            paths,
        })
    }
}

/// The vars a daemonized server has none of on its own (ADR-0009 §7), as `attach_client` sends
/// them. A remote target omits the two that name this machine.
pub fn env_overlay(target: &Target, mode: Mode) -> BTreeMap<String, String> {
    let local = !matches!(target, Target::Remote(_));
    let mut overlay = BTreeMap::new();
    for key in ["SSH_AUTH_SOCK", "DISPLAY", "TERM", "LANG"] {
        if !local && matches!(key, "SSH_AUTH_SOCK" | "DISPLAY") {
            continue;
        }
        if let Ok(value) = std::env::var(key) {
            overlay.insert(key.to_string(), value);
        }
    }
    for (key, value) in std::env::vars() {
        if key.starts_with("LC_") {
            overlay.insert(key, value);
        }
    }
    overlay.extend([colorfgbg(mode)]);
    overlay
}

/// rxvt's "fg;bg" palette indices, which claude's "auto" theme and vim read before any OSC 11 reply.
pub fn colorfgbg(mode: Mode) -> (String, String) {
    let value = match mode {
        Mode::Dark => "15;0",
        Mode::Light => "0;15",
    };
    ("COLORFGBG".to_string(), value.to_string())
}
