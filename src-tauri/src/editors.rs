//! Detect the editors, terminals, and file managers a user has installed, so
//! the "Open in…" menu only offers apps that will launch and can show each
//! app's real icon rather than a generic glyph.
//!
//! Detection is platform-specific:
//! - **macOS**: locate `<name>.app` bundles and rasterize each `.icns` to a
//!   small PNG (via `sips`), handed to the frontend as a `data:` URL.
//! - **Windows**: probe well-known install locations for each `.exe` (the full
//!   path becomes `openWith`), falling back to a PATHEXT-aware PATH lookup so
//!   `.cmd` shims like VS Code's `code.cmd` are found.
//! - **Linux**: PATH lookup of each command name.
//!
//! **Terminals are macOS-only.** `openPath(dir, app)` bottoms out in
//! `Command::new(app).arg(dir)`: one program, one positional argument. Editors
//! read that as "open this folder"; terminals want their own flag instead
//! (`wt -d`, `kitty --directory`, `wezterm start --cwd`), and a single
//! `openWith` string has nowhere to put one. Only macOS can express it, via
//! `open -a Terminal <dir>`, so elsewhere terminals are omitted rather than
//! listed and silently failing.
//!
//! The frontend opens through `@tauri-apps/plugin-opener`, passing `openWith`
//! for the GUI apps below (Finder/Explorer use the default handler).

#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;

/// What sort of app an entry is, so the menu can group/label sensibly.
#[derive(Debug, Clone, Copy, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum EditorKind {
    /// The OS file manager (Finder / Explorer / default handler).
    Files,
    Editor,
    Terminal,
}

/// One installed app the user can open a directory in. `openWith` is the value
/// the frontend passes to `openPath(dir, openWith)`; `None` means "open with
/// the OS default handler for a directory" (the file manager). `iconDataUrl` is
/// a `data:image/png` of the app's real icon when we could rasterize one.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EditorTarget {
    pub id: String,
    pub name: String,
    pub kind: EditorKind,
    pub open_with: Option<String>,
    pub icon_data_url: Option<String>,
}

/// A candidate app and how to find / open it on each platform.
struct Candidate {
    id: &'static str,
    name: &'static str,
    kind: EditorKind,
    /// macOS application name, used both to locate `<name>.app` and as the
    /// `open -a` target the frontend passes as `openWith`. Unused off macOS.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    macos_app: &'static str,
    /// Windows: executable paths relative to common install roots (see
    /// [`windows_roots`]). The first existing match's full path becomes the
    /// `openWith` value. Unused off Windows.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    windows_paths: &'static [&'static str],
    /// Command names to look for on PATH (Linux, and as a PATHEXT-aware Windows
    /// fallback). The matched name is the `openWith` value. Unused on macOS,
    /// which detects via `.app` bundles instead, and unused for terminals
    /// anywhere, since only macOS lists those (see the module docs).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    bins: &'static [&'static str],
}

/// The catalog of apps we know how to detect and open. Order here is the order
/// they appear in the menu (file manager first, then editors, then terminals).
const CANDIDATES: &[Candidate] = &[
    Candidate {
        id: "cursor",
        name: "Cursor",
        kind: EditorKind::Editor,
        macos_app: "Cursor",
        windows_paths: &["cursor\\Cursor.exe"],
        bins: &["cursor"],
    },
    Candidate {
        id: "vscode",
        name: "VS Code",
        kind: EditorKind::Editor,
        macos_app: "Visual Studio Code",
        windows_paths: &["Microsoft VS Code\\Code.exe"],
        bins: &["code"],
    },
    Candidate {
        id: "vscode-insiders",
        name: "VS Code Insiders",
        kind: EditorKind::Editor,
        macos_app: "Visual Studio Code - Insiders",
        windows_paths: &["Microsoft VS Code Insiders\\Code - Insiders.exe"],
        bins: &["code-insiders"],
    },
    Candidate {
        id: "windsurf",
        name: "Windsurf",
        kind: EditorKind::Editor,
        macos_app: "Windsurf",
        windows_paths: &["Windsurf\\Windsurf.exe"],
        bins: &["windsurf"],
    },
    Candidate {
        id: "zed",
        name: "Zed",
        kind: EditorKind::Editor,
        macos_app: "Zed",
        windows_paths: &["Zed\\Zed.exe"],
        bins: &["zed"],
    },
    Candidate {
        id: "sublime",
        name: "Sublime Text",
        kind: EditorKind::Editor,
        macos_app: "Sublime Text",
        windows_paths: &[
            "Sublime Text\\sublime_text.exe",
            "Sublime Text 3\\sublime_text.exe",
        ],
        bins: &["subl"],
    },
    Candidate {
        id: "ghostty",
        name: "Ghostty",
        kind: EditorKind::Terminal,
        macos_app: "Ghostty",
        windows_paths: &[],
        bins: &["ghostty"],
    },
    Candidate {
        id: "iterm",
        name: "iTerm",
        kind: EditorKind::Terminal,
        macos_app: "iTerm",
        windows_paths: &[],
        bins: &[],
    },
    Candidate {
        id: "warp",
        name: "Warp",
        kind: EditorKind::Terminal,
        macos_app: "Warp",
        windows_paths: &[],
        bins: &["warp-terminal"],
    },
    Candidate {
        id: "wezterm",
        name: "WezTerm",
        kind: EditorKind::Terminal,
        macos_app: "WezTerm",
        windows_paths: &[],
        bins: &["wezterm"],
    },
    Candidate {
        id: "kitty",
        name: "kitty",
        kind: EditorKind::Terminal,
        macos_app: "kitty",
        windows_paths: &[],
        bins: &["kitty"],
    },
    Candidate {
        id: "alacritty",
        name: "Alacritty",
        kind: EditorKind::Terminal,
        macos_app: "Alacritty",
        windows_paths: &[],
        bins: &["alacritty"],
    },
];

