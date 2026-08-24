//! Sweeps the vendored 22-theme Omarchy fixture corpus through parse,
//! validate, resolve, and the resolved-token contrast gates.
//!
//! Vendored, not read from `/usr/share/omarchy`, because the release
//! builders are macOS and Windows (ADR-0001) and a corpus test that silently
//! skips off Omarchy is a corpus test that only ever ran once. This is the
//! opposite choice from `docs_corpus_smoke.rs`, whose corpus is the user's
//! own thoughts tree and genuinely cannot be vendored — noted here so nobody
//! "fixes" this file to match that one later.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hyprlayer_core::omarchy;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/omarchy")
}

/// Every vendored theme name (one directory per theme, each holding a
/// `colors.toml`), sorted for deterministic failure output.
fn theme_names() -> Vec<String> {
    let mut names: Vec<String> = walkdir::WalkDir::new(fixtures_dir())
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Stage a vendored fixture at the nested path [`omarchy::read_palette`]
/// expects (`<dir>/theme/colors.toml`, mirroring
/// `~/.local/state/omarchy/current/`) and parse it.
fn read_fixture(name: &str) -> omarchy::OmarchyPalette {
    let staged = tempfile::tempdir().expect("tempdir");
    let theme_dir = staged.path().join("theme");
    std::fs::create_dir_all(&theme_dir).expect("create staged theme dir");
    std::fs::copy(
        fixtures_dir().join(name).join("colors.toml"),
        theme_dir.join("colors.toml"),
    )
    .unwrap_or_else(|e| panic!("failed to stage fixture {name}: {e}"));

    omarchy::read_palette(staged.path())
        .unwrap_or_else(|e| panic!("{name}: failed to parse/validate: {e}"))
}

/// Stage and resolve a vendored fixture in one step.
fn resolve_fixture(name: &str) -> omarchy::OmarchyTheme {
    let palette = read_fixture(name);
    omarchy::resolve(name.to_string(), &palette)
}

#[test]
fn corpus_has_exactly_22_vendored_themes() {
    let names = theme_names();
    // A lost or mis-copied fixture must fail loudly rather than silently
    // shrinking the corpus.
    assert_eq!(
        names.len(),
        22,
        "expected 22 vendored Omarchy theme fixtures, found {}: {names:?}",
        names.len()
    );
}

#[test]
fn every_fixture_parses_and_validates() {
    for name in theme_names() {
        read_fixture(&name);
    }
}

// ---------------------------------------------------------------------
// Phase 2: resolved-token contrast gates.
//
// `luminance`/`contrast` are re-implemented here rather than imported —
// `omarchy::resolve`'s own copies are private (Phase 2's design: "private
// helpers, each with unit tests" in `omarchy.rs`'s own `mod tests`) — so
// this suite checks `resolve()`'s *output* against an independent
// implementation of the same WCAG formula, rather than exercising one
// implementation to validate itself.
// ---------------------------------------------------------------------

/// WCAG 2.1 relative luminance of an sRGB colour, in `[0.0, 1.0]`.
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

/// WCAG 2.1 contrast ratio between two colours, order-independent.
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The resolved value of `name` in `theme.tokens`, panicking with the theme
/// name if it is missing — every assertion below wants failures to name
/// both the theme and the token.
fn token_value<'a>(theme: &'a omarchy::OmarchyTheme, name: &str) -> &'a str {
    theme
        .tokens
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("{}: missing token {name}", theme.name))
        .value
        .as_str()
}

fn token_rgb(theme: &omarchy::OmarchyTheme, name: &str) -> [u8; 3] {
    let value = token_value(theme, name);
    omarchy::hex(value)
        .unwrap_or_else(|| panic!("{}: {name} = {value:?} is not valid hex", theme.name))
}

