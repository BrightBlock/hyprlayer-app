//! Omarchy desktop theme integration (read-only).
//!
//! Omarchy keeps its live theme state in `~/.local/state/omarchy/current/`:
//! `theme.name` (plain text, kebab-case) and `theme/` (the active theme
//! directory, whose `colors.toml` is the palette). We read both; we never
//! write anything under `~/.config/omarchy` or `~/.local/state/omarchy`.
//!
//! Verified against Omarchy 4.0.0 (`ID=omarchy`, `VERSION_ID="4.0.0"` in
//! /etc/os-release) on 2026-08-24.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Resolve Omarchy's live-state directory — `~/.local/state/omarchy/current`,
/// falling back to the pre-v4 `~/.config/omarchy/current` — returning the
/// first of the two whose `theme.name` file is present.
///
/// This is a path probe rather than `OMARCHY_PATH`, `DESKTOP_SESSION=omarchy`,
/// or `ID=omarchy` in `/etc/os-release`. All three are present on this
/// machine, but a GUI app's environment depends on how it was launched
/// (desktop entry, terminal, `pnpm tauri dev`), and the path is what the
/// feature actually needs: a readable `colors.toml`.
///
/// `~/.config/omarchy/` **exists on v4 too** — it holds `hooks/`, `plugins/`,
/// `themed/`, `backgrounds/` — so its mere presence is not a v3 signal. The
/// fallback therefore probes `config_dir()/omarchy/current/theme.name`
/// specifically, never the bare `config_dir()/omarchy` directory. That
/// fallback is **UNVERIFIED** — only v4 was available to test. The migration
/// that moved `current/` out of `~/.config/omarchy` and into `state_dir()` is
/// `/usr/share/omarchy/migrations/1781043107.sh`.
///
/// Deliberately not the sibling branch's `dirs::state_dir().or_else(dirs::
/// data_local_dir)` spelling (`crates/hyprlayer-core/src/desktop_stats.rs:
/// 250-252` on `hyprlayer/omarchy-quattro-plugin-5b8d`). That fallback exists
/// so a *writer* doesn't silently no-op on macOS/Windows; this is a
/// Linux-only *reader*, and probing `~/Library/Application Support/omarchy/
/// current` is dead code.
///
/// The `?` on `dirs::state_dir()` is load-bearing and must stay spelled
/// `dirs::state_dir()?`, not chained into the v3 fallback with `.or_else(...)`.
/// `state_dir()` returns `None` on macOS and Windows (dirs-5.0.1 `mac.rs:17`,
/// `win.rs:15`), so short-circuiting there is exactly what makes this
/// function — and therefore the Phase 3 command and poll built on it — inert
/// off Linux without a `#[cfg]`. The `config_dir()` probe below is reached
/// only on Linux, where it is the genuine v3 path and not a dead
/// cross-platform branch: `config_dir()` resolves on every platform, so
/// gating it behind the `state_dir()` short-circuit is what keeps it
/// Linux-only in practice. Both facts matter — without the `?`, the v3 probe
/// would also be *reachable* (if uselessly) on macOS, which is precisely the
/// shape the sibling branch's writer wants and this reader does not.
pub fn current_dir() -> Option<PathBuf> {
    let candidate = dirs::state_dir()?.join("omarchy/current");
    if candidate.join("theme.name").is_file() {
        return Some(candidate);
    }
    let candidate = dirs::config_dir()?.join("omarchy/current");
    if candidate.join("theme.name").is_file() {
        return Some(candidate);
    }
    None
}

