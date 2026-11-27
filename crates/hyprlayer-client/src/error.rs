use hyprlayer_core::CoreError;
use hyprlayer_proto::{StaleTranscript, WireError};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub remedy: Option<String>,
    #[serde(default)]
    #[specta(optional)]
    pub stale_transcript: Option<StaleTranscriptDto>,
}

// Hand-written: `skip_serializing_if` makes specta split the binding in two.
// Omitting a `None` detail keeps every other envelope byte-identical.
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let len = if self.stale_transcript.is_some() {
            4
        } else {
            3
        };
        let mut s = serializer.serialize_struct("AppError", len)?;
        s.serialize_field("code", &self.code)?;
        s.serialize_field("message", &self.message)?;
        s.serialize_field("remedy", &self.remedy)?;
        if let Some(stale) = &self.stale_transcript {
            s.serialize_field("staleTranscript", stale)?;
        }
        s.end()
    }
}

/// `StaleTranscript` across Tauri: specta exports no `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StaleTranscriptDto {
    pub rev: f64,
    pub committed_rev: f64,
    pub missing_turns: f64,
}

impl From<StaleTranscript> for StaleTranscriptDto {
    fn from(s: StaleTranscript) -> Self {
        Self {
            rev: s.rev as f64,
            committed_rev: s.committed_rev as f64,
            missing_turns: s.missing_turns as f64,
        }
    }
}

impl From<StaleTranscriptDto> for StaleTranscript {
    fn from(dto: StaleTranscriptDto) -> Self {
        Self {
            rev: dto.rev as u64,
            committed_rev: dto.committed_rev as u64,
            missing_turns: dto.missing_turns as u64,
        }
    }
}

impl From<CoreError> for AppError {
    fn from(err: CoreError) -> Self {
        Self {
            code: err.code().to_string(),
            message: err.to_string(),
            remedy: err.remedy(),
            stale_transcript: None,
        }
    }
}

impl From<WireError> for AppError {
    fn from(err: WireError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            remedy: err.remedy,
            stale_transcript: err.stale_transcript.map(StaleTranscriptDto::from),
        }
    }
}

