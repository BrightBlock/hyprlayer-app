//! Undo the AppImage runtime's environment for children we spawn.
//!
//! The AppImage `AppRun` rewrites the process environment to point at the
//! mounted AppDir before handing control to the app: `PATH`,
//! `LD_LIBRARY_PATH`, `XDG_DATA_DIRS`, the GTK/GIO module caches, and — the
//! one that bites hardest — `PYTHONHOME`/`PYTHONPATH`. We ship no Python, so
//! `PYTHONHOME=/tmp/.mount_XXXX/usr/` names a tree with no stdlib in it, and
//! every `python` launched from a hyprlayer terminal dies at interpreter
//! bootstrap with `ModuleNotFoundError: No module named 'encodings'`. The same
//! leak makes children resolve our bundled GTK/webkit libs instead of the
//! system's, and pins `GDK_BACKEND=x11` on a Wayland session.
//!
//! Those variables are correct for *us* — the webview's out-of-process helpers
//! need them — so we can't scrub them globally at startup. We scrub per-spawn
//! instead, on the way out to a child that is emphatically not part of the
//! bundle. This is best-effort restoration: the `AppRun` we ship saves no
//! `APPIMAGE_ORIGINAL_*` copies, so all we can do is drop AppDir-owned entries
//! and leave whatever the user's session had underneath.

use std::env;
use std::path::Path;

/// Variables whose value is a `:`-separated search path. AppDir entries are
/// filtered out; if nothing survives, the variable is unset entirely (an empty
/// `PATH` or `LD_LIBRARY_PATH` means "current directory" to some loaders,
/// which is both wrong and a security footgun).
const PATH_LIST_VARS: &[&str] = &[
    "PATH",
    "LD_LIBRARY_PATH",
    "PYTHONPATH",
    "PERLLIB",
    "PERL5LIB",
    "XDG_DATA_DIRS",
    "XDG_CONFIG_DIRS",
    "QT_PLUGIN_PATH",
    "GTK_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GSETTINGS_SCHEMA_DIR",
    "GIO_EXTRA_MODULES",
];

/// Variables holding a single AppDir path (a prefix or a module cache file).
/// Unset when the value points inside the AppDir, left alone otherwise.
const APPDIR_VALUE_VARS: &[&str] = &[
    "PYTHONHOME",
    "GTK_DATA_PREFIX",
    "GTK_EXE_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GDK_PIXBUF_MODULE_FILE",
];

/// Variables the AppImage runtime owns outright: it either invents them or
/// overwrites whatever was there, so for a child they are pure noise.
/// `GDK_BACKEND` is the notable one — `AppRun` forces it to `x11` to dodge a
/// Tauri/Wayland crash, and a GUI app launched from a hyprlayer terminal
/// should not inherit that.
const ALWAYS_UNSET: &[&str] = &[
    "APPDIR",
    "APPIMAGE",
    "ARGV0",
    "OWD",
    "APPIMAGE_GTK_THEME",
    "PYTHONDONTWRITEBYTECODE",
    "GDK_BACKEND",
    "GTK_THEME",
];

/// Environment edits to apply to a child process: `(key, Some(value))` to
/// override, `(key, None)` to unset. Empty when we are not running from an
/// AppImage, which is every non-Linux platform and every Linux dev build.
///
/// Apply these *before* any caller-supplied env, so an explicit override still
/// wins.
pub fn child_env_overrides() -> Vec<(String, Option<String>)> {
    let Some(appdir) = env::var_os("APPDIR") else {
        return Vec::new();
    };
    let appdir = appdir.to_string_lossy().into_owned();
    if appdir.is_empty() {
        return Vec::new();
    }
    overrides_for(&appdir, |key| env::var(key).ok())
}

/// The pure core of [`child_env_overrides`], with the AppDir and the
/// environment lookup injected so it can be tested without touching the
/// process-wide environment.
fn overrides_for(
    appdir: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();

    for key in PATH_LIST_VARS {
        let Some(value) = lookup(key) else { continue };
        let kept: Vec<&str> = value
            .split(':')
            .filter(|entry| !entry.is_empty() && !is_under(entry, appdir))
            .collect();
        if kept.len() == value.split(':').filter(|e| !e.is_empty()).count() {
            continue; // nothing of ours in there — leave it untouched
        }
        out.push((
            (*key).to_string(),
            (!kept.is_empty()).then(|| kept.join(":")),
        ));
    }

    for key in APPDIR_VALUE_VARS {
        if lookup(key).is_some_and(|value| is_under(&value, appdir)) {
            out.push(((*key).to_string(), None));
        }
    }

    for key in ALWAYS_UNSET {
        if lookup(key).is_some() {
            out.push(((*key).to_string(), None));
        }
    }

    out
}

/// Is `path` inside the mounted AppDir? Always false when we are not running
/// from an AppImage.
///
/// Used to keep AppDir entries out of the PATH the binary resolver builds and
/// hands to children — that PATH is applied *after* [`child_env_overrides`],
/// so without this filter it would put them straight back.
pub fn is_appdir_path(path: &Path) -> bool {
    env::var_os("APPDIR")
        .is_some_and(|appdir| !appdir.is_empty() && path.starts_with(Path::new(&appdir)))
}