/// Read `current/theme.name`, trimmed. `""` on any failure — missing,
/// unreadable, or (the case that matters) truncated mid-write.
///
/// `/usr/bin/omarchy-theme-set:168` writes this file with a non-atomic
/// `echo "$THEME_NAME" >theme.name` — a truncate-in-place, so a reader can
/// observe an empty or partial name while a theme switch is in flight. That
/// is safe to degrade silently: the palette itself comes from
/// `current/theme/colors.toml`, never from `<themes>/<name>/`, so a
/// truncated name cannot mis-resolve a path — it only degrades the display
/// label for one poll tick.
pub fn theme_name(current: &Path) -> String {
    std::fs::read_to_string(current.join("theme.name"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The active theme's `colors.toml`, tolerantly parsed: `#[serde(default)]`
/// fills any missing key with an empty string rather than failing, and any
/// key this struct does not name (`hyprland_active_border`,
/// `active_tab_background`, …) is silently ignored by serde's default
/// behaviour rather than rejected. [`OmarchyPalette::validate`] is the actual
/// usability gate — parsing alone accepts a struct of empty strings.
///
/// Coverage measured across all 22 built-in themes on Omarchy 4.0.0: every
/// field below except `orange` and `brown` is present in all 22. `orange`
/// and `brown` are present in 19 — missing from exactly `last-horizon`,
/// `solitude`, `white` — which is why [`OmarchyPalette::orange`] and
/// [`OmarchyPalette::brown`] exist as fallback accessors rather than plain
/// field reads.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OmarchyPalette {
    pub mode: String,
    pub accent: String,
    pub selection: String,
    pub muted: String,
    pub background: String,
    pub dark_background: String,
    pub darker_background: String,
    pub lighter_background: String,
    pub foreground: String,
    pub dark_foreground: String,
    pub light_foreground: String,
    pub bright_foreground: String,
    pub red: String,
    pub yellow: String,
    pub orange: String,
    pub green: String,
    pub cyan: String,
    pub blue: String,
    pub magenta: String,
    pub brown: String,
    pub bright_red: String,
    pub bright_yellow: String,
    pub bright_green: String,
    pub bright_cyan: String,
    pub bright_blue: String,
    pub bright_magenta: String,
}

impl OmarchyPalette {
    /// Every 22/22 key (every field above except `orange` and `brown`, whose
    /// absence *and* malformation are both handled by the fallback accessors
    /// rather than validation, and `mode`, which is not a colour) must be a
    /// `#rgb` or `#rrggbb` hex string, case-insensitive — seven built-ins use
    /// uppercase hex
    /// (`ethereal`, `flexoki-light`, `hackerman`, `kanagawa`, `last-horizon`,
    /// `matte-black`, `osaka-jade`).
    ///
    /// Returns the same [`CoreError::OmarchyPaletteParse`] variant as a
    /// syntax failure, since from the caller's side both mean "this palette
    /// is unusable". This is what closes the empty-file hole: an empty TOML
    /// document parses successfully into a struct of empty strings, so the
    /// parse step alone is not a validity check.
    fn validate(&self, path: &Path) -> Result<()> {
        let fields: [(&str, &str); 23] = [
            ("accent", self.accent.as_str()),
            ("selection", self.selection.as_str()),
            ("muted", self.muted.as_str()),
            ("background", self.background.as_str()),
            ("dark_background", self.dark_background.as_str()),
            ("darker_background", self.darker_background.as_str()),
            ("lighter_background", self.lighter_background.as_str()),
            ("foreground", self.foreground.as_str()),
            ("dark_foreground", self.dark_foreground.as_str()),
            ("light_foreground", self.light_foreground.as_str()),
            ("bright_foreground", self.bright_foreground.as_str()),
            ("red", self.red.as_str()),
            ("yellow", self.yellow.as_str()),
            ("green", self.green.as_str()),
            ("cyan", self.cyan.as_str()),
            ("blue", self.blue.as_str()),
            ("magenta", self.magenta.as_str()),
            ("bright_red", self.bright_red.as_str()),
            ("bright_yellow", self.bright_yellow.as_str()),
            ("bright_green", self.bright_green.as_str()),
            ("bright_cyan", self.bright_cyan.as_str()),
            ("bright_blue", self.bright_blue.as_str()),
            ("bright_magenta", self.bright_magenta.as_str()),
        ];
        for (name, value) in fields {
            if hex(value).is_none() {
                return Err(CoreError::OmarchyPaletteParse {
                    path: path.to_path_buf(),
                    message: format!("`{name}` is not a `#rgb`/`#rrggbb` hex colour: {value:?}"),
                });
            }
        }
        Ok(())
    }

    /// `orange`, else `bright_red`, else `red`. Both chains terminate at
    /// `red`, which is 22/22 and validated, so neither can fail — this always
    /// returns a parseable value for a palette that passed
    /// [`OmarchyPalette::validate`].
    ///
    /// The arms test `hex(..).is_some()`, not `!is_empty()`. `orange` and
    /// `brown` are the two fields `validate` skips, so a user theme carrying
    /// `orange = "not-a-color"` reaches here; an emptiness test would return
    /// it verbatim and [`resolve_hue`]'s `unwrap_or(ink)` would silently paint
    /// `--color-handoff` as body ink instead of walking this chain. Absent and
    /// malformed collapse to the same "keep walking" answer.
    pub fn orange(&self) -> &str {
        if hex(&self.orange).is_some() {
            &self.orange
        } else if hex(&self.bright_red).is_some() {
            &self.bright_red
        } else {
            &self.red
        }
    }

    /// `brown`, else [`OmarchyPalette::orange`]. Also terminates at `red`.
    ///
    /// An earlier draft wrote this as "`brown`, else `orange()`, else
    /// `yellow`", which is wrong in a way worth recording so it doesn't get
    /// re-introduced: `orange()` always returns a value, so a `yellow` arm
    /// could never run.
    ///
    /// Bypassing `orange()` to reach `yellow` directly was tried and rejected
    /// on measurement. On the three themes missing both keys, `yellow` is
    /// itself desaturated — `last-horizon` `#6B5E73` (muted purple),
    /// `solitude` `#d9dbdc` (near-white), `white` `#4a4a4a` (grey) — so
    /// routing there produces a *worse* `attention` than delegating on two of
    /// the three. Delegating yields `#c38b7b` on `last-horizon` and
    /// `#de6145` on `solitude`, both genuinely warm and much closer to the
    /// muted amber `--color-attention` is meant to be.
    pub fn brown(&self) -> &str {
        if hex(&self.brown).is_some() {
            &self.brown
        } else {
            self.orange()
        }
    }
}

/// Omarchy's `mode` key: `"dark"` (17 of 22 built-in themes) or `"light"`
/// (5 of 22). Same shape as [`crate::system::FullDiskAccess`].
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum OmarchyMode {
    Dark,
    Light,
}

impl OmarchyMode {
    /// Tolerant parse per ADR-0004: `"light"` (case-insensitive, trimmed)
    /// parses to [`OmarchyMode::Light`]; anything else — `"dark"`, an empty
    /// string, an absent key, or garbage — parses to [`OmarchyMode::Dark`],
    /// matching Omarchy's own majority.
    pub fn parse(s: &str) -> OmarchyMode {
        if s.trim().eq_ignore_ascii_case("light") {
            OmarchyMode::Light
        } else {
            OmarchyMode::Dark
        }
    }
}

/// Read and parse `current/theme/colors.toml`. Mirrors
/// [`crate::repo_settings::read`]: `toml::from_str`, a parse failure maps to
/// [`CoreError::OmarchyPaletteParse`], a missing file maps to
/// [`CoreError::Io`]. Unlike `repo_settings::read`, a missing file here is a
/// real error rather than a default: the caller already knows `current`
/// exists (see [`current_dir`]), so an absent `colors.toml` inside it is
/// unexpected, not the ordinary "never configured" case.
pub fn read_palette(current: &Path) -> Result<OmarchyPalette> {
    let path = current.join("theme").join("colors.toml");
    let raw = std::fs::read_to_string(&path)?;
    let palette: OmarchyPalette =
        toml::from_str(&raw).map_err(|e| CoreError::OmarchyPaletteParse {
            path: path.clone(),
            message: e.to_string(),
        })?;
    palette.validate(&path)?;
    Ok(palette)
}

/// Parse a `#rgb` or `#rrggbb` hex colour, case-insensitive. `#rgb` expands
/// each digit (`#abc` becomes `[0xaa, 0xbb, 0xcc]`), the same doubling every
/// other hex-shorthand parser (CSS included) uses.
pub fn hex(s: &str) -> Option<[u8; 3]> {
    let digits = s.strip_prefix('#')?;
    let expanded: String = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_string(),
        _ => return None,
    };
    let mut channels = expanded.as_bytes().chunks_exact(2).map(|pair| {
        std::str::from_utf8(pair)
            .ok()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
    });
    Some([channels.next()??, channels.next()??, channels.next()??])
}

/// Render as lowercase `#rrggbb` — the canonical form every derived token
/// uses regardless of how the source theme spelled it (seven built-ins use
/// uppercase hex).
pub fn to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

// ---------------------------------------------------------------------
// Palette mapping: colour maths, surface tiers, and the public resolve()
// entry point that turns a validated OmarchyPalette into hyprlayer's 46
// `--color-*` tokens (30 semantic + 16 ANSI terminal slots). `--color-bad`
// is deliberately never derived — see the comment inside `resolve` below.
// ---------------------------------------------------------------------

// --- Colour maths --------------------------------------------------------

/// WCAG 2.1 relative luminance of an sRGB colour, in `[0.0, 1.0]`. Each
/// channel is normalised to `[0.0, 1.0]` and linearised — `c/12.92` below
/// the sRGB toe, `((c+0.055)/1.055).powf(2.4)` above it — then weighted
/// `0.2126 R + 0.7152 G + 0.0722 B`.
fn luminance(rgb: [u8; 3]) -> f64 {
    let channel = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

/// WCAG 2.1 contrast ratio between two colours — `(hi + 0.05) / (lo + 0.05)`
/// over the pair's relative luminances, order-independent (`contrast(a, b)
/// == contrast(b, a)`). Black against white gives the WCAG maximum, 21:1.
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Linear interpolation between two colours in sRGB byte space, each
/// channel rounded to the nearest integer. Deliberately not perceptual — not
/// OKLab, not linear-light: the app's 31 hand-picked hex tokens were chosen
/// by eye in sRGB, and a perceptual blend would produce values that don't
/// sit with them.
fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let lerp = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
}

/// Returns `fg` unchanged if it already clears `ratio` against `bg`.
/// Otherwise walks toward `ink` in up to twenty 5% steps (5%, 10%, … 100% of
/// the distance from the original `fg`), returning the first step that
/// clears; returns `ink` itself if none do.
///
/// Because [`resolve_ink`] floors `ink` to at least 4.5:1 against `surface`
/// (see its doc comment), every clamp this function performs terminates at
/// a value that clears any `ratio` up to 4.5. That is what makes the Phase 2
/// contrast gates hold by construction rather than by luck — the corpus
/// test in `tests/omarchy_corpus.rs` is a regression guard on this
/// function, not a discovery of whether the ratios happen to work.
fn ensure_contrast(fg: [u8; 3], bg: [u8; 3], ratio: f64, ink: [u8; 3]) -> [u8; 3] {
    if contrast(fg, bg) >= ratio {
        return fg;
    }
    for step in 1..=20 {
        let candidate = mix(fg, ink, step as f64 * 0.05);
        if contrast(candidate, bg) >= ratio {
            return candidate;
        }
    }
    ink
}

// --- Surface tiers ---------------------------------------------------------

/// Ranks the four `*background` fields by luminance — ascending (darkest
/// first) for [`OmarchyMode::Dark`], descending (lightest first) for
/// [`OmarchyMode::Light`] — with the field's position in the fixed order
/// `background`, `darker_background`, `dark_background`, `lighter_background`
/// as a stable tie-break. [`resolve`] assigns `surface = rank[0]`,
/// `panel = rank[1]`, `panel-2 = rank[2]`; `rank[3]` is returned but has no
/// token of its own.
///
/// The four `*background` values are NOT luminance-ordered by name, so
/// ranking them is the whole point of this function. Measured across all 22
/// built-in themes on Omarchy 4.0.0: on every one of the five light themes
/// `lighter_background` is DARKER than `background` (`white`: 0.527 vs
/// 1.000; `catppuccin-latte`: 0.744 vs 0.879), and on the dark theme
/// `vantablack` `background` #000000 (0.000) is darker than
/// `darker_background` (0.002). `last-horizon` and `solitude` have
/// `background == lighter_background` byte-for-byte, which is why the sort
/// is stable on the field index.
fn rank_backgrounds(p: &OmarchyPalette, mode: OmarchyMode) -> [[u8; 3]; 4] {
    let fields = [
        hex(&p.background).unwrap_or_default(),
        hex(&p.darker_background).unwrap_or_default(),
        hex(&p.dark_background).unwrap_or_default(),
        hex(&p.lighter_background).unwrap_or_default(),
    ];
    let mut order = [0usize, 1, 2, 3];
    order.sort_by(|&i, &j| {
        let (li, lj) = (luminance(fields[i]), luminance(fields[j]));
        let by_luminance = if mode == OmarchyMode::Dark {
            li.partial_cmp(&lj)
        } else {
            lj.partial_cmp(&li)
        }
        .unwrap_or(std::cmp::Ordering::Equal);
        by_luminance.then(i.cmp(&j))
    });
    [
        fields[order[0]],
        fields[order[1]],
        fields[order[2]],
        fields[order[3]],
    ]
}

// --- Text and accent -------------------------------------------------------

/// The best-contrast foreground against `surface` among the four
/// `*foreground` fields (`foreground`, `dark_foreground`, `light_foreground`,
/// `bright_foreground`), floored: if that best candidate still fails 4.5:1,
/// it is replaced by whichever of pure black and pure white scores higher
/// against `surface`.
///
/// The floor is not dead code for the built-in corpus — measured, every one
/// of the 22 clears 4.5 on its own (minimum 6.66, `rose-pine`) and the floor
/// never fires. It exists for **user-installed themes**
/// (`~/.config/omarchy/themes/`), which the vendored corpus cannot cover by
/// construction. Without it, a user theme whose foreground family sits
/// close to its background collapses `ink-dim`, `ok`, `warn`, `brand`,
/// `brand-soft` and all 11 stage hues onto `ink` — every one of them
/// terminates there via [`ensure_contrast`] — producing a uniformly
/// unreadable UI with no warning and no fallback.
///
/// The floor is also what makes [`ensure_contrast`]'s termination argument a
/// construction rather than a property of the corpus: because `ink` is now
/// guaranteed ≥4.5:1 against `surface`, every clamp that walks toward `ink`
/// ends at a value clearing any ratio ≤4.5. `max(black, white)` is provably
/// safe for the same reason given for [`resolve_accent_ink`] below — the two
/// cross at luminance ≈0.179, where both give ≈4.58:1, so the better of them
/// is never below that for any surface.
fn resolve_ink(p: &OmarchyPalette, surface: [u8; 3]) -> [u8; 3] {
    let candidates = [
        hex(&p.foreground).unwrap_or_default(),
        hex(&p.dark_foreground).unwrap_or_default(),
        hex(&p.light_foreground).unwrap_or_default(),
        hex(&p.bright_foreground).unwrap_or_default(),
    ];
    let best = candidates
        .into_iter()
        .max_by(|a, b| contrast(*a, surface).total_cmp(&contrast(*b, surface)))
        .expect("candidates is a non-empty fixed-size array");
    if contrast(best, surface) >= 4.5 {
        return best;
    }
    let (black, white) = ([0, 0, 0], [255, 255, 255]);
    if contrast(black, surface) >= contrast(white, surface) {
        black
    } else {
        white
    }
}

/// Whichever of `surface` and `ink` has the higher contrast against
/// `accent`, if that clears 4.5:1; otherwise whichever of pure black and
/// pure white is better. The second branch is provably safe: the two pure
/// endpoints cross at luminance ≈0.179, where both give ≈4.58:1, so
/// `max(black, white)` is never below 4.58:1 for any colour. Preferring
/// `surface`/`ink` first keeps the pair in-family; measured, the first
/// branch wins on 20 of the 22 built-in themes and the fallback fires only
/// on `catppuccin-latte` and `rose-pine`.
fn resolve_accent_ink(surface: [u8; 3], ink: [u8; 3], accent: [u8; 3]) -> [u8; 3] {
    let (cs, ci) = (contrast(surface, accent), contrast(ink, accent));
    let (best, best_contrast) = if cs >= ci { (surface, cs) } else { (ink, ci) };
    if best_contrast >= 4.5 {
        return best;
    }
    let (black, white) = ([0, 0, 0], [255, 255, 255]);
    if contrast(black, accent) >= contrast(white, accent) {
        black
    } else {
        white
    }
}

// --- Semantic and stage hues -----------------------------------------------

/// Parse `source` as hex and clamp it to `ratio` against `surface` via
/// [`ensure_contrast`]. `source` is always one of [`OmarchyPalette`]'s own
/// validated fields, or an [`OmarchyPalette::orange`]/[`OmarchyPalette::brown`]
/// fallback — and those two accessors test `hex(..).is_some()` on each arm
/// precisely so this holds for the two fields [`OmarchyPalette::validate`]
/// skips. The `hex` parse therefore cannot fail for a palette that passed
/// `validate`; `ink` is the fallback for the case it somehow does.
fn resolve_hue(source: &str, surface: [u8; 3], ink: [u8; 3], ratio: f64) -> [u8; 3] {
    ensure_contrast(hex(source).unwrap_or(ink), surface, ratio, ink)
}

// --- Public entry point -----------------------------------------------------

/// One resolved CSS custom property. `name` is the full property name
/// (`"--color-surface"`), so the frontend can `setProperty(name, value)`
/// with no mapping table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OmarchyToken {
    pub name: String,
    pub value: String,
}

