use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

// `state_dir`/`config_dir` are OS-convention roots the caller resolves (e.g. via
// `dirs::state_dir()`/`dirs::config_dir()`); core only joins the fixed suffix and
// checks for the marker file, so it never reads the platform environment itself.
pub fn current_dir(state_dir: Option<&Path>, config_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(state_dir) = state_dir {
        let candidate = state_dir.join("omarchy/current");
        if candidate.join("theme.name").is_file() {
            return Some(candidate);
        }
    }
    if let Some(config_dir) = config_dir {
        let candidate = config_dir.join("omarchy/current");
        if candidate.join("theme.name").is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn theme_name(current: &Path) -> String {
    std::fs::read_to_string(current.join("theme.name"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

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

    pub fn orange(&self) -> &str {
        if hex(&self.orange).is_some() {
            &self.orange
        } else if hex(&self.bright_red).is_some() {
            &self.bright_red
        } else {
            &self.red
        }
    }

    pub fn brown(&self) -> &str {
        if hex(&self.brown).is_some() {
            &self.brown
        } else {
            self.orange()
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum OmarchyMode {
    Dark,
    Light,
}

impl OmarchyMode {
    pub fn parse(s: &str) -> OmarchyMode {
        if s.trim().eq_ignore_ascii_case("light") {
            OmarchyMode::Light
        } else {
            OmarchyMode::Dark
        }
    }
}

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

pub fn hex(s: &str) -> Option<[u8; 3]> {
    let digits = s.strip_prefix('#')?;
    let expanded: String = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 => digits.to_string(),
        _ => return None,
    };
    let mut channels = expanded.as_bytes().as_chunks::<2>().0.iter().map(|pair| {
        std::str::from_utf8(pair)
            .ok()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
    });
    Some([channels.next()??, channels.next()??, channels.next()??])
}

pub fn to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

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

fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let lerp = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    [lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2])]
}

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

fn resolve_hue(source: &str, surface: [u8; 3], ink: [u8; 3], ratio: f64) -> [u8; 3] {
    ensure_contrast(hex(source).unwrap_or(ink), surface, ratio, ink)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OmarchyToken {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OmarchyTheme {
    pub name: String,
    pub mode: OmarchyMode,
    pub tokens: Vec<OmarchyToken>,
}

pub fn resolve(name: String, p: &OmarchyPalette) -> OmarchyTheme {
    let mode = OmarchyMode::parse(&p.mode);
    let rank = rank_backgrounds(p, mode);
    let (surface, panel, panel_2) = (rank[0], rank[1], rank[2]);

    let ink = resolve_ink(p, surface);

    let edge = mix(panel_2, ink, 0.20);
    let ink_dim = resolve_hue(&p.muted, surface, ink, 4.5);
    let accent = resolve_hue(&p.accent, surface, ink, 3.0);
    let accent_ink = resolve_accent_ink(surface, ink, accent);

    let ok = resolve_hue(&p.green, surface, ink, 4.5);
    let warn = resolve_hue(&p.yellow, surface, ink, 4.5);
    let brand = resolve_hue(&p.blue, surface, ink, 4.5);
    let brand_soft = resolve_hue(&p.cyan, surface, ink, 4.5);

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

    let nebula_1 = hex(&p.cyan).unwrap_or(ink);
    let nebula_2 = hex(&p.bright_blue).unwrap_or(ink);
    let nebula_3 = hex(&p.blue).unwrap_or(ink);
    let nebula_4 = hex(&p.bright_magenta).unwrap_or(ink);
    let nebula_5 = hex(&p.magenta).unwrap_or(ink);
    let nebula_bg_1 = mix(surface, nebula_3, 0.06);
    let nebula_bg_0 = mix(surface, nebula_3, 0.20);

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct OmarchyStatus {
    pub installed: bool,
    pub theme: Option<OmarchyTheme>,
}

pub fn status(dir: Option<&Path>) -> OmarchyStatus {
    let Some(dir) = dir else {
        return OmarchyStatus {
            installed: false,
            theme: None,
        };
    };
    let name = theme_name(dir);
    let theme = match read_palette(dir) {
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

    fn stage(base: &Path, raw: &str) {
        let theme_dir = base.join("theme");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("colors.toml"), raw).unwrap();
    }

    fn fixture(theme: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/omarchy/{theme}/colors.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|e| panic!("missing vendored fixture for {theme}: {e}"))
    }

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
    fn malformed_orange_and_brown_fall_back_the_same_way_a_missing_one_does() {
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
        assert!((luminance([255, 0, 0]) - 0.2126).abs() < 1e-9);
        assert!((luminance([0, 255, 0]) - 0.7152).abs() < 1e-9);
        assert!((luminance([0, 0, 255]) - 0.0722).abs() < 1e-9);
    }

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

    #[test]
    fn ensure_contrast_returns_fg_unchanged_when_it_already_clears() {
        let fg = [0, 0, 0];
        let bg = [255, 255, 255];
        let ink = [0, 0, 0];
        assert_eq!(ensure_contrast(fg, bg, 4.5, ink), fg);
    }

    #[test]
    fn ensure_contrast_terminates_at_ink_when_no_ratio_this_high_is_reachable() {
        let fg = [0xc8, 0xc8, 0xc8];
        let bg = [0xff, 0xff, 0xff];
        let ink = [0, 0, 0];
        assert_eq!(ensure_contrast(fg, bg, 25.0, ink), ink);
    }

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
