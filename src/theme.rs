//! Colours from herdr's theme. herdr has no theme API, so codeMorph reads
//! `[theme]` from herdr's `config.toml` and resolves it the way herdr does:
//! built-in palette by name, then `[theme.custom]`, then the dark or light
//! mode overrides. The palettes are ported from herdr's `src/app/state.rs`
//! (Apache-2.0, herdr v0.9.0).

use std::path::{Path, PathBuf};

use ratatui::style::Color;
use serde::Deserialize;

use crate::canvas::{Band, Tone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub accent: Color,
    pub panel_bg: Color,
    pub sidebar_bg: Color,
    pub active_row_bg: Color,
    pub selection_bg: Color,
    pub surface0: Color,
    pub surface1: Color,
    pub surface_dim: Color,
    pub overlay0: Color,
    pub overlay1: Color,
    pub text: Color,
    pub subtext0: Color,
    pub mauve: Color,
    pub green: Color,
    pub yellow: Color,
    pub red: Color,
    pub blue: Color,
    pub teal: Color,
    pub peach: Color,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[allow(clippy::too_many_arguments)]
const fn pal(
    accent: Color,
    panel_bg: Color,
    active_row_bg: Color,
    selection_bg: Color,
    surface0: Color,
    surface1: Color,
    surface_dim: Color,
    overlay0: Color,
    overlay1: Color,
    text: Color,
    subtext0: Color,
    mauve: Color,
    green: Color,
    yellow: Color,
    red: Color,
    blue: Color,
    teal: Color,
    peach: Color,
) -> Palette {
    Palette {
        accent,
        panel_bg,
        sidebar_bg: Color::Reset,
        active_row_bg,
        selection_bg,
        surface0,
        surface1,
        surface_dim,
        overlay0,
        overlay1,
        text,
        subtext0,
        mauve,
        green,
        yellow,
        red,
        blue,
        teal,
        peach,
    }
}

impl Palette {
    pub fn catppuccin() -> Self {
        pal(
            rgb(137, 180, 250),
            rgb(24, 24, 37),
            rgb(30, 30, 46),
            rgb(49, 50, 68),
            rgb(49, 50, 68),
            rgb(69, 71, 90),
            rgb(30, 30, 46),
            rgb(108, 112, 134),
            rgb(127, 132, 156),
            rgb(205, 214, 244),
            rgb(166, 173, 200),
            rgb(203, 166, 247),
            rgb(166, 227, 161),
            rgb(249, 226, 175),
            rgb(243, 139, 168),
            rgb(137, 180, 250),
            rgb(148, 226, 213),
            rgb(250, 179, 135),
        )
    }
    pub fn catppuccin_latte() -> Self {
        pal(
            rgb(30, 102, 245),
            rgb(239, 241, 245),
            rgb(230, 233, 239),
            rgb(189, 208, 245),
            rgb(204, 208, 218),
            rgb(188, 192, 204),
            rgb(230, 233, 239),
            rgb(156, 160, 176),
            rgb(140, 143, 161),
            rgb(76, 79, 105),
            rgb(108, 111, 133),
            rgb(136, 57, 239),
            rgb(64, 160, 43),
            rgb(223, 142, 29),
            rgb(210, 15, 57),
            rgb(30, 102, 245),
            rgb(23, 146, 153),
            rgb(254, 100, 11),
        )
    }
    pub fn terminal() -> Self {
        Palette {
            accent: Color::Blue,
            panel_bg: Color::Reset,
            sidebar_bg: Color::Reset,
            active_row_bg: Color::DarkGray,
            selection_bg: Color::Reset,
            surface0: Color::Reset,
            surface1: Color::DarkGray,
            surface_dim: Color::DarkGray,
            overlay0: Color::Gray,
            overlay1: Color::White,
            text: Color::Reset,
            subtext0: Color::Gray,
            mauve: Color::Gray,
            green: Color::Green,
            yellow: Color::Yellow,
            red: Color::LightRed,
            blue: Color::Blue,
            teal: Color::Cyan,
            peach: Color::Yellow,
        }
    }
    pub fn tokyo_night() -> Self {
        pal(
            rgb(122, 162, 247),
            rgb(26, 27, 38),
            rgb(35, 38, 54),
            rgb(45, 54, 80),
            rgb(36, 40, 59),
            rgb(65, 72, 104),
            rgb(26, 27, 38),
            rgb(86, 95, 137),
            rgb(105, 113, 150),
            rgb(192, 202, 245),
            rgb(169, 177, 214),
            rgb(187, 154, 247),
            rgb(158, 206, 106),
            rgb(224, 175, 104),
            rgb(247, 118, 142),
            rgb(122, 162, 247),
            rgb(125, 207, 255),
            rgb(255, 158, 100),
        )
    }
    pub fn tokyo_night_day() -> Self {
        pal(
            rgb(46, 125, 233),
            rgb(225, 226, 231),
            rgb(210, 211, 218),
            rgb(182, 202, 231),
            rgb(196, 200, 218),
            rgb(168, 174, 203),
            rgb(210, 211, 218),
            rgb(137, 144, 179),
            rgb(104, 112, 154),
            rgb(55, 96, 191),
            rgb(97, 114, 176),
            rgb(120, 71, 189),
            rgb(88, 117, 57),
            rgb(140, 108, 62),
            rgb(245, 42, 101),
            rgb(46, 125, 233),
            rgb(17, 140, 116),
            rgb(177, 92, 0),
        )
    }
    pub fn dracula() -> Self {
        pal(
            rgb(189, 147, 249),
            rgb(40, 42, 54),
            rgb(55, 60, 82),
            rgb(70, 63, 93),
            rgb(68, 71, 90),
            rgb(98, 114, 164),
            rgb(40, 42, 54),
            rgb(98, 114, 164),
            rgb(130, 140, 180),
            rgb(248, 248, 242),
            rgb(210, 210, 220),
            rgb(255, 121, 198),
            rgb(80, 250, 123),
            rgb(241, 250, 140),
            rgb(255, 85, 85),
            rgb(139, 233, 253),
            rgb(139, 233, 253),
            rgb(255, 184, 108),
        )
    }
    pub fn nord() -> Self {
        pal(
            rgb(136, 192, 208),
            rgb(46, 52, 64),
            rgb(67, 76, 94),
            rgb(64, 80, 93),
            rgb(59, 66, 82),
            rgb(67, 76, 94),
            rgb(46, 52, 64),
            rgb(76, 86, 106),
            rgb(100, 110, 130),
            rgb(236, 239, 244),
            rgb(216, 222, 233),
            rgb(180, 142, 173),
            rgb(163, 190, 140),
            rgb(235, 203, 139),
            rgb(191, 97, 106),
            rgb(129, 161, 193),
            rgb(143, 188, 187),
            rgb(208, 135, 112),
        )
    }
    pub fn gruvbox() -> Self {
        pal(
            rgb(215, 153, 33),
            rgb(40, 40, 40),
            rgb(50, 49, 48),
            rgb(75, 63, 39),
            rgb(60, 56, 54),
            rgb(80, 73, 69),
            rgb(40, 40, 40),
            rgb(146, 131, 116),
            rgb(168, 153, 132),
            rgb(235, 219, 178),
            rgb(213, 196, 161),
            rgb(211, 134, 155),
            rgb(184, 187, 38),
            rgb(250, 189, 47),
            rgb(251, 73, 52),
            rgb(131, 165, 152),
            rgb(142, 192, 124),
            rgb(254, 128, 25),
        )
    }
    pub fn gruvbox_light() -> Self {
        pal(
            rgb(7, 102, 120),
            rgb(251, 241, 199),
            rgb(242, 229, 188),
            rgb(235, 219, 178),
            rgb(235, 219, 178),
            rgb(213, 196, 161),
            rgb(242, 229, 188),
            rgb(146, 131, 116),
            rgb(124, 111, 100),
            rgb(60, 56, 54),
            rgb(80, 73, 69),
            rgb(143, 63, 113),
            rgb(121, 116, 14),
            rgb(181, 118, 20),
            rgb(157, 0, 6),
            rgb(7, 102, 120),
            rgb(66, 123, 88),
            rgb(175, 58, 3),
        )
    }
    pub fn one_dark() -> Self {
        pal(
            rgb(97, 175, 239),
            rgb(40, 44, 52),
            rgb(49, 54, 64),
            rgb(51, 70, 89),
            rgb(44, 49, 58),
            rgb(62, 68, 81),
            rgb(40, 44, 52),
            rgb(92, 99, 112),
            rgb(115, 122, 135),
            rgb(171, 178, 191),
            rgb(150, 156, 168),
            rgb(198, 120, 221),
            rgb(152, 195, 121),
            rgb(229, 192, 123),
            rgb(224, 108, 117),
            rgb(97, 175, 239),
            rgb(86, 182, 194),
            rgb(209, 154, 102),
        )
    }
    pub fn one_light() -> Self {
        pal(
            rgb(64, 120, 242),
            rgb(250, 250, 250),
            rgb(216, 219, 226),
            rgb(205, 219, 248),
            rgb(240, 240, 241),
            rgb(229, 229, 230),
            rgb(245, 245, 246),
            rgb(160, 161, 167),
            rgb(104, 107, 119),
            rgb(56, 58, 66),
            rgb(104, 107, 119),
            rgb(166, 38, 164),
            rgb(80, 161, 79),
            rgb(193, 132, 1),
            rgb(228, 86, 73),
            rgb(64, 120, 242),
            rgb(1, 132, 188),
            rgb(152, 104, 1),
        )
    }
    pub fn solarized() -> Self {
        pal(
            rgb(38, 139, 210),
            rgb(0, 43, 54),
            rgb(22, 75, 87),
            rgb(8, 62, 85),
            rgb(7, 54, 66),
            rgb(88, 110, 117),
            rgb(0, 43, 54),
            rgb(88, 110, 117),
            rgb(101, 123, 131),
            rgb(147, 161, 161),
            rgb(131, 148, 150),
            rgb(211, 54, 130),
            rgb(133, 153, 0),
            rgb(181, 137, 0),
            rgb(220, 50, 47),
            rgb(38, 139, 210),
            rgb(42, 161, 152),
            rgb(203, 75, 22),
        )
    }
    pub fn solarized_light() -> Self {
        pal(
            rgb(38, 139, 210),
            rgb(253, 246, 227),
            rgb(238, 232, 213),
            rgb(201, 220, 223),
            rgb(238, 232, 213),
            rgb(147, 161, 161),
            rgb(238, 232, 213),
            rgb(147, 161, 161),
            rgb(88, 110, 117),
            rgb(101, 123, 131),
            rgb(131, 148, 150),
            rgb(211, 54, 130),
            rgb(133, 153, 0),
            rgb(181, 137, 0),
            rgb(220, 50, 47),
            rgb(38, 139, 210),
            rgb(42, 161, 152),
            rgb(203, 75, 22),
        )
    }
    pub fn kanagawa() -> Self {
        pal(
            rgb(126, 156, 216),
            rgb(31, 31, 40),
            rgb(54, 54, 70),
            rgb(50, 56, 75),
            rgb(42, 42, 55),
            rgb(54, 54, 70),
            rgb(31, 31, 40),
            rgb(114, 113, 105),
            rgb(135, 134, 125),
            rgb(220, 215, 186),
            rgb(200, 195, 170),
            rgb(149, 127, 184),
            rgb(118, 148, 106),
            rgb(192, 163, 110),
            rgb(195, 64, 67),
            rgb(126, 156, 216),
            rgb(127, 180, 202),
            rgb(255, 160, 102),
        )
    }
    pub fn kanagawa_lotus() -> Self {
        pal(
            rgb(77, 105, 155),
            rgb(242, 236, 188),
            rgb(213, 206, 163),
            rgb(220, 213, 172),
            rgb(220, 213, 172),
            rgb(201, 203, 209),
            rgb(213, 206, 163),
            rgb(160, 156, 172),
            rgb(138, 137, 128),
            rgb(84, 84, 100),
            rgb(67, 67, 108),
            rgb(98, 76, 131),
            rgb(111, 137, 78),
            rgb(119, 113, 63),
            rgb(200, 64, 83),
            rgb(77, 105, 155),
            rgb(78, 140, 162),
            rgb(204, 109, 0),
        )
    }
    pub fn rose_pine() -> Self {
        pal(
            rgb(196, 167, 231),
            rgb(25, 23, 36),
            rgb(38, 35, 58),
            rgb(59, 52, 75),
            rgb(31, 29, 46),
            rgb(38, 35, 58),
            rgb(38, 35, 58),
            rgb(110, 106, 134),
            rgb(144, 140, 170),
            rgb(224, 222, 244),
            rgb(200, 197, 220),
            rgb(196, 167, 231),
            rgb(49, 116, 143),
            rgb(246, 193, 119),
            rgb(235, 111, 146),
            rgb(49, 116, 143),
            rgb(156, 207, 216),
            rgb(234, 154, 151),
        )
    }
    pub fn rose_pine_dawn() -> Self {
        pal(
            rgb(144, 122, 169),
            rgb(250, 244, 237),
            rgb(227, 217, 207),
            rgb(242, 233, 225),
            rgb(242, 233, 225),
            rgb(255, 250, 243),
            rgb(242, 233, 225),
            rgb(152, 147, 165),
            rgb(121, 117, 147),
            rgb(70, 66, 97),
            rgb(121, 117, 147),
            rgb(144, 122, 169),
            rgb(40, 105, 131),
            rgb(234, 157, 52),
            rgb(180, 99, 122),
            rgb(40, 105, 131),
            rgb(86, 148, 159),
            rgb(215, 130, 126),
        )
    }
    pub fn vesper() -> Self {
        pal(
            rgb(255, 199, 153),
            rgb(26, 26, 26),
            rgb(16, 16, 16),
            rgb(35, 35, 35),
            rgb(35, 35, 35),
            rgb(40, 40, 40),
            rgb(16, 16, 16),
            rgb(92, 92, 92),
            rgb(126, 126, 126),
            rgb(255, 255, 255),
            rgb(160, 160, 160),
            rgb(255, 209, 168),
            rgb(153, 255, 228),
            rgb(255, 199, 153),
            rgb(255, 128, 128),
            rgb(176, 176, 176),
            rgb(102, 221, 204),
            rgb(255, 199, 153),
        )
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match canonical_theme_name(name)? {
            "catppuccin" => Self::catppuccin(),
            "catppuccin-latte" => Self::catppuccin_latte(),
            "terminal" => Self::terminal(),
            "tokyo-night" => Self::tokyo_night(),
            "tokyo-night-day" => Self::tokyo_night_day(),
            "dracula" => Self::dracula(),
            "nord" => Self::nord(),
            "gruvbox" => Self::gruvbox(),
            "gruvbox-light" => Self::gruvbox_light(),
            "one-dark" => Self::one_dark(),
            "one-light" => Self::one_light(),
            "solarized" => Self::solarized(),
            "solarized-light" => Self::solarized_light(),
            "kanagawa" => Self::kanagawa(),
            "kanagawa-lotus" => Self::kanagawa_lotus(),
            "rose-pine" => Self::rose_pine(),
            "rose-pine-dawn" => Self::rose_pine_dawn(),
            "vesper" => Self::vesper(),
            _ => return None,
        })
    }

    fn apply(&mut self, c: &ColorOverrides) {
        let set = |slot: &mut Color, v: &Option<String>| {
            if let Some(v) = v {
                *slot = parse_color(v);
            }
        };
        set(&mut self.accent, &c.accent);
        set(&mut self.panel_bg, &c.panel_bg);
        set(&mut self.sidebar_bg, &c.sidebar_bg);
        set(&mut self.active_row_bg, &c.active_row_bg);
        set(&mut self.selection_bg, &c.selection_bg);
        set(&mut self.surface0, &c.surface0);
        set(&mut self.surface1, &c.surface1);
        set(&mut self.surface_dim, &c.surface_dim);
        set(&mut self.overlay0, &c.overlay0);
        set(&mut self.overlay1, &c.overlay1);
        set(&mut self.text, &c.text);
        set(&mut self.subtext0, &c.subtext0);
        set(&mut self.mauve, &c.mauve);
        set(&mut self.green, &c.green);
        set(&mut self.yellow, &c.yellow);
        set(&mut self.red, &c.red);
        set(&mut self.blue, &c.blue);
        set(&mut self.teal, &c.teal);
        set(&mut self.peach, &c.peach);
    }
}