impl From<AppError> for WireError {
    fn from(err: AppError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            remedy: err.remedy,
            stale_transcript: err.stale_transcript.map(StaleTranscript::from),
        }
    }
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
            remedy: None,
            stale_transcript: None,
        }
    }

    pub fn project_not_found(path: &str) -> Self {
        Self {
            code: "project_not_found".into(),
            message: format!("No hyprlayer project is mapped at {path}"),
            remedy: Some("Add the repository as a project first.".into()),
            stale_transcript: None,
        }
    }

    /// The host does not have the server this build needs: nothing at the
    /// version-keyed path, a binary there whose bytes are not ours, or an ssh
    /// exit naming a missing command. One code, because it is one condition and
    /// one remedy — Phase 7 turns it into the trigger for provisioning rather
    /// than a dead end, and the panel names which of the three it was.
    pub fn remote_cli_missing(host: &str) -> Self {
        Self {
            code: "remote_cli_missing".into(),
            message: format!("{host} does not have the Hyprlayer server this build needs."),
            remedy: Some("Install it from the Hyprlayer server panel.".into()),
            stale_transcript: None,
        }
    }

    /// The server answered but is older than `MIN_SERVER_VERSION`; its fix
    /// ends the sessions on it, so it is offered rather than done on connect.
    pub fn remote_server_outdated(host: &str, running: &str, minimum: &str) -> Self {
        Self {
            code: "remote_server_outdated".into(),
            message: format!(
                "{host} is running Hyprlayer server {running}; this app needs {minimum} or newer."
            ),
            remedy: Some("Update it from the Hyprlayer server panel.".into()),
            stale_transcript: None,
        }
    }

    /// The host runs something none of ADR-0008's three targets covers, so
    /// there is no binary to install on it (plan Phase 7 §2).
    pub fn remote_platform_unsupported(host: &str, uname: &str) -> Self {
        Self {
            code: "remote_platform_unsupported".into(),
            message: format!("{host} reports an unsupported platform: {uname}."),
            remedy: Some(
                "Hyprlayer's server ships for Linux x86_64, macOS arm64, and Windows x86_64."
                    .into(),
            ),
            stale_transcript: None,
        }
    }

    /// ssh reached the host but the connection was refused on credentials —
    /// a bad key, agent, or host-key check (plan Phase 1 §1).
    pub fn remote_auth_failed(host: &str, detail: impl Into<String>) -> Self {
        Self {
            code: "remote_auth_failed".into(),
            message: format!("Could not authenticate to {host}: {}", detail.into()),
            remedy: Some("Check your SSH credentials.".into()),
            stale_transcript: None,
        }
    }

    /// A run that may not ask, such as the launch connect, needed the user: a
    /// password, a passphrase, or trust in a host key it has not seen.
    pub fn remote_sign_in_required(host: &str) -> Self {
        Self {
            code: "remote_sign_in_required".into(),
            message: format!("{host} needs you to sign in."),
            remedy: None,
            stale_transcript: None,
        }
    }

    pub fn remote_sign_in_cancelled(host: &str, timed_out: bool) -> Self {
        Self {
            code: "remote_sign_in_cancelled".into(),
            message: if timed_out {
                format!("Signing in to {host} timed out waiting for an answer.")
            } else {
                format!("Signing in to {host} was cancelled.")
            },
            remedy: None,
            stale_transcript: None,
        }
    }

    /// Never offered for trust in the app: ssh refuses a changed key outright.
    /// `key_host` is the name `known_hosts` files it under.
    pub fn remote_host_key_changed(host: &str, key_host: &str, known_hosts: Option<&str>) -> Self {
        let default = dirs::home_dir().map(|home| home.join(".ssh").join("known_hosts"));
        let file = known_hosts
            .filter(|file| default.as_deref() != Some(std::path::Path::new(file)))
            .map(|file| format!(" -f {}", shell_quote(file)))
            .unwrap_or_default();
        Self {
            code: "remote_host_key_changed".into(),
            message: format!(
                "The host key for {host} has changed. A rebuilt host does this, and so does \
                 someone intercepting the connection."
            ),
            remedy: Some(format!(
                "If you expected it, remove the old key with `ssh-keygen -R {}{file}` \
                 and connect again.",
                shell_quote(key_host)
            )),
            stale_transcript: None,
        }
    }

    /// ssh could not reach the host at all, or could not even spawn on this
    /// machine (plan Phase 1 §1). Also the fallback arm of
    /// `supervisor::classify_ssh_failure` — a stderr this doesn't recognise
    /// still lands here rather than as a silent `internal`.
    pub fn remote_unreachable(host: &str, detail: impl Into<String>) -> Self {
        Self {
            code: "remote_unreachable".into(),
            message: format!("Could not reach {host}: {}", detail.into()),
            // No remedy: `message` carries ssh's own words, and this is also the
            // fallback arm, where a specific instruction would be a guess.
            remedy: None,
            stale_transcript: None,
        }
    }

    /// A command ran on the host and failed. Its stderr belongs to that remote
    /// program, not to ssh, so it is reported verbatim rather than read as a
    /// credential problem or a reachability one.
    pub fn remote_command_failed(host: &str, detail: impl Into<String>) -> Self {
        Self {
            code: "remote_command_failed".into(),
            message: format!("A command failed on {host}: {}", detail.into()),
            remedy: None,
            stale_transcript: None,
        }
    }

    /// The server binary for this version could not be fetched — no published
    /// asset, or the network refused. An expected condition rather than a
    /// fault, so it gets a code that can be switched on and a remedy naming the
    /// way out: `internal` carried neither (plan Phase 7, found in manual
    /// verification 2026-09-16).
    pub fn server_download_failed(version: &str, triple: &str, detail: impl Into<String>) -> Self {
        Self {
            code: "server_download_failed".into(),
            message: format!(
                "Could not download hyprlayer-server {version} for {triple}: {}",
                detail.into()
            ),
            remedy: Some(
                "No release publishes this version's server yet — stage one with scripts/stage-server-binary.sh."
                    .into(),
            ),
            stale_transcript: None,
        }
    }

    /// The active server changed since this window opened, so reconnecting in
    /// place would attach it to another profile's data. The localStorage
    /// namespace is fixed when the window is created (plan Phase 4 §2), so
    /// relaunching is the only honest answer here — and the only place this
    /// phase still asks for one (Phase 8 §4).
    pub fn host_changed(host: &str) -> Self {
        Self {
            code: "host_changed".into(),
            message: format!("The active Hyprlayer server is now {host}."),
            remedy: Some("Relaunch Hyprlayer to connect to it.".into()),
            stale_transcript: None,
        }
    }

    /// The *local* server could not be started or reached. `detail` is the
    /// same message text `Outcome::Blocked` used to show verbatim, so it
    /// already carries its own remedy where one applies (plan Phase 1 §1-§2).
    pub fn server_unavailable(detail: impl Into<String>) -> Self {
        Self {
            code: "server_unavailable".into(),
            message: detail.into(),
            remedy: None,
            stale_transcript: None,
        }
    }

    pub fn server_unavailable_with(message: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            remedy: Some(remedy.into()),
            ..Self::server_unavailable(message)
        }
    }
}

pub type CmdResult<T> = Result<T, AppError>;

/// Both halves come from ssh config values, and the remedy is meant to be pasted.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V0_7_APP_ERRORS_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/compat/v0_7/app-errors.json"
    ));

    fn pinned(slice: &str) -> (String, String, Option<String>) {
        let fixture: serde_json::Value = serde_json::from_str(V0_7_APP_ERRORS_FIXTURE).unwrap();
        let entry = &fixture[slice];
        (
            entry["code"].as_str().unwrap().to_string(),
            entry["message"].as_str().unwrap().to_string(),
            entry["remedy"].as_str().map(str::to_string),
        )
    }

    #[test]
    fn internal_carries_no_remedy() {
        assert_eq!(AppError::internal("boom").remedy, None);
    }

    #[test]
    fn wire_error_round_trips_through_app_error() {
        let (code, message, remedy) = pinned("providersSessions");
        let wire = WireError {
            code: code.clone(),
            message: message.clone(),
            remedy: remedy.clone(),
            stale_transcript: None,
        };
        let app = AppError::from(wire);
        assert_eq!(
            (app.code.clone(), app.message.clone(), app.remedy.clone()),
            (code.clone(), message.clone(), remedy.clone())
        );
        let back = WireError::from(app);
        assert_eq!(
            (back.code, back.message, back.remedy),
            (code, message, remedy)
        );
    }
}