/// The active Omarchy theme, resolved into hyprlayer's token vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OmarchyTheme {
    /// From `theme.name`; may be empty (see [`theme_name`]).
    pub name: String,
    pub mode: OmarchyMode,
    /// Exactly 46 entries: the 30 semantic `--color-*` tokens (`--color-bad`
    /// deliberately absent, see the comment inside [`resolve`]) followed by
    /// the 16 `--color-ansi-*` terminal slots, strictly appended in that
    /// order.
    pub tokens: Vec<OmarchyToken>,
}

/// Resolve a validated palette into hyprlayer's 46 `--color-*` tokens: the
/// 30 semantic tokens, in the same order the `@theme` block declares them
/// in `src/index.css:76-166` (minus `--color-bad`), followed by the 16
/// `--color-ansi-*` terminal slots in ANSI index order (0-15).
pub fn resolve(name: String, p: &OmarchyPalette) -> OmarchyTheme {
    let mode = OmarchyMode::parse(&p.mode);
    let rank = rank_backgrounds(p, mode);
    let (surface, panel, panel_2) = (rank[0], rank[1], rank[2]);

    let ink = resolve_ink(p, surface);

    // edge = mix(panel_2, ink, 0.20), unconditionally — not a threshold
    // gated on rank[3] (the fourth ranked background, otherwise unused). An
    // earlier draft used rank[3] when contrast(rank[3], panel_2) >= 1.30 and
    // synthesized otherwise, on the assumption the ranked value would
    // normally win. Measured across all 22 built-ins, that raw contrast runs
    // 1.000 to 1.485 and only three themes clear 1.30 — catppuccin 1.305,
    // osaka-jade 1.374, white 1.485 — so a branch firing on 3 of 22 themes
    // is a second, unrelated behaviour hiding behind a constant, not a
    // fallback. Synthesizing always is also the better-behaved option:
    // measured contrast(edge, panel-2) this way is 1.329-1.878, inside
    // Omarchy's own `selection` (1.129-2.455, a floor below the app's
    // reference dividers) and tighter than `muted` (1.321-4.895, whose top
    // end is a harsh rule, not a divider) — both considered and rejected on
    // the same evidence.
    let edge = mix(panel_2, ink, 0.20);
    let ink_dim = resolve_hue(&p.muted, surface, ink, 4.5);
    let accent = resolve_hue(&p.accent, surface, ink, 3.0);
    let accent_ink = resolve_accent_ink(surface, ink, accent);

    // The four syntax-highlight tokens (index.css:87-92) get read as body
    // text in a code block, so they're gated at the same 4.5:1 body-text bar
    // as ink-dim above.
    let ok = resolve_hue(&p.green, surface, ink, 4.5);
    let warn = resolve_hue(&p.yellow, surface, ink, 4.5);
    let brand = resolve_hue(&p.blue, surface, ink, 4.5);
    let brand_soft = resolve_hue(&p.cyan, surface, ink, 4.5);

    // The 11 stage/doc/activity hues preserve the equalities the default
    // palette already has (index.css:111-127) — research/working share
    // #38bdf8, validate/background share #a78bfa, implement/ship share
    // #34d399 — so a stage badge and its matching header action still read
    // the same hue: research/working ← blue, plan ← yellow, implement/ship
    // ← green, validate/background ← magenta, revise ← bright_magenta,
    // handoff ← orange() (orange, else bright_red, else red), commit ←
    // cyan, attention ← brown() (brown, else orange()). They're gated at
    // 3.0:1, the WCAG bar for UI components and large text rather than body
    // copy. `attention` takes `brown` rather than `yellow` because
    // index.css:98-102 states it is deliberately not --color-warn: warn
    // doubles as a syntax hue and reads neon, so attention surfaces use a
    // muted amber instead, and `brown` is Omarchy's muted-warm slot.
    //
    // Accepted gap: on a monochrome theme every derived hue is neutral.
    // `white` and `vantablack` are greyscale by design — `white`'s orange()
    // resolves to #2a2a2a, so its handoff and attention chips render dark
    // grey rather than warm. This is inherent to the theme the user chose,
    // not a mapping defect: unlike --color-bad, where a grey *error* is a
    // legibility problem, a grey attention chip is still perfectly legible
    // (#2a2a2a clears 3:1 against white's surface) and simply carries no
    // hue. Not worth a second pinned token — do not "fix" this later by
    // pinning stage hues too.
    //
    // The contrast gate in the corpus test cannot catch a hue problem of
    // this kind — #2a2a2a on white's surface passes 3.0 comfortably. That
    // test pins the resolved attention/handoff literals for last-horizon,
    // solitude and white instead, so a future change to either fallback
    // chain shows up as a diff rather than silently passing.
    let research = resolve_hue(&p.blue, surface, ink, 3.0);
    let working = research;
    let plan = resolve_hue(&p.yellow, surface, ink, 3.0);
    let implement = resolve_hue(&p.green, surface, ink, 3.0);
    let ship = implement;
    let validate = resolve_hue(&p.magenta, surface, ink, 3.0);
    let background = validate;
    let revise = resolve_hue(&p.bright_magenta, surface, ink, 3.0);
    let handoff = resolve_hue(p.orange(), surface, ink, 3.0);
    let commit = resolve_hue(&p.cyan, surface, ink, 3.0);
    let attention = resolve_hue(p.brown(), surface, ink, 3.0);

    // Reproduces the default's own relationship (index.css:135-141: Dark+
    // has surface #0c0c0c, nebula-bg-1 #0c0d11, nebula-bg-0 #151a26 — a
    // faint blue cast lifting further toward the star family). Hue names in
    // Omarchy palettes are slot names, not colour claims — on 8 of the 22
    // built-ins `bright_blue` isn't even blue-dominant (`matte-black`
    // #f59e0b, `ristretto` #f8a788, …) — so the ramp is often not a hue
    // sweep, and that's fine: every star still comes from the theme, which
    // is the property that matters. No contrast clamp: these are decorative
    // stars on a radial backdrop, not text.
    let nebula_1 = hex(&p.cyan).unwrap_or(ink);
    let nebula_2 = hex(&p.bright_blue).unwrap_or(ink);
    let nebula_3 = hex(&p.blue).unwrap_or(ink);
    let nebula_4 = hex(&p.bright_magenta).unwrap_or(ink);
    let nebula_5 = hex(&p.magenta).unwrap_or(ink);
    let nebula_bg_1 = mix(surface, nebula_3, 0.06);
    let nebula_bg_0 = mix(surface, nebula_3, 0.20);

    // Omarchy's own ANSI mapping, from its terminal templates. Alacritty
    // (/usr/share/omarchy/default/themed/alacritty.toml.tpl, [colors.normal]
    // / [colors.bright]) and ghostty (ghostty.conf.tpl, `palette = N=`)
    // agree exactly, so this is Omarchy's answer, not ours. It also
    // resolves the four keys absent from all 22 built-in themes — `black`,
    // `white`, `bright_black`, `bright_white` are never declared in a
    // colors.toml — by filling those slots from `background`, `foreground`,
    // `muted` and `bright_foreground` respectively, the same substitution
    // Omarchy's own templates make.
    //
    // Raw values, no contrast clamping: these are the terminal's own
    // palette, and a program printing `\e[31m` is asking for the theme's
    // red, not a legibility-adjusted one. ANSI 8 deliberately takes the raw
    // `muted` here while `ink_dim` above takes the clamped one — different
    // jobs, deliberately different values.
    //
    // Alacritty and ghostty use `background` for both the terminal
    // background and ANSI 0, so black-on-default is invisible there on
    // every theme. Hyprlayer's terminal background is --color-surface,
    // which is rank[0] — equal to `background` on only 6 of 22 themes (the
    // five light ones plus vantablack). On the other 16, `background` is
    // lighter than the ranked surface, so ANSI 0 renders as a visible dark
    // grey against our surface instead of vanishing. Not a defect to "fix":
    // the 16 ANSI values still match the user's other terminals on all 22
    // themes, which is the property that matters for a program printing
    // \e[31m — only the background tier differs, and that's the
    // ranked-surface rule above working as designed.
    let ansi_black = hex(&p.background).unwrap_or(ink);
    let ansi_red = hex(&p.red).unwrap_or(ink);
    let ansi_green = hex(&p.green).unwrap_or(ink);
    let ansi_yellow = hex(&p.yellow).unwrap_or(ink);
    let ansi_blue = hex(&p.blue).unwrap_or(ink);
    let ansi_magenta = hex(&p.magenta).unwrap_or(ink);
    let ansi_cyan = hex(&p.cyan).unwrap_or(ink);
    let ansi_white = hex(&p.foreground).unwrap_or(ink);
    let ansi_bright_black = hex(&p.muted).unwrap_or(ink);
    let ansi_bright_red = hex(&p.bright_red).unwrap_or(ink);
    let ansi_bright_green = hex(&p.bright_green).unwrap_or(ink);
    let ansi_bright_yellow = hex(&p.bright_yellow).unwrap_or(ink);
    let ansi_bright_blue = hex(&p.bright_blue).unwrap_or(ink);
    let ansi_bright_magenta = hex(&p.bright_magenta).unwrap_or(ink);
    let ansi_bright_cyan = hex(&p.bright_cyan).unwrap_or(ink);
    let ansi_bright_white = hex(&p.bright_foreground).unwrap_or(ink);

    let entries: [(&str, [u8; 3]); 46] = [
        ("--color-surface", surface),
        ("--color-panel", panel),
        ("--color-panel-2", panel_2),
        ("--color-edge", edge),
        ("--color-ink", ink),
        ("--color-ink-dim", ink_dim),
        ("--color-accent", accent),
        ("--color-accent-ink", accent_ink),
        ("--color-ok", ok),
        ("--color-warn", warn),
        // --color-bad is not derived. Omarchy's `red` is not required to be
        // red: vantablack's is #a4a4a4 and its bright_red is the same grey,
        // so a derived error colour would render every error state as
        // neutral text. The app's own red is used instead, supplied by the
        // CSS cascade rather than by this list (see applyTheme in
        // src/lib/theme.ts).
        ("--color-brand", brand),
        ("--color-brand-soft", brand_soft),
        ("--color-attention", attention),
        ("--color-ship", ship),
        ("--color-commit", commit),
        ("--color-research", research),
        ("--color-plan", plan),
        ("--color-implement", implement),
        ("--color-validate", validate),
        ("--color-revise", revise),
        ("--color-handoff", handoff),
        ("--color-working", working),
        ("--color-background", background),
        ("--color-nebula-bg-0", nebula_bg_0),
        ("--color-nebula-bg-1", nebula_bg_1),
        ("--color-nebula-1", nebula_1),
        ("--color-nebula-2", nebula_2),
        ("--color-nebula-3", nebula_3),
        ("--color-nebula-4", nebula_4),
        ("--color-nebula-5", nebula_5),
        ("--color-ansi-black", ansi_black),
        ("--color-ansi-red", ansi_red),
        ("--color-ansi-green", ansi_green),
        ("--color-ansi-yellow", ansi_yellow),
        ("--color-ansi-blue", ansi_blue),
        ("--color-ansi-magenta", ansi_magenta),
        ("--color-ansi-cyan", ansi_cyan),
        ("--color-ansi-white", ansi_white),
        ("--color-ansi-bright-black", ansi_bright_black),
        ("--color-ansi-bright-red", ansi_bright_red),
        ("--color-ansi-bright-green", ansi_bright_green),
        ("--color-ansi-bright-yellow", ansi_bright_yellow),
        ("--color-ansi-bright-blue", ansi_bright_blue),
        ("--color-ansi-bright-magenta", ansi_bright_magenta),
        ("--color-ansi-bright-cyan", ansi_bright_cyan),
        ("--color-ansi-bright-white", ansi_bright_white),
    ];

    let tokens = entries
        .into_iter()
        .map(|(name, rgb)| OmarchyToken {
            name: name.to_string(),
            value: to_hex(rgb),
        })
        .collect();

    OmarchyTheme { name, mode, tokens }
}

