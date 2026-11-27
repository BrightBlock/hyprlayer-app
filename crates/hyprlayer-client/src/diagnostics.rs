//! What a bug report needs: the versions, the connection, and the tail of
//! every log on both machines, with the home directory masked.

use std::path::{Path, PathBuf};

use hyprlayer_server::logging as server_logging;

use crate::hosts::Target;
use crate::provision;
use crate::server_version::{MIN_SERVER_VERSION, SERVER_VERSION};
use crate::ssh::Ssh;
use crate::supervisor;

const TAIL_BYTES: u64 = 64 * 1024;

pub struct App<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub identifier: &'a str,
}

pub struct Connection {
    pub target: Target,
    pub state: String,
    pub server_release: Option<String>,
    pub server_protocol: Option<u32>,
    pub server_outdated: bool,
    pub error: Option<(String, String)>,
}

pub struct Report<'a> {
    pub app: App<'a>,
    pub connection: Connection,
    pub client_log: PathBuf,
    pub app_log_dir: PathBuf,
    pub app_data_dir: PathBuf,
}

pub async fn build(report: Report<'_>, ssh: &Ssh) -> String {
    let Report {
        app,
        connection,
        client_log,
        app_log_dir,
        app_data_dir,
    } = report;
    let mut out = format!(
        "{} {} ({}, {} build) on {}/{}, protocol {}, ships server {} (needs {})\n",
        app.name,
        app.version,
        app.identifier,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        std::env::consts::OS,
        std::env::consts::ARCH,
        hyprlayer_proto::PROTOCOL_VERSION,
        SERVER_VERSION,
        MIN_SERVER_VERSION,
    );
    out.push_str(&format!(
        "server: {}, {}, version {}, protocol {}{}\n",
        connection.target.name(),
        connection.state,
        connection.server_release.as_deref().unwrap_or("unknown"),
        connection
            .server_protocol
            .map_or("unknown".to_string(), |v| v.to_string()),
        if connection.server_outdated {
            ", not the server this app ships"
        } else {
            ""
        }
    ));
    if let Some((code, message)) = &connection.error {
        out.push_str(&format!("connection error: {code}: {message}\n"));
    }

    out.push_str(&format!("\n== {} ==\n", client_log.display()));
    out.push_str(&server_logging::tail_file(&client_log, TAIL_BYTES));
    out.push_str("\n== server ==\n");
    out.push_str(&server_logs(&connection.target, &app_log_dir, &app_data_dir, ssh).await);
    mask(&out)
}

async fn server_logs(
    target: &Target,
    own_log_dir: &Path,
    app_data_dir: &Path,
    ssh: &Ssh,
) -> String {
    match target {
        Target::Own => server_logging::tail_logs(own_log_dir, TAIL_BYTES),
        Target::LocalProfile(data_dir) => {
            server_logging::tail_logs(&server_logging::log_dir(data_dir), TAIL_BYTES)
        }
        Target::Remote(host) => {
            // A stale install cannot be relayed through but can still print logs.
            let version =
                match provision::readiness(host, app_data_dir, ssh).await {
                    Ok(found) => found
                        .usable_version
                        .or((found.state == provision::HostServerState::Stale)
                            .then_some(found.version)),
                    Err(e) => {
                        return format!(
                            "could not inspect the server on {}: {}: {}\n",
                            host.destination, e.code, e.message
                        )
                    }
                };
            let Some(version) = version else {
                return format!("no usable server is installed on {}\n", host.destination);
            };
            match supervisor::ssh_capture(host, &provision::logs_command(&version), ssh).await {
                Ok(text) => format!("on {}:\n{text}", host.destination),
                Err(e) => format!(
                    "could not read the server logs on {}: {}: {}\n",
                    host.destination, e.code, e.message
                ),
            }
        }
    }
}

/// A report gets pasted into issues and chats: no home path, no URL tokens.
fn mask(text: &str) -> String {
    let text = hyprlayer_core::redact::url_credentials(text);
    match dirs::home_dir().map(|home| home.display().to_string()) {
        Some(home) if !home.is_empty() => text.replace(&home, "~"),
        _ => text,
    }
}
