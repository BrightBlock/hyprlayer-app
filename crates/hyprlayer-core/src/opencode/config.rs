//! Reading and writing OpenCode's `auth.json` and `opencode.json`.
//!
//! OpenCode follows XDG conventions on every platform (including macOS — it does
//! NOT use `~/Library/Application Support`), so we resolve paths against
//! `$XDG_*` / `~/.config` / `~/.local/share` explicitly rather than via
//! `dirs::config_dir()`. Both files are user/CLI-owned: we merge-preserve and
//! never drop fields or keys we didn't author.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{CoreError, Result};

const CONFIG_SCHEMA_URL: &str = "https://opencode.ai/config.json";

/// One configured provider credential, as surfaced to the UI. The raw key is
/// deliberately never returned — only that a credential exists and its kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OpencodeAuthEntry {
    /// models.dev provider id (e.g. `"anthropic"`, `"openai"`).
    pub provider: String,
    /// The stored credential `type`: `"api"`, `"oauth"`, `"wellknown"`, …
    pub kind: String,
}

/// Path to OpenCode's `auth.json` (`$OPENCODE_AUTH_JSON`, else
/// `$XDG_DATA_HOME/opencode/auth.json`, else `~/.local/share/opencode/auth.json`).
pub fn auth_json_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OPENCODE_AUTH_JSON") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")))?;
    Some(base.join("opencode").join("auth.json"))
}

/// Path to OpenCode's `opencode.json` (`$OPENCODE_CONFIG`, else
/// `$XDG_CONFIG_HOME/opencode/opencode.json`, else `~/.config/opencode/opencode.json`).
pub fn config_json_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OPENCODE_CONFIG") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))?;
    Some(base.join("opencode").join("opencode.json"))
}

/// List configured providers from `auth.json` (never the raw keys). Missing or
/// empty file ⇒ empty list; a malformed file is an error so the UI can surface it.
pub fn list_auth() -> Result<Vec<OpencodeAuthEntry>> {
    let Some(path) = auth_json_path() else {
        return Ok(Vec::new());
    };
    let map = read_object(&path, "opencode auth.json")?;
    let mut out: Vec<OpencodeAuthEntry> = map
        .into_iter()
        .map(|(provider, v)| {
            let kind = v
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("api")
                .to_string();
            OpencodeAuthEntry { provider, kind }
        })
        .collect();
    out.sort_by(|a, b| a.provider.cmp(&b.provider));
    Ok(out)
}

/// Upsert an API key for `provider` in `auth.json`, preserving every other
/// provider. Creates the file (and parents) with `0600` if absent.
pub fn set_provider_key(provider: &str, key: &str) -> Result<()> {
    let path = auth_json_path().ok_or_else(|| no_home("auth.json"))?;
    let mut map = read_object_or_empty(&path, "opencode auth.json")?;
    let entry = serde_json::json!({ "type": "api", "key": key });
    // OpenCode normalizes provider ids by stripping a trailing slash.
    map.insert(provider.trim_end_matches('/').to_string(), entry);
    write_json_atomic(&path, &Value::Object(map), 0o600)
}

/// Remove a provider's credential from `auth.json` (no-op if absent).
pub fn remove_provider(provider: &str) -> Result<()> {
    let path = auth_json_path().ok_or_else(|| no_home("auth.json"))?;
    if !path.exists() {
        return Ok(());
    }
    let mut map = read_object_or_empty(&path, "opencode auth.json")?;
    map.remove(provider.trim_end_matches('/'));
    write_json_atomic(&path, &Value::Object(map), 0o600)
}

/// Mirror the default model into `opencode.json`'s `model` field
/// (`"provider/model"`), merge-preserving. Best-effort: if the existing file is
/// JSONC/commented (not strict-JSON parseable) we leave it untouched rather than
/// risk dropping the user's comments — the driver still passes the model per
/// request, so this is only for `opencode` run standalone.
pub fn set_default_model(model: Option<&str>) -> Result<()> {
    let Some(path) = config_json_path() else {
        return Ok(());
    };
    let mut obj = match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(Value::Object(m)) => m,
            // Non-object or unparseable (JSONC) — don't clobber the user's file.
            Ok(_) | Err(_) => return Ok(()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e.into()),
    };
    if !obj.contains_key("$schema") {
        obj.insert("$schema".into(), Value::String(CONFIG_SCHEMA_URL.into()));
    }
    match model {
        Some(m) if !m.is_empty() => {
            obj.insert("model".into(), Value::String(m.to_string()));
        }
        _ => {
            obj.remove("model");
        }
    }
    write_json_atomic(&path, &Value::Object(obj), 0o644)
}

/* ----------------------------- helpers --------------------------------- */

