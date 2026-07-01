use pathfinder_color::ColorU;
use warp_core::ui::{
    color::{blend::Blend, coloru_with_opacity, OPAQUE},
    theme::{
        AnsiColor, AnsiColors, Details, Fill, HorizontalGradient, TerminalColors,
        VerticalGradient, WarpTheme,
    },
};
use warp_core::ui::theme::ui_colors::UiColors;

const DARK_MODE_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x616161FF),
    AnsiColor::from_u32(0xFF8272FF),
    AnsiColor::from_u32(0xB4FA72FF),
    AnsiColor::from_u32(0xFEFDC2FF),
    AnsiColor::from_u32(0xA5D5FEFF),
    AnsiColor::from_u32(0xFF8FFDFF),
    AnsiColor::from_u32(0xD0D1FEFF),
    AnsiColor::from_u32(0xF1F1F1FF),
);
const DARK_MODE_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x8E8E8EFF),
    AnsiColor::from_u32(0xFFC4BDFF),
    AnsiColor::from_u32(0xD6FCB9FF),
    AnsiColor::from_u32(0xFEFDD5FF),
    AnsiColor::from_u32(0xC1E3FEFF),
    AnsiColor::from_u32(0xFFB1FEFF),
    AnsiColor::from_u32(0xE5E6FEFF),
    AnsiColor::from_u32(0xFEFFFFFF),
);

const LIGHT_MODE_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x212121FF),
    AnsiColor::from_u32(0xC30771FF),
    AnsiColor::from_u32(0x10A778FF),
    AnsiColor::from_u32(0xA89C14FF),
    AnsiColor::from_u32(0x008EC4FF),
    AnsiColor::from_u32(0x523C79FF),
    AnsiColor::from_u32(0x20A5BAFF),
    AnsiColor::from_u32(0xE0E0E0FF),
);
const LIGHT_MODE_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x212121FF),
    AnsiColor::from_u32(0xFB007AFF),
    AnsiColor::from_u32(0x5FD7AFFF),
    AnsiColor::from_u32(0xF3E430FF),
    AnsiColor::from_u32(0x20BBFCFF),
    AnsiColor::from_u32(0x6855DEFF),
    AnsiColor::from_u32(0x4FB8CCFF),
    AnsiColor::from_u32(0xF1F1F1FF),
);

const SOLARIZED_DARK_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x073642FF),
    AnsiColor::from_u32(0xDC322FFF),
    AnsiColor::from_u32(0x859900FF),
    AnsiColor::from_u32(0xB58900FF),
    AnsiColor::from_u32(0x268BD2FF),
    AnsiColor::from_u32(0xD33682FF),
    AnsiColor::from_u32(0x2AA198FF),
    AnsiColor::from_u32(0xEEE8D5FF),
);
const SOLARIZED_DARK_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x002B36FF),
    AnsiColor::from_u32(0xCB4B16FF),
    AnsiColor::from_u32(0x586E75FF),
    AnsiColor::from_u32(0x657B83FF),
    AnsiColor::from_u32(0x839496FF),
    AnsiColor::from_u32(0x6C71C4FF),
    AnsiColor::from_u32(0x93A1A1FF),
    AnsiColor::from_u32(0xFDF6E3FF),
);

const SOLARIZED_LIGHT_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x073642FF),
    AnsiColor::from_u32(0xDC322FFF),
    AnsiColor::from_u32(0x859900FF),
    AnsiColor::from_u32(0xB58900FF),
    AnsiColor::from_u32(0x268BD2FF),
    AnsiColor::from_u32(0xD33682FF),
    AnsiColor::from_u32(0x2AA198FF),
    AnsiColor::from_u32(0xEEE8D5FF),
);
const SOLARIZED_LIGHT_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x002B36FF),
    AnsiColor::from_u32(0xCB4B16FF),
    AnsiColor::from_u32(0x586E75FF),
    AnsiColor::from_u32(0x657B83FF),
    AnsiColor::from_u32(0x839496FF),
    AnsiColor::from_u32(0x6C71C4FF),
    AnsiColor::from_u32(0x93A1A1FF),
    AnsiColor::from_u32(0xFDF6E3FF),
);

