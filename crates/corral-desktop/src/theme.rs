//! The Desktop's colours and type, in one place.

use gpui::{Font, FontFeatures, FontStyle, FontWeight, Hsla, Pixels, Size, Window, px, rgb, size};

/// The monospace family the terminal is painted in.
///
/// A platform's default monospace face rather than a bundled font: PR9 ships
/// no assets, and the cell metrics are measured from whatever resolves.
pub fn monospace() -> Font {
    Font {
        family: platform::MONOSPACE_FAMILY.into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

mod platform {
    #[cfg(target_os = "macos")]
    pub const MONOSPACE_FAMILY: &str = "Menlo";
    #[cfg(not(target_os = "macos"))]
    pub const MONOSPACE_FAMILY: &str = "DejaVu Sans Mono";
}

/// The terminal's type size.
pub const TERMINAL_FONT_PX: f32 = 12.0;

/// One cell of the terminal grid for a font, measured from the font itself:
/// the advance of a narrow glyph, and a line height a quarter over the size.
pub fn cell_size(window: &Window, font: &Font, font_px: Pixels) -> Size<Pixels> {
    let text_system = window.text_system();
    let id = text_system.resolve_font(font);
    // A font whose advance cannot be measured is one the platform could not
    // load; the width falls back to the conventional 0.6 em so the grid still
    // exists, and the glyphs paint with whatever fallback the system chose.
    let width = text_system
        .advance(id, font_px, 'M')
        .map(|advance| advance.width)
        .unwrap_or(font_px * 0.6);
    size(width, (font_px * 1.25).round())
}

pub fn window_bg() -> Hsla {
    rgb(0x1b1b1d).into()
}

pub fn panel_bg() -> Hsla {
    rgb(0x232326).into()
}

pub fn terminal_bg() -> Hsla {
    rgb(0x1e1e1e).into()
}

pub fn terminal_fg() -> Hsla {
    rgb(0xd4d4d4).into()
}

pub fn text() -> Hsla {
    rgb(0xe6e6e6).into()
}

pub fn muted() -> Hsla {
    rgb(0x9a9a9f).into()
}

pub fn border() -> Hsla {
    rgb(0x3a3a3f).into()
}

pub fn accent() -> Hsla {
    rgb(0x4f8cff).into()
}

pub fn selected_bg() -> Hsla {
    rgb(0x2f3a55).into()
}

pub fn hover_bg() -> Hsla {
    rgb(0x2b2b30).into()
}

pub fn button_bg() -> Hsla {
    rgb(0x34343a).into()
}

pub fn danger() -> Hsla {
    rgb(0xe06c75).into()
}

pub fn cursor() -> Hsla {
    gpui::hsla(0.55, 0.8, 0.6, 0.6)
}

pub fn ui_font_px() -> Pixels {
    px(13.0)
}
