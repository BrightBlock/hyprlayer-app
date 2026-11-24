use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::error::{CoreError, Result};

const CONFIG_SCHEMA_URL: &str = "https://opencode.ai/config.json";

const OLLAMA_DEFAULT_NUM_CTX: u64 = 32768;

pub fn opencode_model_context_window(model_id: &str) -> Option<u32> {
    if model_id.starts_with("ollama/") {
        Some(OLLAMA_DEFAULT_NUM_CTX as u32)
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OpencodeAuthEntry {
    pub provider: String,
    pub kind: String,
    /// Set when OpenCode 2 cannot use the entry: a v1 sign-in, which records no sign-in method.
    pub remedy: Option<String>,
}

/// The `auth.json` entries hyprlayer hands its OpenCode server: API keys by provider, and the
/// providers whose sign-in OpenCode 2 cannot take.
pub struct ApiKeys {
    pub keys: std::collections::BTreeMap<String, String>,
    pub unusable: Vec<String>,
}

fn unusable_remedy(_provider: &str) -> String {
    "OpenCode 2 can't use this sign-in. Sign in again here, or add an API key.".into()
}

/// A provider sign-in OpenCode offers (OAuth: a browser or a device code).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OpencodeSignInMethod {
    pub provider: String,
    pub method_id: String,
    pub label: String,
}

/// A sign-in under way: the page to open, what to tell the user, and whether a pasted
/// address or code can finish it (a browser that cannot reach the server's loopback).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OpencodeSignInAttempt {
    pub provider: String,
    pub attempt_id: String,
    pub url: String,
    pub instructions: String,
    /// `code`: the user pastes a code; `auto`: OpenCode finishes it, or a pasted address does.
    pub needs_code: bool,
    pub accepts_paste: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum OpencodeSignInStatus {
    Pending,
    Complete,
    Failed { message: String },
}

/// The providers signed in through hyprlayer's OpenCode server, whose credentials live in its
/// own data dir. Kept apart from `auth.json`, which OpenCode 1 and other tools read too.
pub fn list_sign_ins(path: &Path) -> Vec<String> {
    std::fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

fn set_listed(path: &Path, provider: &str, listed: bool) -> Result<()> {
    let _guard = write_lock();
    let mut all = list_sign_ins(path);
    all.retain(|p| p != provider);
    if listed {
        all.push(provider.to_string());
        all.sort();
    }
    write_json_atomic(path, &json!(all), 0o600)
}

/// hyprlayer's record of its sign-ins: the providers signed in, and those with a sign-in begun
/// whose outcome it has not read yet. OpenCode stores the credential the moment the browser
/// finishes, whoever is watching, so a pending provider's sign-in credential is the user's.
#[derive(Debug, Clone)]
pub struct SignInRecords {
    pub signed_in: PathBuf,
    pub pending: PathBuf,
}

impl SignInRecords {
    pub fn signed_in(&self) -> Vec<String> {
        list_sign_ins(&self.signed_in)
    }

    pub fn pending(&self) -> Vec<String> {
        list_sign_ins(&self.pending)
    }

    pub fn begun(&self, provider: &str) -> Result<()> {
        set_listed(&self.pending, provider, true)
    }

    pub fn completed(&self, provider: &str) -> Result<()> {
        set_listed(&self.signed_in, provider, true)?;
        set_listed(&self.pending, provider, false)
    }

    pub fn abandoned(&self, provider: &str) -> Result<()> {
        set_listed(&self.pending, provider, false)
    }

    pub fn removed(&self, provider: &str) -> Result<()> {
        set_listed(&self.signed_in, provider, false)?;
        set_listed(&self.pending, provider, false)
    }
}

/// `auth.json`'s entries with the sign-ins made through hyprlayer: a provider signed in that way
/// reads as such, in place of an `auth.json` entry OpenCode 2 cannot use.
pub fn list_auth_with_sign_ins(
    auth: &Path,
    sign_ins: &SignInRecords,
) -> Result<Vec<OpencodeAuthEntry>> {
    let signed = sign_ins.signed_in();
    let mut out: Vec<OpencodeAuthEntry> = list_auth(auth)?
        .into_iter()
        .filter(|e| !(e.remedy.is_some() && signed.contains(&e.provider)))
        .collect();
    for provider in signed {
        if !out.iter().any(|e| e.provider == provider) {
            out.push(OpencodeAuthEntry {
                provider,
                kind: "signIn".into(),
                remedy: None,
            });
        }
    }
    out.sort_by(|a, b| a.provider.cmp(&b.provider));
    Ok(out)
}

pub fn api_keys(path: &Path) -> Result<ApiKeys> {
    let mut keys = std::collections::BTreeMap::new();
    let mut unusable = Vec::new();
    for (provider, entry) in read_object(path, "opencode auth.json")? {
        let kind = entry.get("type").and_then(Value::as_str).unwrap_or("api");
        match (kind, entry.get("key").and_then(Value::as_str)) {
            ("api", Some(key)) if !key.is_empty() => {
                keys.insert(provider, key.to_string());
            }
            _ => unusable.push(provider),
        }
    }
    Ok(ApiKeys { keys, unusable })
}

pub fn list_auth(path: &Path) -> Result<Vec<OpencodeAuthEntry>> {
    let map = read_object(path, "opencode auth.json")?;
    let mut out: Vec<OpencodeAuthEntry> = map
        .into_iter()
        .map(|(provider, v)| {
            let kind = v
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("api")
                .to_string();
            let keyed = kind == "api"
                && v.get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|k| !k.is_empty());
            let remedy = (!keyed).then(|| unusable_remedy(&provider));
            OpencodeAuthEntry {
                provider,
                kind,
                remedy,
            }
        })
        .collect();
    out.sort_by(|a, b| a.provider.cmp(&b.provider));
    Ok(out)
}

pub fn set_provider_key(path: &Path, provider: &str, key: &str) -> Result<()> {
    let _guard = write_lock();
    // A parse failure must not read as empty: the rewrite would erase every other key.
    let mut map = read_object(path, "opencode auth.json")?;
    let entry = serde_json::json!({ "type": "api", "key": key });
    map.insert(provider.trim_end_matches('/').to_string(), entry);
    write_json_atomic(path, &Value::Object(map), 0o600)
}

pub fn remove_provider(path: &Path, provider: &str) -> Result<()> {
    let _guard = write_lock();
    if !path.exists() {
        return Ok(());
    }
    let mut map = read_object(path, "opencode auth.json")?;
    map.remove(provider.trim_end_matches('/'));
    write_json_atomic(path, &Value::Object(map), 0o600)
}

pub fn set_default_model(path: &Path, model: Option<&str>) -> Result<()> {
    let _guard = write_lock();
    let mut obj = match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(Value::Object(m)) => m,
            // Rewriting JSONC would discard the user's comments.
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
    write_json_atomic(path, &Value::Object(obj), 0o644)
}

pub fn clear_default_model_for_provider(path: &Path, provider: &str) -> Result<bool> {
    let _guard = write_lock();
    let mut obj = match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(Value::Object(m)) => m,
            // Rewriting JSONC would discard the user's comments.
            Ok(_) | Err(_) => return Ok(false),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let matches_provider = obj
        .get("model")
        .and_then(Value::as_str)
        .and_then(|model| model.split('/').next())
        .is_some_and(|configured| configured.eq_ignore_ascii_case(provider));
    if !matches_provider {
        return Ok(false);
    }
    obj.remove("model");
    write_json_atomic(path, &Value::Object(obj), 0o644)?;
    Ok(true)
}

pub fn set_ollama_provider(path: &Path, base_url: &str, models: &[String]) -> Result<()> {
    let _guard = write_lock();
    let mut obj = match std::fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => Map::new(),
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(Value::Object(m)) => m,
            Ok(_) | Err(_) => {
                tracing::warn!(
                    path = %path.display(),
                    "opencode.json is not strict JSON; skipped Ollama provider sync"
                );
                return Ok(());
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e.into()),
    };
    if !obj.contains_key("$schema") {
        obj.insert("$schema".into(), Value::String(CONFIG_SCHEMA_URL.into()));
    }

    let mut providers = match obj.remove("provider") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };

    if models.is_empty() {
        providers.remove("ollama");
    } else {
        let base = base_url.trim_end_matches('/');
        let mut model_map = Map::new();
        for id in models {
            model_map.insert(
                id.clone(),
                json!({
                    "name": id,
                    "tool_call": true,
                    "options": { "num_ctx": OLLAMA_DEFAULT_NUM_CTX },
                }),
            );
        }
        providers.insert(
            "ollama".into(),
            json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "Ollama",
                "options": { "baseURL": format!("{base}/v1"), "apiKey": "ollama" },
                "models": Value::Object(model_map),
            }),
        );
    }

    if providers.is_empty() {
        obj.remove("provider");
    } else {
        obj.insert("provider".into(), Value::Object(providers));
    }

    write_json_atomic(path, &Value::Object(obj), 0o644)
}