const DRACULA_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x000000FF),
    AnsiColor::from_u32(0xFF5555FF),
    AnsiColor::from_u32(0x50FA7BFF),
    AnsiColor::from_u32(0xF1FA8CFF),
    AnsiColor::from_u32(0xBD93F9FF),
    AnsiColor::from_u32(0xFF79C6FF),
    AnsiColor::from_u32(0x8BE9FDFF),
    AnsiColor::from_u32(0xBBBBBBFF),
);
const DRACULA_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x555555FF),
    AnsiColor::from_u32(0xFF5555FF),
    AnsiColor::from_u32(0x50FA7BFF),
    AnsiColor::from_u32(0xF1FA8CFF),
    AnsiColor::from_u32(0xCAA9FAFF),
    AnsiColor::from_u32(0xFF79C6FF),
    AnsiColor::from_u32(0x8BE9FDFF),
    AnsiColor::from_u32(0xFFFFFFFF),
);

const GRUVBOX_DARK_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x282828FF),
    AnsiColor::from_u32(0xCC241DFF),
    AnsiColor::from_u32(0x98971AFF),
    AnsiColor::from_u32(0xD79921FF),
    AnsiColor::from_u32(0x458588FF),
    AnsiColor::from_u32(0xB16286FF),
    AnsiColor::from_u32(0x689D6AFF),
    AnsiColor::from_u32(0xA89984FF),
);
const GRUVBOX_DARK_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x928374FF),
    AnsiColor::from_u32(0xFB4934FF),
    AnsiColor::from_u32(0xB8BB26FF),
    AnsiColor::from_u32(0xFABD2FFF),
    AnsiColor::from_u32(0x83A598FF),
    AnsiColor::from_u32(0xD3869BFF),
    AnsiColor::from_u32(0x8EC07CFF),
    AnsiColor::from_u32(0xEBDBB2FF),
);

const GRUVBOX_LIGHT_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0xFBF1C7FF),
    AnsiColor::from_u32(0xCC241DFF),
    AnsiColor::from_u32(0x98971AFF),
    AnsiColor::from_u32(0xD79921FF),
    AnsiColor::from_u32(0x458588FF),
    AnsiColor::from_u32(0xB16286FF),
    AnsiColor::from_u32(0x689D6AFF),
    AnsiColor::from_u32(0x7C6F64FF),
);
const GRUVBOX_LIGHT_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x928374FF),
    AnsiColor::from_u32(0x9D0006FF),
    AnsiColor::from_u32(0x79740EFF),
    AnsiColor::from_u32(0xB57614FF),
    AnsiColor::from_u32(0x076678FF),
    AnsiColor::from_u32(0x8F3F71FF),
    AnsiColor::from_u32(0x427B58FF),
    AnsiColor::from_u32(0x3C3836FF),
);

const TOKYO_NIGHT_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x15161EFF),
    AnsiColor::from_u32(0xF7768EFF),
    AnsiColor::from_u32(0x9ECE6AFF),
    AnsiColor::from_u32(0xE0AF68FF),
    AnsiColor::from_u32(0x7AA2F7FF),
    AnsiColor::from_u32(0xBB9AF7FF),
    AnsiColor::from_u32(0x7DCFFFFF),
    AnsiColor::from_u32(0xA9B1D6FF),
);
const TOKYO_NIGHT_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x414868FF),
    AnsiColor::from_u32(0xF7768EFF),
    AnsiColor::from_u32(0x9ECE6AFF),
    AnsiColor::from_u32(0xE0AF68FF),
    AnsiColor::from_u32(0x7AA2F7FF),
    AnsiColor::from_u32(0xBB9AF7FF),
    AnsiColor::from_u32(0x7DCFFFFF),
    AnsiColor::from_u32(0xC0CAF5FF),
);