/// Detect every installed app from the catalog, with the platform file manager
/// always first, and (on macOS) each app's rendered icon. Spawns `sips` per
/// app, so callers should run it off the async runtime (spawn_blocking); the
/// frontend caches the result for the session.
pub fn detect() -> Vec<EditorTarget> {
    // (target, bundle path used for icon extraction, macOS only).
    let mut entries: Vec<(EditorTarget, Option<PathBuf>)> = Vec::new();

    // The OS file manager always exists; `openWith: None` opens the directory
    // with the default handler (Finder on macOS, Explorer on Windows).
    entries.push((
        EditorTarget {
            id: "files".into(),
            name: file_manager_name().into(),
            kind: EditorKind::Files,
            open_with: None,
            icon_data_url: None,
        },
        None,
    ));

    for c in CANDIDATES {
        if let Some((open_with, app_path)) = detect_candidate(c) {
            entries.push((
                EditorTarget {
                    id: c.id.into(),
                    name: c.name.into(),
                    kind: c.kind,
                    open_with: Some(open_with),
                    icon_data_url: None,
                },
                app_path,
            ));
        }
    }

    // The system terminal ships with macOS; surface it after the rest so it's a
    // dependable fallback even when no third-party terminal is installed.
    #[cfg(target_os = "macos")]
    if let Some(path) = find_app_bundle("Terminal") {
        entries.push((
            EditorTarget {
                id: "terminal".into(),
                name: "Terminal".into(),
                kind: EditorKind::Terminal,
                open_with: Some("Terminal".into()),
                icon_data_url: None,
            },
            Some(path),
        ));
    }

    // Rasterize each bundle's icon in parallel, one short-lived `sips` per app.
    #[cfg(target_os = "macos")]
    {
        let icons: Vec<Option<String>> = std::thread::scope(|scope| {
            let handles: Vec<_> = entries
                .iter()
                .map(|(_, app_path)| {
                    let app_path = app_path.clone();
                    scope.spawn(move || app_path.as_deref().and_then(icon_data_url))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().ok().flatten())
                .collect()
        });
        for (entry, icon) in entries.iter_mut().zip(icons) {
            entry.0.icon_data_url = icon;
        }
    }

    entries.into_iter().map(|(target, _)| target).collect()
}

fn file_manager_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Finder"
    }
    #[cfg(target_os = "windows")]
    {
        "Explorer"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Files"
    }
}

/// Returns the `openWith` value (and, on macOS, the bundle path for icon
/// extraction) if the candidate is installed.
fn detect_candidate(c: &Candidate) -> Option<(String, Option<PathBuf>)> {
    #[cfg(target_os = "macos")]
    {
        find_app_bundle(c.macos_app).map(|path| (c.macos_app.to_string(), Some(path)))
    }
    #[cfg(target_os = "windows")]
    {
        if matches!(c.kind, EditorKind::Terminal) {
            return None;
        }
        // Prefer a concrete install (full exe path is the most reliable
        // `openWith`); fall back to a CLI shim on PATH.
        for rel in c.windows_paths {
            for root in windows_roots() {
                let exe = root.join(rel);
                if exe.is_file() {
                    return Some((exe.to_string_lossy().into_owned(), None));
                }
            }
        }
        c.bins
            .iter()
            .find(|bin| bin_on_path(bin))
            .map(|bin| (bin.to_string(), None))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if matches!(c.kind, EditorKind::Terminal) {
            return None;
        }
        c.bins
            .iter()
            .find(|bin| bin_on_path(bin))
            .map(|bin| (bin.to_string(), None))
    }
}

