#[cfg(native)]
#[cfg_attr(not(macos), allow(dead_code))]
pub mod font_kit;

#[cfg(test)]
#[path = "text_layout_test.rs"]
mod text_layout_tests;

pub use warpui_core::fonts::*;

/// Use real text metrics in headless layout tests instead of the zero-width test font.
/// Callers load their bundled fonts into this cache, without opening a native window.
#[cfg(all(feature = "test-util", native))]
pub fn platform_font_cache_for_tests() -> Cache {
    #[cfg(target_os = "macos")]
    let font_db = crate::platform::mac::fonts::FontDB::new();
    #[cfg(not(target_os = "macos"))]
    let font_db = crate::windowing::winit::fonts::FontDB::new();
    Cache::new(Box::new(font_db))
}

#[cfg(test)]
pub(crate) use text_layout_tests::{collect_glyph_indices, init_fonts};

#[cfg(all(test, target_os = "macos"))]
pub(crate) use text_layout_tests::collect_line_caret_position_starts;