const ONE_DARK_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x282C34FF),
    AnsiColor::from_u32(0xE06C75FF),
    AnsiColor::from_u32(0x98C379FF),
    AnsiColor::from_u32(0xE5C07BFF),
    AnsiColor::from_u32(0x61AFEFFF),
    AnsiColor::from_u32(0xC678DDFF),
    AnsiColor::from_u32(0x56B6C2FF),
    AnsiColor::from_u32(0xABB2BFFF),
);
const ONE_DARK_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x636D83FF),
    AnsiColor::from_u32(0xEA858BFF),
    AnsiColor::from_u32(0xAAD581FF),
    AnsiColor::from_u32(0xFFD885FF),
    AnsiColor::from_u32(0x85C1FFFF),
    AnsiColor::from_u32(0xD398EBFF),
    AnsiColor::from_u32(0x6ED5DEFF),
    AnsiColor::from_u32(0xFAFAFAFF),
);

const ADEBERRY_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x121212FF),
    AnsiColor::from_u32(0xC76156FF),
    AnsiColor::from_u32(0x57C78AFF),
    AnsiColor::from_u32(0xC8A35AFF),
    AnsiColor::from_u32(0x5785C7FF),
    AnsiColor::from_u32(0xC756A9FF),
    AnsiColor::from_u32(0x57C7C3FF),
    AnsiColor::from_u32(0xEEEDEBFF),
);
const ADEBERRY_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x292929FF),
    AnsiColor::from_u32(0xD22D1EFF),
    AnsiColor::from_u32(0x1CA05AFF),
    AnsiColor::from_u32(0xE5A01AFF),
    AnsiColor::from_u32(0x1458B8FF),
    AnsiColor::from_u32(0xA43787FF),
    AnsiColor::from_u32(0x4D9989FF),
    AnsiColor::from_u32(0xFFFFFFFF),
);

const WEZTERM_CLASSIC_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x000000FF),
    AnsiColor::from_u32(0xCC5555FF),
    AnsiColor::from_u32(0x55CC55FF),
    AnsiColor::from_u32(0xCDCD55FF),
    AnsiColor::from_u32(0x5555CCFF),
    AnsiColor::from_u32(0xCC55CCFF),
    AnsiColor::from_u32(0x7ACACAFF),
    AnsiColor::from_u32(0xCCCCCCFF),
);
const WEZTERM_CLASSIC_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x555555FF),
    AnsiColor::from_u32(0xFF5555FF),
    AnsiColor::from_u32(0x55FF55FF),
    AnsiColor::from_u32(0xFFFF55FF),
    AnsiColor::from_u32(0x5555FFFF),
    AnsiColor::from_u32(0xFF55FFFF),
    AnsiColor::from_u32(0x55FFFFFF),
    AnsiColor::from_u32(0xFFFFFFFF),
);

// 16-color ANSI color source: vscode/extensions/theme-defaults/themes/2026-dark.json
const VSCODE_2026_DARK_NORMAL_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x000000FF),
    AnsiColor::from_u32(0xCD3131FF),
    AnsiColor::from_u32(0x0DBC79FF),
    AnsiColor::from_u32(0xE5E510FF),
    AnsiColor::from_u32(0x2472C8FF),
    AnsiColor::from_u32(0xBC3FBCFF),
    AnsiColor::from_u32(0x11A8CDFF),
    AnsiColor::from_u32(0xE5E5E5FF),
);
const VSCODE_2026_DARK_BRIGHT_COLORS: AnsiColors = AnsiColors::new(
    AnsiColor::from_u32(0x666666FF),
    AnsiColor::from_u32(0xF14C4CFF),
    AnsiColor::from_u32(0x23D18BFF),
    AnsiColor::from_u32(0xF5F543FF),
    AnsiColor::from_u32(0x3B8EEAFF),
    AnsiColor::from_u32(0xD670D6FF),
    AnsiColor::from_u32(0x29B8DBFF),
    AnsiColor::from_u32(0xE5E5E5FF),
);