/// Whether Omarchy is installed, and its palette if one could be read.
///
/// The two answers are separate because they have opposite remedies and the
/// frontend must not conflate them. `installed: false` is permanent until the
/// user does something (Omarchy is gone, or was never here), so a stored
/// palette is stale and must be dropped. `installed: true, theme: None` is
/// the transient: Omarchy replaces the theme directory with `rm -rf` + `mv`
/// (/usr/bin/omarchy-theme-set:164-165), so a poll tick landing inside that
/// window reads no `colors.toml` for one cycle, and dropping the user's
/// palette for ~2s on every theme switch would be worse than keeping it.
///
/// `installed` cannot flicker across that window: [`current_dir`] probes
/// `theme.name`, which omarchy-theme-set:168 writes with a truncate-in-place
/// `echo >` *after* the directory swap. The file never ceases to exist, so a
/// theme switch can only ever produce the second shape, never the first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OmarchyStatus {
    pub installed: bool,
    pub theme: Option<OmarchyTheme>,
}

/// Probe Omarchy and resolve its active palette.
///
/// Infallible by design. A parse failure is logged and collapses to
/// `theme: None` rather than raising an error: the frontend's questions are
/// "is Omarchy here" and "is there a palette to apply", and a transient read
/// during a theme switch is not user-actionable. A *permanent* failure is
/// still reported as `installed: true, theme: None`, which the frontend
/// leaves alone — the accepted gap, and the reason the log line is `warn`.
pub fn status() -> OmarchyStatus {
    let Some(dir) = current_dir() else {
        return OmarchyStatus {
            installed: false,
            theme: None,
        };
    };
    let name = theme_name(&dir);
    let theme = match read_palette(&dir) {
        Ok(p) => Some(resolve(name, &p)),
        Err(e) => {
            tracing::warn!(error = %e, "omarchy palette read failed");
            None
        }
    };
    OmarchyStatus {
        installed: true,
        theme,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stage a vendored fixture's `colors.toml` (or arbitrary raw TOML) at
    /// the nested path [`read_palette`] expects: `<base>/theme/colors.toml`.
    /// The vendored fixture layout is flat (`tests/fixtures/omarchy/<theme>/
    /// colors.toml`); this reproduces the one hop the real
    /// `~/.local/state/omarchy/current/` directory has that the fixtures do
    /// not.
    fn stage(base: &Path, raw: &str) {
        let theme_dir = base.join("theme");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("colors.toml"), raw).unwrap();
    }

    /// Read a vendored fixture's `colors.toml` by theme name.
    fn fixture(theme: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/omarchy/{theme}/colors.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("missing vendored fixture for {theme}: {e}"))
    }

    // --- theme_name ---------------------------------------------------

    #[test]
    fn theme_name_on_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(theme_name(dir.path()), "");
    }

    #[test]
    fn theme_name_on_empty_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("theme.name"), b"").unwrap();
        assert_eq!(theme_name(dir.path()), "");
    }

    #[test]
    fn theme_name_trims_the_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("theme.name"), b"diablo-dreams\n").unwrap();
        assert_eq!(theme_name(dir.path()), "diablo-dreams");
    }

    // --- OmarchyMode::parse --------------------------------------------

    #[test]
    fn mode_parse_recognizes_light_case_and_whitespace_insensitively() {
        assert_eq!(OmarchyMode::parse("light"), OmarchyMode::Light);
        assert_eq!(OmarchyMode::parse("Light"), OmarchyMode::Light);
        assert_eq!(OmarchyMode::parse(" light "), OmarchyMode::Light);
    }

    #[test]
    fn mode_parse_defaults_to_dark() {
        assert_eq!(OmarchyMode::parse("dark"), OmarchyMode::Dark);
        assert_eq!(OmarchyMode::parse(""), OmarchyMode::Dark);
        assert_eq!(OmarchyMode::parse("garbage"), OmarchyMode::Dark);
    }

    // --- hex / to_hex ----------------------------------------------------

    #[test]
    fn hex_expands_shorthand() {
        assert_eq!(hex("#abc"), Some([0xaa, 0xbb, 0xcc]));
    }

    #[test]
    fn hex_parses_uppercase_and_round_trips_lowercase() {
        assert_eq!(hex("#FEF2C9"), Some([0xfe, 0xf2, 0xc9]));
        assert_eq!(to_hex(hex("#FEF2C9").unwrap()), "#fef2c9");
    }

    #[test]
    fn hex_rejects_malformed_input() {
        assert_eq!(hex("#gg0000"), None);
        assert_eq!(hex("#12345"), None);
        assert_eq!(hex("abc"), None);
    }

    // --- read_palette / validate -----------------------------------------

    #[test]
    fn missing_file_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_palette(dir.path()).unwrap_err();
        assert!(matches!(err, CoreError::Io(_)), "expected Io, got {err:?}");
    }

    #[test]
    fn empty_toml_fails_validation_rather_than_parsing_to_blank_strings() {
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), "");
        let err = read_palette(dir.path()).unwrap_err();
        assert!(
            matches!(err, CoreError::OmarchyPaletteParse { .. }),
            "expected OmarchyPaletteParse, got {err:?}"
        );
    }

    #[test]
    fn partial_toml_fails_to_parse() {
        // First 40 bytes of a real vendored fixture: a truncated write caught
        // mid-echo, not a synthetic string.
        let full = fixture("tokyo-night");
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), &full[..40]);
        let err = read_palette(dir.path()).unwrap_err();
        assert!(
            matches!(err, CoreError::OmarchyPaletteParse { .. }),
            "expected OmarchyPaletteParse, got {err:?}"
        );
    }

    #[test]
    fn unknown_keys_are_ignored() {
        // last-horizon ships hyprland_active_border, hyprland_inactive_border,
        // active_border_color and active_tab_background — none of which
        // OmarchyPalette declares.
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), &fixture("last-horizon"));
        let palette = read_palette(dir.path()).unwrap();
        assert_eq!(palette.background, "#0c0b0c");
    }

    #[test]
    fn missing_orange_and_brown_fall_back_through_bright_red() {
        // The three built-ins missing both keys, and the literal fallback
        // value each resolves to.
        let cases = [
            ("last-horizon", "#c38b7b"),
            ("solitude", "#de6145"),
            ("white", "#2a2a2a"),
        ];
        for (theme, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            stage(dir.path(), &fixture(theme));
            let palette = read_palette(dir.path()).unwrap();
            assert_eq!(palette.orange, "", "{theme} should have no orange key");
            assert_eq!(palette.brown, "", "{theme} should have no brown key");
            assert_eq!(palette.orange(), expected, "{theme} orange() fallback");
            assert_eq!(palette.brown(), expected, "{theme} brown() fallback");
        }
    }

    #[test]
    fn malformed_orange_and_brown_fall_back_the_same_way_a_missing_one_does() {
        // `orange` and `brown` are the two fields validate() skips, so a user
        // theme carrying garbage in either reaches the accessors. Gating on
        // emptiness instead of parseability returned it verbatim and
        // resolve_hue's `unwrap_or(ink)` painted --color-handoff as body ink.
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), &fixture("tokyo-night"));
        let mut palette = read_palette(dir.path()).unwrap();
        palette.orange = "not-a-color".into();
        palette.brown = "#12345".into();
        assert_eq!(palette.orange(), palette.bright_red);
        assert_eq!(palette.brown(), palette.bright_red);
        assert!(hex(palette.orange()).is_some());
        assert!(hex(palette.brown()).is_some());
    }
    // --- luminance ---------------------------------------------------------

    #[test]
    fn luminance_of_black_is_zero() {
        assert_eq!(luminance([0, 0, 0]), 0.0);
    }

    #[test]
    fn luminance_of_white_is_one() {
        assert!((luminance([255, 255, 255]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn luminance_of_the_three_primaries_matches_the_wcag_channel_weights() {
        // Each primary isolates one WCAG weight: at full intensity the
        // linearised channel is exactly 1.0, so luminance reduces to the
        // weight itself.
        assert!((luminance([255, 0, 0]) - 0.2126).abs() < 1e-9);
        assert!((luminance([0, 255, 0]) - 0.7152).abs() < 1e-9);
        assert!((luminance([0, 0, 255]) - 0.0722).abs() < 1e-9);
    }

    // --- contrast ------------------------------------------------------------

    #[test]
    fn contrast_is_symmetric() {
        let a = [0x33, 0x33, 0x33];
        let b = [0xfa, 0xf9, 0xf6];
        assert_eq!(contrast(a, b), contrast(b, a));
    }

    #[test]
    fn contrast_of_black_and_white_is_the_wcag_maximum() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
    }

    // --- mix -------------------------------------------------------------

    #[test]
    fn mix_at_the_endpoints_returns_the_endpoint_unchanged() {
        let a = [0x0c, 0x0c, 0x0c];
        let b = [0xff, 0xff, 0xff];
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }

    #[test]
    fn mix_interpolates_linearly_in_srgb_byte_space() {
        assert_eq!(mix([0, 0, 0], [100, 100, 100], 0.5), [50, 50, 50]);
    }

    // --- ensure_contrast -------------------------------------------------

    #[test]
    fn ensure_contrast_returns_fg_unchanged_when_it_already_clears() {
        let fg = [0, 0, 0];
        let bg = [255, 255, 255];
        let ink = [0, 0, 0];
        assert_eq!(ensure_contrast(fg, bg, 4.5, ink), fg);
    }

    #[test]
    fn ensure_contrast_terminates_at_ink_when_no_ratio_this_high_is_reachable() {
        // 25:1 exceeds the WCAG maximum (21:1, pure black on pure white), so
        // no step toward `ink` — not even `ink` itself — can clear it. The
        // walk must fall through every iteration to the explicit `ink`
        // fallback.
        let fg = [0xc8, 0xc8, 0xc8];
        let bg = [0xff, 0xff, 0xff];
        let ink = [0, 0, 0];
        assert_eq!(ensure_contrast(fg, bg, 25.0, ink), ink);
    }

    #[test]
    fn ensure_contrast_is_monotonic_in_the_number_of_steps() {
        // A low-contrast fg walking toward a high-contrast ink: each step's
        // contrast against bg must never regress from the step before.
        let bg = [0x0c, 0x0c, 0x0c];
        let fg = [0x20, 0x20, 0x20];
        let ink = [0xff, 0xff, 0xff];
        let mut previous = contrast(fg, bg);
        for step in 1..=20 {
            let candidate = mix(fg, ink, step as f64 * 0.05);
            let current = contrast(candidate, bg);
            assert!(
                current + 1e-9 >= previous,
                "step {step}: contrast {current} regressed from {previous}"
            );
            previous = current;
        }
    }

    // --- rank_backgrounds --------------------------------------------------

    #[test]
    fn rank_backgrounds_orders_dark_mode_darkest_first() {
        let p = OmarchyPalette {
            background: "#202020".to_string(),
            darker_background: "#101010".to_string(),
            dark_background: "#181818".to_string(),
            lighter_background: "#303030".to_string(),
            ..Default::default()
        };
        let rank = rank_backgrounds(&p, OmarchyMode::Dark);
        assert_eq!(
            rank,
            [
                hex(&p.darker_background).unwrap(),
                hex(&p.dark_background).unwrap(),
                hex(&p.background).unwrap(),
                hex(&p.lighter_background).unwrap(),
            ]
        );
    }

    #[test]
    fn rank_backgrounds_orders_light_mode_lightest_first() {
        let p = OmarchyPalette {
            background: "#faf9f6".to_string(),
            darker_background: "#e8e8e8".to_string(),
            dark_background: "#d0d0d0".to_string(),
            lighter_background: "#ffffff".to_string(),
            ..Default::default()
        };
        let rank = rank_backgrounds(&p, OmarchyMode::Light);
        assert_eq!(
            rank,
            [
                hex(&p.lighter_background).unwrap(),
                hex(&p.background).unwrap(),
                hex(&p.darker_background).unwrap(),
                hex(&p.dark_background).unwrap(),
            ]
        );
    }

    #[test]
    fn rank_backgrounds_breaks_ties_by_field_position() {
        // background and lighter_background byte-for-byte equal — the
        // last-horizon/solitude case. background (field index 0) must sort
        // ahead of lighter_background (field index 3) once the sort reaches
        // the tied pair.
        let p = OmarchyPalette {
            background: "#0c0b0c".to_string(),
            darker_background: "#000000".to_string(),
            dark_background: "#050505".to_string(),
            lighter_background: "#0c0b0c".to_string(),
            ..Default::default()
        };
        let rank = rank_backgrounds(&p, OmarchyMode::Dark);
        assert_eq!(rank[2], hex(&p.background).unwrap());
        assert_eq!(rank[3], hex(&p.lighter_background).unwrap());
    }
}