/// Is `path` the AppDir itself, or something inside it?
///
/// Compared as a path rather than a string prefix so a sibling mount named
/// `/tmp/.mount_AbcXYZ` isn't swallowed by `/tmp/.mount_Abc`. `AppRun` emits
/// doubled separators (`$APPDIR//usr/lib`), which `Path`'s component-wise
/// comparison normalises for free.
fn is_under(path: &str, appdir: &str) -> bool {
    Path::new(path).starts_with(Path::new(appdir))
}

/// Apply [`child_env_overrides`] to a `std::process::Command`.
pub fn scrub_std(cmd: &mut std::process::Command) {
    for (key, value) in child_env_overrides() {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        };
    }
}

/// Apply [`child_env_overrides`] to a `tokio::process::Command`.
pub fn scrub_tokio(cmd: &mut tokio::process::Command) {
    for (key, value) in child_env_overrides() {
        match value {
            Some(value) => cmd.env(key, value),
            None => cmd.env_remove(key),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const APPDIR: &str = "/tmp/.mount_HyprlaAbc12";

    fn overrides(env: &[(&str, &str)]) -> HashMap<String, Option<String>> {
        let env: HashMap<&str, &str> = env.iter().copied().collect();
        overrides_for(APPDIR, |key| env.get(key).map(|v| (*v).to_string()))
            .into_iter()
            .collect()
    }

    #[test]
    fn unsets_pythonhome_pointing_into_the_appdir() {
        // The reported failure: python can't find its stdlib and dies with
        // "No module named 'encodings'".
        let out = overrides(&[("PYTHONHOME", "/tmp/.mount_HyprlaAbc12/usr/")]);
        assert_eq!(out.get("PYTHONHOME"), Some(&None));
    }

    #[test]
    fn leaves_a_user_owned_pythonhome_alone() {
        let out = overrides(&[("PYTHONHOME", "/home/jt/.venv")]);
        assert!(!out.contains_key("PYTHONHOME"));
    }

    #[test]
    fn strips_appdir_entries_from_path_but_keeps_the_rest() {
        let out = overrides(&[(
            "PATH",
            "/tmp/.mount_HyprlaAbc12/usr/bin/:/tmp/.mount_HyprlaAbc12/bin/:/usr/bin:/home/jt/.local/bin",
        )]);
        assert_eq!(
            out.get("PATH"),
            Some(&Some("/usr/bin:/home/jt/.local/bin".to_string()))
        );
    }

    #[test]
    fn unsets_a_search_path_that_was_entirely_ours() {
        // AppRun writes LD_LIBRARY_PATH from scratch and leaves a trailing `:`.
        // Keeping an empty value would tell the loader to search the cwd.
        let out = overrides(&[(
            "LD_LIBRARY_PATH",
            "/tmp/.mount_HyprlaAbc12/usr/lib/:/tmp/.mount_HyprlaAbc12/lib64/:",
        )]);
        assert_eq!(out.get("LD_LIBRARY_PATH"), Some(&None));
    }

    #[test]
    fn leaves_an_untouched_search_path_alone() {
        let out = overrides(&[("PATH", "/usr/bin:/home/jt/.local/bin")]);
        assert!(!out.contains_key("PATH"));
    }

    #[test]
    fn tolerates_the_doubled_separators_apprun_emits() {
        let out = overrides(&[(
            "GTK_PATH",
            "/tmp/.mount_HyprlaAbc12//usr/lib/gtk-3.0:/usr/lib64/gtk-3.0",
        )]);
        assert_eq!(
            out.get("GTK_PATH"),
            Some(&Some("/usr/lib64/gtk-3.0".to_string()))
        );
    }

    #[test]
    fn does_not_swallow_a_sibling_mount_with_a_shared_prefix() {
        let out = overrides(&[("PATH", "/tmp/.mount_HyprlaAbc12XY/bin:/usr/bin")]);
        assert!(!out.contains_key("PATH"), "sibling mount must survive");
    }

    #[test]
    fn drops_the_forced_x11_backend_and_appimage_markers() {
        let out = overrides(&[
            ("GDK_BACKEND", "x11"),
            ("APPDIR", APPDIR),
            ("APPIMAGE", "/home/jt/.local/bin/Hyprlayer"),
        ]);
        assert_eq!(out.get("GDK_BACKEND"), Some(&None));
        assert_eq!(out.get("APPDIR"), Some(&None));
        assert_eq!(out.get("APPIMAGE"), Some(&None));
    }

    #[test]
    fn is_inert_outside_an_appimage() {
        // No APPDIR in the environment -> nothing to undo. Guards the dev build
        // and every non-Linux platform.
        let out = overrides_for("", |_| None);
        assert!(out.is_empty());
        assert!(overrides(&[]).is_empty());
    }
}