/// The 31 `--color-*` custom properties declared in the `@theme` block at
/// `src/index.css:76-166`, before the `--color-ansi-*` block Phase 5
/// appends (see [`EXPECTED_ANSI_TOKEN_NAMES`] below), minus `--color-bad`
/// (deliberately never derived — see the comment inside `omarchy::resolve`).
/// Verified directly against the file — `awk 'NR==76,NR==141' src/index.css
/// | grep -oE '^\s*--color-[a-z0-9-]+:'` — rather than transcribed from the
/// plan, and listed here in the same order `resolve()` emits them.
const EXPECTED_TOKEN_NAMES: [&str; 30] = [
    "--color-surface",
    "--color-panel",
    "--color-panel-2",
    "--color-edge",
    "--color-ink",
    "--color-ink-dim",
    "--color-accent",
    "--color-accent-ink",
    "--color-ok",
    "--color-warn",
    "--color-brand",
    "--color-brand-soft",
    "--color-attention",
    "--color-ship",
    "--color-commit",
    "--color-research",
    "--color-plan",
    "--color-implement",
    "--color-validate",
    "--color-revise",
    "--color-handoff",
    "--color-working",
    "--color-background",
    "--color-nebula-bg-0",
    "--color-nebula-bg-1",
    "--color-nebula-1",
    "--color-nebula-2",
    "--color-nebula-3",
    "--color-nebula-4",
    "--color-nebula-5",
];

/// The 16 `--color-ansi-*` custom properties Phase 5 appends to the same
/// `@theme` block, immediately after [`EXPECTED_TOKEN_NAMES`]
/// (`src/index.css:143-165`). Listed in ANSI index order (0-15), the same
/// order `omarchy::resolve` appends them in.
const EXPECTED_ANSI_TOKEN_NAMES: [&str; 16] = [
    "--color-ansi-black",
    "--color-ansi-red",
    "--color-ansi-green",
    "--color-ansi-yellow",
    "--color-ansi-blue",
    "--color-ansi-magenta",
    "--color-ansi-cyan",
    "--color-ansi-white",
    "--color-ansi-bright-black",
    "--color-ansi-bright-red",
    "--color-ansi-bright-green",
    "--color-ansi-bright-yellow",
    "--color-ansi-bright-blue",
    "--color-ansi-bright-magenta",
    "--color-ansi-bright-cyan",
    "--color-ansi-bright-white",
];

#[test]
fn every_fixture_resolves_to_exactly_the_46_expected_tokens() {
    let expected: BTreeSet<&str> = EXPECTED_TOKEN_NAMES
        .iter()
        .chain(EXPECTED_ANSI_TOKEN_NAMES.iter())
        .copied()
        .collect();
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        assert_eq!(
            theme.tokens.len(),
            46,
            "{name}: expected 46 tokens, got {}",
            theme.tokens.len()
        );
        let actual: BTreeSet<&str> = theme.tokens.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            actual, expected,
            "{name}: resolved token name set does not match the 31 @theme names minus \
             --color-bad, plus the 16 --color-ansi-* names"
        );
        assert!(
            !actual.contains("--color-bad"),
            "{name}: --color-bad must never be derived"
        );
        // Phase 5 strictly APPENDS the ANSI block; it must never reorder or
        // interleave with the 30 tokens Phase 2 already committed, so the
        // first 30 stay positionally identical to EXPECTED_TOKEN_NAMES.
        let first_30: Vec<&str> = theme.tokens[..30].iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            first_30,
            EXPECTED_TOKEN_NAMES.as_slice(),
            "{name}: first 30 tokens changed order/identity — the ANSI block must be \
             strictly appended, never interleaved"
        );
    }
}

#[test]
fn ink_clears_4_5_against_surface_on_all_22() {
    let mut min = (f64::INFINITY, String::new());
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        let surface = token_rgb(&theme, "--color-surface");
        let ink = token_rgb(&theme, "--color-ink");
        let ratio = contrast(ink, surface);
        assert!(
            ratio >= 4.5,
            "{name}: ink/surface contrast {ratio:.4} < 4.5"
        );
        if ratio < min.0 {
            min = (ratio, name.clone());
        }
    }
    eprintln!("ink/surface minimum: {:.4} ({})", min.0, min.1);
}

#[test]
fn ink_dim_clears_4_5_against_surface_on_all_22() {
    let mut min = (f64::INFINITY, String::new());
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        let surface = token_rgb(&theme, "--color-surface");
        let ink_dim = token_rgb(&theme, "--color-ink-dim");
        let ratio = contrast(ink_dim, surface);
        assert!(
            ratio >= 4.5,
            "{name}: ink-dim/surface contrast {ratio:.4} < 4.5"
        );
        if ratio < min.0 {
            min = (ratio, name.clone());
        }
    }
    eprintln!("ink-dim/surface minimum: {:.4} ({})", min.0, min.1);
}