/// Common install roots editors land in on Windows (user-scoped first).
#[cfg(target_os = "windows")]
fn windows_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        roots.push(local.join("Programs")); // per-user installs (VS Code, Cursor…)
        roots.push(local);
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(dir) = std::env::var_os(var) {
            roots.push(PathBuf::from(dir));
        }
    }
    roots
}

/// True if `bin` is found on the process PATH. On Windows the lookup honors
/// PATHEXT so `.cmd`/`.bat` shims (e.g. VS Code's `code.cmd`) are matched, not
/// just `.exe`.
#[cfg(not(target_os = "macos"))]
fn bin_on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        if dir.join(bin).is_file() {
            return true;
        }
        #[cfg(target_os = "windows")]
        {
            let pathext =
                std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
            return pathext
                .split(';')
                .filter(|ext| !ext.is_empty())
                .any(|ext| dir.join(format!("{bin}{ext}")).is_file());
        }
        #[allow(unreachable_code)]
        false
    })
}

/// The path to `<name>.app` in any standard macOS location, if present.
#[cfg(target_os = "macos")]
fn find_app_bundle(app_name: &str) -> Option<PathBuf> {
    let bundle = format!("{app_name}.app");
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
        .into_iter()
        .map(|root| root.join(&bundle))
        .find(|candidate| candidate.exists())
}

/// Rasterize a macOS app's icon to a small PNG `data:` URL via the built-in
/// `sips`. Returns `None` for apps that ship only an asset catalog (no loose
/// `.icns`), the UI falls back to a glyph.
#[cfg(target_os = "macos")]
fn icon_data_url(app_path: &Path) -> Option<String> {
    use base64::Engine;

    let icns = resolve_icns(app_path)?;

    // Distinct temp file per bundle so the parallel conversions don't collide.
    let stem = app_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app");
    let out = std::env::temp_dir().join(format!("hyprlayer-icon-{stem}.png"));
    let status = std::process::Command::new("sips")
        .args(["-s", "format", "png", "-Z", "64"])
        .arg(&icns)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }

    let bytes = std::fs::read(&out).ok()?;
    let _ = std::fs::remove_file(&out);
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Locate a bundle's primary app-icon `.icns`. Prefers the `CFBundleIconFile`
/// declared in Info.plist (read via the built-in `defaults`, which handles
/// binary plists), since picking by file size mis-fires on apps that ship a
/// larger document icon (e.g. Zed's `Document.icns`). Falls back to a
/// name-matched then largest `.icns`.
#[cfg(target_os = "macos")]
fn resolve_icns(app_path: &Path) -> Option<PathBuf> {
    let resources = app_path.join("Contents/Resources");

    if let Ok(output) = std::process::Command::new("defaults")
        .arg("read")
        .arg(app_path.join("Contents/Info"))
        .arg("CFBundleIconFile")
        .output()
    {
        if output.status.success() {
            let mut name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                if !name.to_ascii_lowercase().ends_with(".icns") {
                    name.push_str(".icns");
                }
                let declared = resources.join(&name);
                if declared.is_file() {
                    return Some(declared);
                }
            }
        }
    }

    // Fallbacks: an `.icns` named after the bundle, else the largest one.
    let app_stem = app_path.file_stem().and_then(|s| s.to_str());
    let mut largest: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(&resources).ok()?.flatten() {
        let path = entry.path();
        let is_icns = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("icns"));
        if !is_icns {
            continue;
        }
        if let (Some(stem), Some(app_stem)) = (path.file_stem().and_then(|s| s.to_str()), app_stem)
        {
            if stem.eq_ignore_ascii_case(app_stem) {
                return Some(path);
            }
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if largest.as_ref().is_none_or(|(b, _)| size > *b) {
            largest = Some((size, path));
        }
    }
    largest.map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_always_includes_file_manager_first() {
        let targets = detect();
        assert!(!targets.is_empty());
        assert_eq!(targets[0].id, "files");
        assert!(matches!(targets[0].kind, EditorKind::Files));
        assert!(targets[0].open_with.is_none());
    }

    #[test]
    fn catalog_ids_are_unique() {
        let mut ids: Vec<&str> = CANDIDATES.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate candidate id in catalog");
    }
}
