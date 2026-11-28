//! The families `src/index.css` names. Nothing is registered: `@fontsource` ships only woff and
//! woff2, which neither Core Text nor fontdb loads, so an uninstalled family falls back.

pub const SANS: &str = "Inter";
pub const MONO: &str = "JetBrains Mono";
pub const SERIF: &str = "Instrument Serif";