/// Returns the 16-color ANSI terminal colors for the VS Code 2026 Dark theme.
pub(super) fn vscode_2026_dark_colors() -> TerminalColors {
    TerminalColors::new(VSCODE_2026_DARK_NORMAL_COLORS, VSCODE_2026_DARK_BRIGHT_COLORS)
}

/// VS Code 2026 Dark built-in theme; color source: vscode/extensions/theme-defaults/themes/2026-dark.json.
/// Includes complete UiColors coverage, mapping VS Code's editor/panel colors to Zaplex UI components.
pub(super) fn vscode_2026_dark() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x191A1BFF)),
        ColorU::from_u32(0xCCCCCCFF),
        Fill::Solid(ColorU::from_u32(0x3994BCFF)),
        Some(Fill::Solid(ColorU::from_u32(0xBFBFBFFF))),
        Some(Details::Darker),
        vscode_2026_dark_colors(),
        None,
        Some("VS Code 2026 Dark".to_string()),
        Some(UiColors {
            surface_1: Some(ColorU { r: 0x20, g: 0x21, b: 0x22, a: 255 }),
            surface_2: Some(ColorU { r: 0x24, g: 0x25, b: 0x26, a: 255 }),
            surface_3: Some(ColorU { r: 0x2A, g: 0x2B, b: 0x2C, a: 255 }),
            border: Some(ColorU { r: 0x33, g: 0x35, b: 0x36, a: 255 }),
            focus_border: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0xB3 }),
            split_pane_border: Some(ColorU { r: 0x2A, g: 0x2B, b: 0x2C, a: 255 }),
            main_text: Some(ColorU { r: 0xED, g: 0xED, b: 0xED, a: 255 }),
            sub_text: Some(ColorU { r: 0x8C, g: 0x8C, b: 0x8C, a: 255 }),
            hint_text: Some(ColorU { r: 0x55, g: 0x55, b: 0x55, a: 255 }),
            disabled_text: Some(ColorU { r: 0x55, g: 0x55, b: 0x55, a: 255 }),
            selection: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0x33 }),
            text_selection: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0x33 }),
            hover: Some(ColorU { r: 0xFF, g: 0xFF, b: 0xFF, a: 0x0D }),
            active: Some(ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 255 }),
            warning: Some(ColorU { r: 0xE5, g: 0xBA, b: 0x7D, a: 255 }),
            error: Some(ColorU { r: 0xF4, g: 0x87, b: 0x71, a: 255 }),
            success: Some(ColorU { r: 0x72, g: 0xC8, b: 0x92, a: 255 }),
            link: Some(ColorU { r: 0x48, g: 0xA0, b: 0xC7, a: 255 }),
        }),
    )
}

pub(super) fn light_mode_colors() -> TerminalColors {
    TerminalColors::new(LIGHT_MODE_NORMAL_COLORS, LIGHT_MODE_BRIGHT_COLORS)
}

pub(super) fn dark_mode_colors() -> TerminalColors {
    TerminalColors::new(DARK_MODE_NORMAL_COLORS, DARK_MODE_BRIGHT_COLORS)
}

pub(super) fn solarized_light_colors() -> TerminalColors {
    TerminalColors::new(SOLARIZED_LIGHT_NORMAL_COLORS, SOLARIZED_LIGHT_BRIGHT_COLORS)
}

pub(super) fn solarized_dark_colors() -> TerminalColors {
    TerminalColors::new(SOLARIZED_DARK_NORMAL_COLORS, SOLARIZED_DARK_BRIGHT_COLORS)
}