#[test]
fn syntax_hues_clear_4_5_against_surface_on_all_22() {
    let tokens = [
        "--color-ok",
        "--color-warn",
        "--color-brand",
        "--color-brand-soft",
    ];
    let mut min = (f64::INFINITY, String::new(), "");
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        let surface = token_rgb(&theme, "--color-surface");
        for &tok in &tokens {
            let ratio = contrast(token_rgb(&theme, tok), surface);
            assert!(
                ratio >= 4.5,
                "{name}/{tok}: contrast {ratio:.4} < 4.5 against surface"
            );
            if ratio < min.0 {
                min = (ratio, name.clone(), tok);
            }
        }
    }
    eprintln!(
        "ok|warn|brand|brand-soft minimum: {:.4} ({}/{})",
        min.0, min.1, min.2
    );
}

#[test]
fn accent_clears_3_0_against_surface_on_all_22() {
    let mut min = (f64::INFINITY, String::new());
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        let surface = token_rgb(&theme, "--color-surface");
        let accent = token_rgb(&theme, "--color-accent");
        let ratio = contrast(accent, surface);
        assert!(
            ratio >= 3.0,
            "{name}: accent/surface contrast {ratio:.4} < 3.0"
        );
        if ratio < min.0 {
            min = (ratio, name.clone());
        }
    }
    eprintln!("accent/surface minimum: {:.4} ({})", min.0, min.1);
}

#[test]
fn accent_ink_clears_4_5_against_accent_on_all_22() {
    let mut min = (f64::INFINITY, String::new());
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        let accent = token_rgb(&theme, "--color-accent");
        let accent_ink = token_rgb(&theme, "--color-accent-ink");
        let ratio = contrast(accent_ink, accent);
        assert!(
            ratio >= 4.5,
            "{name}: accent-ink/accent contrast {ratio:.4} < 4.5"
        );
        if ratio < min.0 {
            min = (ratio, name.clone());
        }
    }
    eprintln!("accent-ink/accent minimum: {:.4} ({})", min.0, min.1);
}

#[test]
fn stage_doc_and_activity_hues_clear_3_0_against_surface_on_all_22() {
    let tokens = [
        "--color-research",
        "--color-working",
        "--color-plan",
        "--color-implement",
        "--color-ship",
        "--color-validate",
        "--color-background",
        "--color-revise",
        "--color-handoff",
        "--color-commit",
        "--color-attention",
    ];
    let mut min = (f64::INFINITY, String::new(), "");
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        let surface = token_rgb(&theme, "--color-surface");
        for &tok in &tokens {
            let ratio = contrast(token_rgb(&theme, tok), surface);
            assert!(
                ratio >= 3.0,
                "{name}/{tok}: contrast {ratio:.4} < 3.0 against surface"
            );
            if ratio < min.0 {
                min = (ratio, name.clone(), tok);
            }
        }
    }
    eprintln!(
        "stage/doc/activity hues minimum: {:.4} ({}/{})",
        min.0, min.1, min.2
    );
}

#[test]
fn edge_panel_2_contrast_stays_within_the_measured_band_on_all_22() {
    // mix(panel_2, ink, 0.20) brackets the app's own reference dividers
    // (Dark+ #333333 on #0c0c0c is 1.52:1; html.light's #d4d0c8 on #faf9f6
    // is 1.44:1) tighter than either raw Omarchy candidate considered and
    // rejected for this token (Omarchy's `selection`: 1.129-2.455;
    // `muted`: 1.321-4.895 — see the doc comment on `rank_backgrounds` in
    // omarchy.rs). The upper bound is asserted, not just the floor: an
    // unbounded check would pass a harsh ~4.9:1 rule, which is what picking
    // `muted` for this token would have produced.
    let mut min = (f64::INFINITY, String::new());
    let mut max = (f64::NEG_INFINITY, String::new());
    for name in theme_names() {
        let theme = resolve_fixture(&name);
        let edge = token_rgb(&theme, "--color-edge");
        let panel_2 = token_rgb(&theme, "--color-panel-2");
        let ratio = contrast(edge, panel_2);
        assert!(
            (1.30..=1.90).contains(&ratio),
            "{name}: edge/panel-2 contrast {ratio:.4} outside 1.30..=1.90"
        );
        if ratio < min.0 {
            min = (ratio, name.clone());
        }
        if ratio > max.0 {
            max = (ratio, name.clone());
        }
    }
    eprintln!(
        "edge/panel-2 range: {:.4} ({}) .. {:.4} ({})",
        min.0, min.1, max.0, max.1
    );
}