/// herdr's theme name aliases (src/config/theme.rs).
pub fn canonical_theme_name(name: &str) -> Option<&'static str> {
    match name.to_lowercase().replace([' ', '_'], "-").as_str() {
        "catppuccin" | "catppuccin-mocha" => Some("catppuccin"),
        "catppuccin-latte" | "latte" | "light" => Some("catppuccin-latte"),
        "terminal" => Some("terminal"),
        "tokyo-night" | "tokyonight" => Some("tokyo-night"),
        "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => Some("tokyo-night-day"),
        "dracula" => Some("dracula"),
        "nord" => Some("nord"),
        "gruvbox" | "gruvbox-dark" => Some("gruvbox"),
        "gruvbox-light" => Some("gruvbox-light"),
        "one-dark" | "onedark" => Some("one-dark"),
        "one-light" | "onelight" => Some("one-light"),
        "solarized" | "solarized-dark" => Some("solarized"),
        "solarized-light" => Some("solarized-light"),
        "kanagawa" => Some("kanagawa"),
        "kanagawa-lotus" | "lotus" => Some("kanagawa-lotus"),
        "rose-pine" | "rosepine" => Some("rose-pine"),
        "rose-pine-dawn" | "rosepine-dawn" | "dawn" => Some("rose-pine-dawn"),
        "vesper" => Some("vesper"),
        _ => None,
    }
}