pub(super) fn dracula_colors() -> TerminalColors {
    TerminalColors::new(DRACULA_NORMAL_COLORS, DRACULA_BRIGHT_COLORS)
}

pub(super) fn gruvbox_dark_colors() -> TerminalColors {
    TerminalColors::new(GRUVBOX_DARK_NORMAL_COLORS, GRUVBOX_DARK_BRIGHT_COLORS)
}

pub(super) fn gruvbox_light_colors() -> TerminalColors {
    TerminalColors::new(GRUVBOX_LIGHT_NORMAL_COLORS, GRUVBOX_LIGHT_BRIGHT_COLORS)
}

pub(super) fn adeberry_colors() -> TerminalColors {
    TerminalColors::new(ADEBERRY_NORMAL_COLORS, ADEBERRY_BRIGHT_COLORS)
}

pub(super) fn tokyo_night_colors() -> TerminalColors {
    TerminalColors::new(TOKYO_NIGHT_NORMAL_COLORS, TOKYO_NIGHT_BRIGHT_COLORS)
}

pub(super) fn one_dark_colors() -> TerminalColors {
    TerminalColors::new(ONE_DARK_NORMAL_COLORS, ONE_DARK_BRIGHT_COLORS)
}

pub(super) fn wezterm_classic_colors() -> TerminalColors {
    TerminalColors::new(WEZTERM_CLASSIC_NORMAL_COLORS, WEZTERM_CLASSIC_BRIGHT_COLORS)
}

/// Default bundled themes
pub fn dark_theme() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x000000FF)),
        ColorU::from_u32(0xffffffff),
        Fill::Solid(ColorU::from_u32(0x19AAD8FF)),
        None,
        Some(Details::Darker),
        dark_mode_colors(),
        None,
        Some("Dark".to_string()),
        None,
    )
}

pub fn light_theme() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::white()),
        ColorU::new(17, 17, 17, OPAQUE),
        Fill::Solid(ColorU::from_u32(0x00c2ffff)),
        None,
        Some(Details::Lighter),
        light_mode_colors(),
        None,
        Some("Light".to_string()),
        None,
    )
}

pub(super) fn dracula() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x282A36FF)),
        ColorU::from_u32(0xF8F8F2FF),
        Fill::Solid(ColorU::from_u32(0xFF79C6FF)),
        None,
        Some(Details::Darker),
        dracula_colors(),
        None,
        Some("Dracula".to_string()),
        None,
    )
}

pub(super) fn solarized_light() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0xFDF6E3FF)),
        ColorU::from_u32(0x586E75FF),
        Fill::Solid(ColorU::from_u32(0x66B5A9FF)),
        None,
        Some(Details::Lighter),
        solarized_light_colors(),
        None,
        Some("Solarized Light".to_string()),
        None,
    )
}

pub(super) fn solarized_dark() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x002B36FF)),
        ColorU::from_u32(0xF8F8F2FF),
        Fill::Solid(ColorU::from_u32(0xCB4B16FF)),
        None,
        Some(Details::Darker),
        solarized_dark_colors(),
        None,
        Some("Solarized Dark".to_string()),
        None,
    )
}

pub(super) fn gruvbox_dark() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x282828FF)),
        ColorU::from_u32(0xEBDBB2FF),
        Fill::Solid(ColorU::from_u32(0xFC802DFF)),
        None,
        Some(Details::Darker),
        gruvbox_dark_colors(),
        None,
        Some("Gruvbox Dark".to_string()),
        None,
    )
}

pub(super) fn gruvbox_light() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0xFBF1C7FF)),
        ColorU::from_u32(0x3C3836FF),
        Fill::Solid(ColorU::from_u32(0xAD3B14FF)),
        None,
        Some(Details::Lighter),
        gruvbox_light_colors(),
        None,
        Some("Gruvbox Light".to_string()),
        None,
    )
}

