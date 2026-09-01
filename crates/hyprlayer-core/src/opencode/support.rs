use std::path::Path;

use crate::error::Result;

pub(crate) const UNSUPPORTED_OPENCODE_PROVIDER_IDS: &[&str] = &["github-copilot"];

pub fn opencode_provider_is_supported(provider: &str) -> bool {
    let provider = provider.trim().trim_end_matches('/');
    !UNSUPPORTED_OPENCODE_PROVIDER_IDS
        .iter()
        .any(|unsupported| provider.eq_ignore_ascii_case(unsupported))
}

pub fn opencode_model_is_supported(model: &str) -> bool {
    let model = model
        .strip_prefix("opencode:")
        .or_else(|| {
            model
                .get(.."opencode:".len())
                .filter(|prefix| prefix.eq_ignore_ascii_case("opencode:"))
                .map(|_| &model["opencode:".len()..])
        })
        .unwrap_or(model);
    let provider = model.split('/').next().unwrap_or(model);
    opencode_provider_is_supported(provider)
}

/// The `opencode_remove_github_copilot_default_v1` runtime migration: clears a
/// stale default-model pointer at a retired provider, run against the profile's
/// own `RuntimePaths.opencode_config_file` by `runtime::migrations`. Idempotent —
/// a config with no matching default is a no-op — so a rerun after a crash
/// between the config write and the ledger commit is always safe.
pub fn opencode_remove_github_copilot_default_v1(config_path: &Path) -> Result<bool> {
    let mut changed = false;
    for provider in UNSUPPORTED_OPENCODE_PROVIDER_IDS {
        if crate::opencode::config::clear_default_model_for_provider(config_path, provider)? {
            changed = true;
        }
    }
    Ok(changed)
}