/// herdr's `parse_color`: hex, #rgb, rgb(r,g,b), names, reset aliases.
pub fn parse_color(s: &str) -> Color {
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "reset" | "default" | "none" | "transparent" => return Color::Reset,
        _ => {}
    }
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return Color::Rgb(r, g, b);
            }
        } else if hex.len() == 3 {
            let v: Vec<u8> = hex
                .chars()
                .filter_map(|c| u8::from_str_radix(&c.to_string(), 16).ok())
                .collect();
            if v.len() == 3 {
                return Color::Rgb(v[0] * 17, v[1] * 17, v[2] * 17);
            }
        }
    }
    if let Some(inner) = s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                parts[0].trim().parse::<u8>(),
                parts[1].trim().parse::<u8>(),
                parts[2].trim().parse::<u8>(),
            ) {
                return Color::Rgb(r, g, b);
            }
        }
    }
    match s.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        _ => Color::Cyan,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ColorOverrides {
    pub accent: Option<String>,
    pub panel_bg: Option<String>,
    pub sidebar_bg: Option<String>,
    pub active_row_bg: Option<String>,
    pub selection_bg: Option<String>,
    pub surface0: Option<String>,
    pub surface1: Option<String>,
    pub surface_dim: Option<String>,
    pub overlay0: Option<String>,
    pub overlay1: Option<String>,
    pub text: Option<String>,
    pub subtext0: Option<String>,
    pub mauve: Option<String>,
    pub green: Option<String>,
    pub yellow: Option<String>,
    pub red: Option<String>,
    pub blue: Option<String>,
    pub teal: Option<String>,
    pub peach: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CustomColors {
    #[serde(flatten)]
    pub base: ColorOverrides,
    pub light: Option<ColorOverrides>,
    pub dark: Option<ColorOverrides>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub name: Option<String>,
    pub auto_switch: bool,
    pub dark_name: Option<String>,
    pub light_name: Option<String>,
    pub custom: Option<CustomColors>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ConfigFile {
    theme: ThemeConfig,
}

/// herdr's config file: `HERDR_CONFIG_PATH`, else `$XDG_CONFIG_HOME/herdr`
/// or `~/.config/herdr`, `config.toml`.
pub fn herdr_config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HERDR_CONFIG_PATH").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(x).join("herdr").join("config.toml");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".config").join("herdr").join("config.toml")
}

pub fn parse_theme_config(toml_text: &str) -> ThemeConfig {
    toml::from_str::<ConfigFile>(toml_text)
        .map(|c| c.theme)
        .unwrap_or_default()
}

/// Resolve the palette herdr would use for a config. With `auto_switch`
/// codeMorph cannot see the host appearance, so it assumes dark.
pub fn resolve_palette(cfg: &ThemeConfig) -> (Palette, String) {
    let (name, mode) = if cfg.auto_switch {
        (
            cfg.dark_name.clone().unwrap_or_else(|| "catppuccin".into()),
            Some(true),
        )
    } else {
        (
            cfg.name.clone().unwrap_or_else(|| "catppuccin".into()),
            None,
        )
    };
    let canonical = canonical_theme_name(&name).unwrap_or("catppuccin");
    let mut palette = Palette::from_name(canonical).unwrap_or_else(Palette::catppuccin);
    if let Some(custom) = &cfg.custom {
        palette.apply(&custom.base);
        if mode == Some(true) {
            if let Some(dark) = &custom.dark {
                palette.apply(dark);
            }
        }
    }
    (palette, canonical.to_string())
}

/// codeMorph's design tokens, resolved to colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    /// Left column (herdr sidebar_bg; the panel colour when that is reset).
    pub rail: Color,
    /// Code and diagram body: the base theme's panel colour, which is what
    /// the terminal draws agent panes on (#282a36 for dracula).
    pub body: Color,
    pub active_row: Color,
    pub selection: Color,
    pub text: Color,
    pub text2: Color,
    pub muted: Color,
    pub rule: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub branch: Color,
    pub add: Color,
    pub del: Color,
    pub modified: Color,
    pub renamed: Color,
    pub conflict: Color,
    pub add_bg: Color,
    pub del_bg: Color,
    pub add_word: Color,
    pub del_word: Color,
}