/// Bundled gradient themes
pub(super) fn cyber_wave() -> WarpTheme {
    WarpTheme::new(
        Fill::VerticalGradient(VerticalGradient::new(
            ColorU::black().blend(&coloru_with_opacity(ColorU::from_u32(0x00C2FFFF), 20)),
            ColorU::black(),
        )),
        ColorU::white(),
        Fill::HorizontalGradient(HorizontalGradient::new(
            ColorU::from_u32(0x007972FF),
            ColorU::from_u32(0x7B008FFF),
        )),
        None,
        Some(Details::Darker),
        dark_mode_colors(),
        None,
        Some("Cyber Wave".to_string()),
        None,
    )
}

pub(super) fn willow_dream() -> WarpTheme {
    WarpTheme::new(
        Fill::VerticalGradient(VerticalGradient::new(
            ColorU::from_u32(0x206169FF),
            ColorU::from_u32(0x022F27FF),
        )),
        ColorU::white(),
        Fill::HorizontalGradient(HorizontalGradient::new(
            ColorU::from_u32(0xF9AEA8FF),
            ColorU::from_u32(0xDD6258FF),
        )),
        None,
        Some(Details::Darker),
        dark_mode_colors(),
        None,
        Some("Willow Dream".to_string()),
        None,
    )
}

pub(super) fn fancy_dracula() -> WarpTheme {
    WarpTheme::new(
        Fill::VerticalGradient(VerticalGradient::new(
            ColorU::from_u32(0x252630FF),
            ColorU::from_u32(0x3D3F4FFF),
        )),
        ColorU::white(),
        Fill::HorizontalGradient(HorizontalGradient::new(
            ColorU::from_u32(0xBCA1F6FF),
            ColorU::from_u32(0xA3E7FCFF),
        )),
        None,
        Some(Details::Darker),
        dracula_colors(),
        None,
        Some("Fancy Dracula".to_string()),
        None,
    )
}

/// Zaplex Dark - the default theme. Deep navy background with the blue->purple
/// accent from the zaplex splash screen; uses the well-tuned Tokyo Night ANSI
/// palette for readable terminal colors. Solid fill, no background image.
pub(super) fn zaplex_dark() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x0E1320FF)),
        ColorU::from_u32(0xE6EAF3FF),
        Fill::Solid(ColorU::from_u32(0x6C82F2FF)),
        None,
        Some(Details::Darker),
        tokyo_night_colors(),
        None,
        Some("Zaplex Dark".to_string()),
        None,
    )
}

pub(super) fn tokyo_night() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x1A1B26FF)),
        ColorU::from_u32(0xC0CAF5FF),
        Fill::Solid(ColorU::from_u32(0x7AA2F7FF)),
        None,
        Some(Details::Darker),
        tokyo_night_colors(),
        None,
        Some("Tokyo Night".to_string()),
        None,
    )
}

pub(super) fn one_dark() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x282C34FF)),
        ColorU::from_u32(0xABB2BFFF),
        Fill::Solid(ColorU::from_u32(0x74ADE8FF)),
        None,
        Some(Details::Darker),
        one_dark_colors(),
        None,
        Some("One Dark".to_string()),
        None,
    )
}

pub(super) fn adeberry() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x1D2022FF)),
        ColorU::from_u32(0xE4EEF5FF),
        Fill::Solid(ColorU::from_u32(0x6C96B4FF)),
        None,
        Some(Details::Darker),
        adeberry_colors(),
        None,
        Some("Adeberry".to_string()),
        None,
    )
}

// 16-color ANSI aligned with background/foreground WezTerm default colors; accent color
// #52AD70 is chosen by this project, not a WezTerm standard color.
pub(super) fn wezterm_classic() -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x000000FF)),
        ColorU::from_u32(0xE0E0E0FF),
        Fill::Solid(ColorU::from_u32(0x52AD70FF)),
        None,
        Some(Details::Darker),
        wezterm_classic_colors(),
        None,
        Some("WezTerm Classic".to_string()),
        None,
    )
}
