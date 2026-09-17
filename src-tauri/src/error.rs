use hyprlayer_core::CoreError;
use hyprlayer_proto::WireError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub remedy: Option<String>,
}

impl From<CoreError> for AppError {
    fn from(err: CoreError) -> Self {
        Self {
            code: err.code().to_string(),
            message: err.to_string(),
            remedy: err.remedy(),
        }
    }
}

impl From<WireError> for AppError {
    fn from(err: WireError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            remedy: err.remedy,
        }
    }
}

impl From<AppError> for WireError {
    fn from(err: AppError) -> Self {
        Self {
            code: err.code,
            message: err.message,
            remedy: err.remedy,
        }
    }
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".into(),
            message: message.into(),
            remedy: None,
        }
    }

    pub fn project_not_found(path: &str) -> Self {
        Self {
            code: "project_not_found".into(),
            message: format!("No hyprlayer project is mapped at {path}"),
            remedy: Some("Add the repository as a project first.".into()),
        }
    }

    pub fn thoughts_not_configured() -> Self {
        Self {
            code: "config_missing".into(),
            message: "Thoughts are not configured yet.".into(),
            remedy: Some("Add the repository as a project first.".into()),
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
        }
    }

    /// ssh reached the host but the connection was refused on credentials —
    /// a bad key, agent, or host-key check (plan Phase 1 §1).
    pub fn remote_auth_failed(host: &str, detail: impl Into<String>) -> Self {
        Self {
            code: "remote_auth_failed".into(),
            message: format!("Could not authenticate to {host}: {}", detail.into()),
            remedy: Some("Check your SSH credentials.".into()),
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
        }
    }
}

pub type CmdResult<T> = Result<T, AppError>;

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
    fn binary_missing_maps_to_the_pinned_v0_7_fixture() {
        let err = AppError::from(CoreError::BinaryMissing {
            name: "claude".into(),
        });
        assert_eq!((err.code, err.message, err.remedy), pinned("binaries"));
    }

    #[test]
    fn config_missing_carries_no_extra_punctuation() {
        let err = AppError::from(CoreError::ConfigMissing {
            path: "/nonexistent-hyprlayer-config-fixture/hyprlayer/config.json".into(),
        });
        let (code, message, remedy) = pinned("settingsConfig");
        assert_eq!((err.code, err.message, err.remedy), (code, message, remedy));
    }

    #[test]
    fn project_not_found_is_a_public_constructor() {
        let err = AppError::project_not_found("/tmp/example-project");
        let (code, message, remedy) = pinned("projects");
        assert_eq!((err.code, err.message, err.remedy), (code, message, remedy));
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