// The server runs each RPC on its own task; unserialized read-modify-writes lose edits.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn write_lock() -> MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read_object(path: &Path, label: &str) -> Result<Map<String, Value>> {
    match std::fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => Ok(Map::new()),
        Ok(s) => match serde_json::from_str::<Value>(&s) {
            Ok(Value::Object(m)) => Ok(m),
            Ok(_) => Ok(Map::new()),
            Err(e) => Err(CoreError::OutputParse {
                program: label.to_string(),
                detail: format!("{}: {e}", path.display()),
            }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e.into()),
    }
}

fn write_json_atomic(path: &Path, value: &Value, mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| CoreError::OutputParse {
        program: "opencode config".into(),
        detail: e.to_string(),
    })?;
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{seq}", std::process::id()));
    std::fs::write(&tmp, json.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)) {
            tracing::warn!("could not set mode {mode:o} on {}: {e}", path.display());
        }
    }
    let _ = mode;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn context_window_pins_ollama_only() {
        assert_eq!(
            opencode_model_context_window("ollama/qwen2.5-coder"),
            Some(OLLAMA_DEFAULT_NUM_CTX as u32)
        );
        assert_eq!(opencode_model_context_window("opencode/grok-code"), None);
        assert_eq!(
            opencode_model_context_window("anthropic/claude-opus-4-6"),
            None
        );
    }

    fn temp_paths(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "hl-opencode-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("auth.json"), dir.join("opencode.json"))
    }

    #[test]
    fn a_sign_in_opencode_2_cannot_use_is_flagged_and_left_out_of_the_keys() {
        let (auth_path, _) = temp_paths("oauth");
        std::fs::write(
            &auth_path,
            r#"{"openai":{"type":"api","key":"sk-1"},"anthropic":{"type":"oauth","refresh":"r","access":"a","expires":1}}"#,
        )
        .unwrap();
        let keys = api_keys(&auth_path).unwrap();
        assert_eq!(keys.keys.get("openai").map(String::as_str), Some("sk-1"));
        assert_eq!(keys.unusable, ["anthropic"]);
        let entries = list_auth(&auth_path).unwrap();
        assert_eq!(entries[0].provider, "anthropic");
        assert_eq!(
            entries[0].remedy.as_deref(),
            Some("OpenCode 2 can't use this sign-in. Sign in again here, or add an API key.")
        );
        assert!(entries[1].remedy.is_none());
    }

    #[test]
    fn a_sign_in_made_here_replaces_the_flagged_entry() {
        let (auth_path, config_path) = temp_paths("signin");
        let sign_ins = SignInRecords {
            signed_in: config_path.with_file_name("sign-ins.json"),
            pending: config_path.with_file_name("sign-ins-pending.json"),
        };
        std::fs::write(
            &auth_path,
            r#"{"openai":{"type":"oauth","refresh":"r","access":"a","expires":1},"xai":{"type":"api","key":"k"}}"#,
        )
        .unwrap();
        sign_ins.completed("openai").unwrap();
        sign_ins.begun("poe").unwrap();
        sign_ins.begun("xai").unwrap();
        assert_eq!(sign_ins.pending(), ["poe", "xai"]);
        sign_ins.completed("poe").unwrap();
        sign_ins.abandoned("xai").unwrap();
        assert!(sign_ins.pending().is_empty());
        let entries = list_auth_with_sign_ins(&auth_path, &sign_ins).unwrap();
        let shown: Vec<(&str, &str, bool)> = entries
            .iter()
            .map(|e| (e.provider.as_str(), e.kind.as_str(), e.remedy.is_some()))
            .collect();
        assert_eq!(
            shown,
            [
                ("openai", "signIn", false),
                ("poe", "signIn", false),
                ("xai", "api", false)
            ]
        );
        sign_ins.removed("poe").unwrap();
        assert_eq!(sign_ins.signed_in(), ["openai"]);
    }

    #[test]
    fn auth_and_config_io() {
        let (auth_path, config_path) = temp_paths("io");

        assert!(list_auth(&auth_path).unwrap().is_empty());

        set_provider_key(&auth_path, "anthropic", "sk-ant-1").unwrap();
        set_provider_key(&auth_path, "openai/", "sk-oai-2").unwrap();

        let entries = list_auth(&auth_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .any(|e| e.provider == "anthropic" && e.kind == "api"));
        assert!(entries.iter().any(|e| e.provider == "openai"));

        let raw = std::fs::read_to_string(&auth_path).unwrap();
        assert!(raw.contains("sk-ant-1") && raw.contains("sk-oai-2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&auth_path).unwrap().mode() & 0o777, 0o600);
        }

        remove_provider(&auth_path, "anthropic").unwrap();
        let entries = list_auth(&auth_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider, "openai");

        set_default_model(&config_path, Some("anthropic/claude-sonnet-4-5")).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(v["model"], "anthropic/claude-sonnet-4-5");
        assert_eq!(v["$schema"], CONFIG_SCHEMA_URL);

        let mut obj = v.as_object().unwrap().clone();
        obj.insert("theme".into(), Value::String("dark".into()));
        std::fs::write(
            &config_path,
            serde_json::to_string_pretty(&Value::Object(obj)).unwrap(),
        )
        .unwrap();
        set_default_model(&config_path, Some("openai/gpt-4o")).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(v["model"], "openai/gpt-4o");
        assert_eq!(v["theme"], "dark");

        set_default_model(&config_path, None).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(v.get("model").is_none());
        assert_eq!(v["theme"], "dark");

        set_default_model(&config_path, Some("GitHub-Copilot/gpt-5")).unwrap();
        assert!(clear_default_model_for_provider(&config_path, "github-copilot").unwrap());
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(v.get("model").is_none());
        assert_eq!(v["theme"], "dark");
        assert!(!clear_default_model_for_provider(&config_path, "github-copilot").unwrap());

        let mut obj = v.as_object().unwrap().clone();
        obj.insert(
            "provider".into(),
            serde_json::json!({ "openai": { "npm": "x" } }),
        );
        std::fs::write(
            &config_path,
            serde_json::to_string_pretty(&Value::Object(obj)).unwrap(),
        )
        .unwrap();

        set_ollama_provider(
            &config_path,
            "http://nexus:11434/",
            &["qwen2.5-coder:7b".into(), "llama3.2:latest".into()],
        )
        .unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["provider"]["openai"]["npm"], "x");
        assert_eq!(
            v["provider"]["ollama"]["options"]["baseURL"],
            "http://nexus:11434/v1"
        );
        assert_eq!(v["provider"]["ollama"]["options"]["apiKey"], "ollama");
        assert_eq!(v["provider"]["ollama"]["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(
            v["provider"]["ollama"]["models"]["qwen2.5-coder:7b"]["tool_call"],
            true
        );
        assert!(v["provider"]["ollama"]["models"]["llama3.2:latest"].is_object());

        set_ollama_provider(&config_path, "http://localhost:11434", &[]).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(v["provider"].get("ollama").is_none());
        assert_eq!(v["provider"]["openai"]["npm"], "x");
    }

    #[test]
    fn a_malformed_auth_file_is_refused_not_overwritten() {
        let (auth_path, _) = temp_paths("malformed");
        let malformed = r#"{"anthropic": {"type": "api", "key": "sk-ant-1"},}"#;
        std::fs::write(&auth_path, malformed).unwrap();

        assert!(set_provider_key(&auth_path, "openai", "sk-oai-2").is_err());
        assert!(remove_provider(&auth_path, "anthropic").is_err());
        assert_eq!(std::fs::read_to_string(&auth_path).unwrap(), malformed);
    }

    #[test]
    fn concurrent_key_writes_all_land() {
        let (auth_path, _) = temp_paths("concurrent");
        std::thread::scope(|scope| {
            for thread in 0..16 {
                let auth_path = &auth_path;
                scope.spawn(move || {
                    for n in 0..8 {
                        set_provider_key(auth_path, &format!("p{thread}-{n}"), "key").unwrap();
                    }
                });
            }
        });
        assert_eq!(list_auth(&auth_path).unwrap().len(), 16 * 8);
    }
}