fn blend(fg: Color, bg: Color, alpha: f32) -> Color {
    match (fg, bg) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let mix = |a: u8, b: u8| -> u8 {
                (a as f32 * alpha + b as f32 * (1.0 - alpha)).round() as u8
            };
            Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
        }
        _ => bg,
    }
}

impl Theme {
    pub fn from_palette(p: &Palette, base: &Palette, name: &str) -> Theme {
        let body = if base.panel_bg == Color::Reset {
            Color::Reset
        } else {
            base.panel_bg
        };
        let rail = if p.sidebar_bg == Color::Reset {
            p.panel_bg
        } else {
            p.sidebar_bg
        };
        let dark_text =
            matches!(p.text, Color::Rgb(r, g, b) if (r as u32 + g as u32 + b as u32) < 384);
        let on_accent = if dark_text {
            Color::Rgb(255, 255, 255)
        } else {
            Color::Rgb(0, 0, 0)
        };
        let (add_bg, del_bg, add_word, del_word) = match body {
            Color::Rgb(..) => (
                blend(p.green, body, 0.15),
                blend(p.red, body, 0.15),
                blend(p.green, body, 0.30),
                blend(p.red, body, 0.30),
            ),
            _ => (Color::Reset, Color::Reset, Color::DarkGray, Color::DarkGray),
        };
        let name: &'static str = match canonical_theme_name(name) {
            Some(n) => n,
            None => "catppuccin",
        };
        Theme {
            name,
            rail,
            body,
            active_row: p.active_row_bg,
            selection: p.selection_bg,
            text: p.text,
            text2: p.subtext0,
            muted: p.overlay1,
            rule: p.overlay0,
            accent: p.accent,
            on_accent,
            branch: p.mauve,
            add: p.green,
            del: p.red,
            modified: p.yellow,
            renamed: p.blue,
            conflict: p.peach,
            add_bg,
            del_bg,
            add_word,
            del_word,
        }
    }

    pub fn from_config_text(text: &str) -> Theme {
        let cfg = parse_theme_config(text);
        let (palette, name) = resolve_palette(&cfg);
        let base = Palette::from_name(&name).unwrap_or_else(Palette::catppuccin);
        Theme::from_palette(&palette, &base, &name)
    }

    /// Load herdr's theme; the default catppuccin theme if there is no config.
    pub fn load() -> Theme {
        Self::load_from(&herdr_config_path())
    }

    pub fn load_from(path: &Path) -> Theme {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Theme::from_config_text(&text)
    }

    pub fn tone(&self, tone: Tone) -> Color {
        match tone {
            Tone::Rule => self.rule,
            Tone::Text => self.text,
            Tone::Dim => self.text2,
            Tone::Muted => self.muted,
            Tone::Add => self.add,
            Tone::Mod => self.modified,
            Tone::Del => self.del,
            Tone::Branch => self.branch,
            Tone::Blue => self.renamed,
            Tone::Accent => self.accent,
            Tone::Warn => self.conflict,
        }
    }

    pub fn band(&self, band: Band) -> Color {
        match band {
            Band::Active => self.active_row,
            Band::Selection => self.selection,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_CONFIG: &str = r##"
onboarding = false

[theme]
name = "dracula"
auto_switch = false

[theme.custom]
panel_bg = "#000000"
sidebar_bg = "#000000"
surface0 = "#000000"
surface1 = "#0a0a0a"
surface_dim = "#000000"
active_row_bg = "#141414"
selection_bg = "#1a1a1a"

[keys]
prefix = "ctrl+b"
"##;

    #[test]
    fn dracula_with_custom_black_surfaces() {
        let t = Theme::from_config_text(USER_CONFIG);
        assert_eq!(t.name, "dracula");
        assert_eq!(t.rail, Color::Rgb(0, 0, 0));
        assert_eq!(t.body, Color::Rgb(0x28, 0x2a, 0x36));
        assert_eq!(t.selection, Color::Rgb(0x1a, 0x1a, 0x1a));
        assert_eq!(t.active_row, Color::Rgb(0x14, 0x14, 0x14));
        assert_eq!(t.accent, Color::Rgb(0xbd, 0x93, 0xf9));
        assert_eq!(t.branch, Color::Rgb(0xff, 0x79, 0xc6));
        assert_eq!(t.muted, Color::Rgb(0x82, 0x8c, 0xb4));
        assert_eq!(t.add, Color::Rgb(0x50, 0xfa, 0x7b));
        // green and red at 15% over #282a36, as in the Loupe design tokens
        assert_eq!(t.add_bg, Color::Rgb(0x2e, 0x49, 0x40));
        assert_eq!(t.del_bg, Color::Rgb(0x48, 0x30, 0x3b));
        assert_eq!(t.on_accent, Color::Rgb(0, 0, 0));
    }

    #[test]
    fn defaults_and_aliases() {
        let t = Theme::from_config_text("");
        assert_eq!(t.name, "catppuccin");
        let t = Theme::from_config_text("[theme]\nname = \"Tokyo Night\"\n");
        assert_eq!(t.name, "tokyo-night");
        let t = Theme::from_config_text("[theme]\nname = \"nope\"\n");
        assert_eq!(t.name, "catppuccin");
        // Broken TOML falls back instead of failing.
        let t = Theme::from_config_text("[theme\nname=");
        assert_eq!(t.name, "catppuccin");
    }

    #[test]
    fn overrides_and_modes() {
        let t = Theme::from_config_text(
            "[theme]\nname = \"nord\"\nauto_switch = true\ndark_name = \"gruvbox\"\n[theme.custom]\naccent = \"rgb(1, 2, 3)\"\n[theme.custom.dark]\nred = \"#f00\"\n",
        );
        assert_eq!(t.name, "gruvbox");
        assert_eq!(t.accent, Color::Rgb(1, 2, 3));
        assert_eq!(t.del, Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("transparent"), Color::Reset);
        assert_eq!(parse_color("#abc"), Color::Rgb(0xaa, 0xbb, 0xcc));
    }

    #[test]
    fn every_builtin_resolves() {
        for name in [
            "catppuccin",
            "catppuccin-latte",
            "terminal",
            "tokyo-night",
            "tokyo-night-day",
            "dracula",
            "nord",
            "gruvbox",
            "gruvbox-light",
            "one-dark",
            "one-light",
            "solarized",
            "solarized-light",
            "kanagawa",
            "kanagawa-lotus",
            "rose-pine",
            "rose-pine-dawn",
            "vesper",
        ] {
            assert!(Palette::from_name(name).is_some(), "{name}");
        }
        let t = Theme::from_config_text("[theme]\nname = \"terminal\"\n");
        assert_eq!(t.body, Color::Reset);
    }
}