#[test]
fn attention_and_handoff_literals_on_the_orange_and_brown_fallback_themes() {
    // The three built-ins missing both `orange` and `brown`
    // (`missing_orange_and_brown_fall_back_through_bright_red` in
    // omarchy.rs's own tests covers the raw fallback values). Neither of
    // these three needs clamping — each raw fallback already clears 3.0
    // against its theme's surface — so the resolved value is the raw
    // fallback verbatim. Pinned here as literals because the contrast gates
    // above cannot catch a hue regression of this kind: a future change to
    // either fallback chain must show up as a diff, not pass silently.
    let cases = [
        ("last-horizon", "#c38b7b"),
        ("solitude", "#de6145"),
        ("white", "#2a2a2a"),
    ];
    for (name, expected) in cases {
        let theme = resolve_fixture(name);
        assert_eq!(
            token_value(&theme, "--color-attention"),
            expected,
            "{name}: --color-attention"
        );
        assert_eq!(
            token_value(&theme, "--color-handoff"),
            expected,
            "{name}: --color-handoff"
        );
    }
}

#[test]
fn ink_floor_fires_on_a_synthetic_low_contrast_palette() {
    // None of the 22 built-in themes exercises the floor in `resolve_ink`
    // (measured: every built-in clears 4.5:1 on its own foreground family,
    // minimum 6.66 on rose-pine) — the floor exists for user-installed
    // themes the vendored corpus cannot cover by construction, so it needs
    // a hand-built palette. Every `*foreground` field here sits within 2:1
    // contrast of the ranked-darkest background (`darker_background`,
    // #181818 — the best candidate, light_foreground, only reaches
    // ~1.71:1), so the flooring branch to max(black, white) must fire.
    let palette = omarchy::OmarchyPalette {
        mode: "dark".to_string(),
        accent: "#d97757".to_string(),
        selection: "#444444".to_string(),
        muted: "#555555".to_string(),
        background: "#202020".to_string(),
        dark_background: "#1c1c1c".to_string(),
        darker_background: "#181818".to_string(),
        lighter_background: "#242424".to_string(),
        foreground: "#3a3a3a".to_string(),
        dark_foreground: "#363636".to_string(),
        light_foreground: "#404040".to_string(),
        bright_foreground: "#3e3e3e".to_string(),
        red: "#ff0000".to_string(),
        yellow: "#ffff00".to_string(),
        orange: "#ffa500".to_string(),
        green: "#00ff00".to_string(),
        cyan: "#00ffff".to_string(),
        blue: "#0000ff".to_string(),
        magenta: "#ff00ff".to_string(),
        brown: "#a52a2a".to_string(),
        bright_red: "#ff5555".to_string(),
        bright_yellow: "#ffff55".to_string(),
        bright_green: "#55ff55".to_string(),
        bright_cyan: "#55ffff".to_string(),
        bright_blue: "#5555ff".to_string(),
        bright_magenta: "#ff55ff".to_string(),
    };
    let theme = omarchy::resolve("synthetic-low-contrast".to_string(), &palette);

    let surface = token_rgb(&theme, "--color-surface");
    assert_eq!(
        surface,
        [0x18, 0x18, 0x18],
        "surface should rank darker_background (#181818) first in dark mode"
    );

    let ink = token_rgb(&theme, "--color-ink");
    let ratio = contrast(ink, surface);
    assert!(
        ratio >= 4.5,
        "ink floor did not fire: ink={ink:?} surface={surface:?} contrast={ratio:.4}"
    );
    // None of the four raw *foreground* candidates clears 4.5 on their own,
    // so a passing ink here can only be the max(black, white) floor —
    // confirmed directly rather than just inferred from the ratio.
    assert_eq!(
        ink,
        [0xff, 0xff, 0xff],
        "expected the floor to pick white over surface #181818"
    );
}
