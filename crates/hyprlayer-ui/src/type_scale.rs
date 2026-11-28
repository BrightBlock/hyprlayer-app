//! The twelve v4 type tokens (ADR-0006 "v4 type scale"). Callers name a token, never a size.
//! GPUI has no letter spacing or text transform, so `label`/`tag` callers uppercase their text.

use gpui::{px, relative, FontWeight, Pixels, Styled};

use crate::fonts;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Display,
    H1,
    H2,
    H3,
    Title,
    BodyLg,
    Body,
    Ui,
    UiSm,
    Meta,
    Label,
    Tag,
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub size: f32,
    pub line_height: f32,
    pub weight: FontWeight,
    pub family: &'static str,
}

impl Type {
    pub fn spec(self) -> Spec {
        let (size, line_height, weight, family) = match self {
            Self::Display => (80.0, 1.02, FontWeight::NORMAL, fonts::SERIF),
            Self::H1 => (32.0, 1.15, FontWeight::BOLD, fonts::SANS),
            Self::H2 => (22.0, 1.2, FontWeight::BOLD, fonts::SANS),
            Self::H3 => (19.0, 1.25, FontWeight::BOLD, fonts::SANS),
            Self::Title => (15.0, 1.3, FontWeight::BOLD, fonts::SANS),
            Self::BodyLg => (14.5, 1.65, FontWeight::NORMAL, fonts::SANS),
            Self::Body => (13.0, 1.4, FontWeight::NORMAL, fonts::SANS),
            Self::Ui => (12.5, 1.35, FontWeight::NORMAL, fonts::SANS),
            Self::UiSm => (11.5, 1.35, FontWeight::NORMAL, fonts::SANS),
            Self::Meta => (11.0, 1.4, FontWeight::NORMAL, fonts::MONO),
            Self::Label => (10.0, 1.2, FontWeight::SEMIBOLD, fonts::MONO),
            Self::Tag => (9.5, 1.0, FontWeight::SEMIBOLD, fonts::MONO),
        };
        Spec {
            size,
            line_height,
            weight,
            family,
        }
    }

    pub fn size(self) -> Pixels {
        px(self.spec().size)
    }
}

pub trait Typed: Styled + Sized {
    fn typed(self, token: Type) -> Self {
        let spec = token.spec();
        self.text_size(px(spec.size))
            .line_height(relative(spec.line_height))
            .font_weight(spec.weight)
            .font_family(spec.family)
    }
}

impl<T: Styled + Sized> Typed for T {}