fn no_home(what: &str) -> CoreError {
    CoreError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("cannot resolve OpenCode {what} path (no home directory)"),
    ))
}

/// Read a JSON object file; missing ⇒ empty map, malformed ⇒ error.
fn read_object(path: &Path, label: &str) -> Result<Map<String, Value>> {
    match std::fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => Ok(Map::new()),
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(Value::Object(m)) => Ok(m),
            Ok(_) => Ok(Map::new()),
            Err(e) => Err(CoreError::OutputParse {
                program: label.to_string(),
                detail: e.to_string(),
            }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e.into()),
    }
}

/// Like [`read_object`] but tolerates a malformed file by starting fresh — used
/// on the write path so a corrupt `auth.json` can be repaired rather than wedge
/// the user out of adding a key.
fn read_object_or_empty(path: &Path, label: &str) -> Result<Map<String, Value>> {
    Ok(read_object(path, label).unwrap_or_default())
}

fn write_json_atomic(path: &Path, value: &Value, mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| CoreError::OutputParse {
        program: "opencode config".into(),
        detail: e.to_string(),
    })?;
    // Unique temp name per write so two concurrent writers to the same file
    // (e.g. an add-key racing a remove-provider) don't clobber each other's
    // staging file. The final `rename` is still atomic per file; last writer
    // wins on the destination, but neither corrupts the other's temp.
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{seq}", std::process::id()));
    std::fs::write(&tmp, json.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    let _ = mode; // (mode is unix-only; silence the unused warning elsewhere)
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp HOME so the XDG fallbacks resolve into an isolated dir.
    struct TmpHome {
        dir: PathBuf,
        prev_home: Option<std::ffi::OsString>,
        prev_data: Option<std::ffi::OsString>,
        prev_config: Option<std::ffi::OsString>,
        prev_auth: Option<std::ffi::OsString>,
        prev_cfg: Option<std::ffi::OsString>,
    }
    impl TmpHome {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("hl-opencode-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let s = Self {
                prev_home: std::env::var_os("HOME"),
                prev_data: std::env::var_os("XDG_DATA_HOME"),
                prev_config: std::env::var_os("XDG_CONFIG_HOME"),
                prev_auth: std::env::var_os("OPENCODE_AUTH_JSON"),
                prev_cfg: std::env::var_os("OPENCODE_CONFIG"),
                dir: dir.clone(),
            };
            std::env::set_var("HOME", &dir);
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("OPENCODE_AUTH_JSON");
            std::env::remove_var("OPENCODE_CONFIG");
            s
        }
    }
    impl Drop for TmpHome {
        fn drop(&mut self) {
            restore("HOME", &self.prev_home);
            restore("XDG_DATA_HOME", &self.prev_data);
            restore("XDG_CONFIG_HOME", &self.prev_config);
            restore("OPENCODE_AUTH_JSON", &self.prev_auth);
            restore("OPENCODE_CONFIG", &self.prev_cfg);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    fn restore(key: &str, prev: &Option<std::ffi::OsString>) {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    // One serial test: these mutate process-wide env (HOME/XDG) and would race
    // each other under cargo's default parallel test threads.
    #[test]
    fn auth_and_config_io() {
        let _h = TmpHome::new("io");

        // --- auth.json ---
        assert!(list_auth().unwrap().is_empty());

        set_provider_key("anthropic", "sk-ant-1").unwrap();
        set_provider_key("openai/", "sk-oai-2").unwrap(); // trailing slash normalized

        let entries = list_auth().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.provider == "anthropic" && e.kind == "api"));
        assert!(entries.iter().any(|e| e.provider == "openai"));

        // The raw key is on disk but never returned by list_auth.
        let auth_path = auth_json_path().unwrap();
        let raw = std::fs::read_to_string(&auth_path).unwrap();
        assert!(raw.contains("sk-ant-1") && raw.contains("sk-oai-2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&auth_path).unwrap().mode() & 0o777, 0o600);
        }

        remove_provider("anthropic").unwrap();
        let entries = list_auth().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider, "openai");

        // --- opencode.json ---
        set_default_model(Some("anthropic/claude-sonnet-4-5")).unwrap();
        let path = config_json_path().unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["model"], "anthropic/claude-sonnet-4-5");
        assert_eq!(v["$schema"], CONFIG_SCHEMA_URL);

        // Pre-existing unrelated keys survive a model update.
        let mut obj = v.as_object().unwrap().clone();
        obj.insert("theme".into(), Value::String("dark".into()));
        std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(obj)).unwrap()).unwrap();
        set_default_model(Some("openai/gpt-4o")).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["model"], "openai/gpt-4o");
        assert_eq!(v["theme"], "dark");

        set_default_model(None).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v.get("model").is_none());
        assert_eq!(v["theme"], "dark");
    }
}
