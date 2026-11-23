use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb, rgba};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePreset {
    #[default]
    VibesCrimson,
    PlasmaNative,
    KanagawaDragon,
    BreezeDark,
    CatppuccinMocha,
}

impl ThemePreset {
    pub const ALL: [Self; 5] = [
        Self::VibesCrimson,
        Self::PlasmaNative,
        Self::KanagawaDragon,
        Self::BreezeDark,
        Self::CatppuccinMocha,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::VibesCrimson => "Vibes Crimson",
            Self::PlasmaNative => "KDE Plasma (kdeglobals)",
            Self::KanagawaDragon => "Kanagawa Dragon",
            Self::BreezeDark => "KDE Breeze Dark",
            Self::CatppuccinMocha => "Catppuccin Mocha",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub is_dark: bool,
    pub bg_deep: u32,
    pub bg_main: u32,
    pub bg_sidebar: u32,
    pub bg_panel: u32,
    pub bg_btn: u32,
    pub bg_btn_hover: u32,
    pub bg_btn_active: u32,
    pub border: u32,
    pub border_subtle: u32,
    pub fg: u32,
    pub fg_muted: u32,
    pub fg_subtle: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_active: u32,
    pub green: u32,
    pub yellow: u32,
    pub red: u32,
    pub orange: u32,
    pub aqua: u32,
    pub violet: u32,
}

impl Palette {
    pub fn vibes_crimson() -> Self {
        Self {
            is_dark: true,
            bg_deep: 0x120a0d,
            bg_main: 0x1a1013,
            bg_sidebar: 0x211519,
            bg_panel: 0x2c1b21,
            bg_btn: 0x3c262d,
            bg_btn_hover: 0x4c313a,
            bg_btn_active: 0x311f25,
            border: 0x472e37,
            border_subtle: 0x301f25,
            fg: 0xf1e5e8,
            fg_muted: 0x876e76,
            fg_subtle: 0xb89fa7,
            accent: 0xe04f54,
            accent_hover: 0xec686c,
            accent_active: 0xc73b40,
            green: 0x76c289,
            yellow: 0xe5be78,
            red: 0xf06565,
            orange: 0xe28d64,
            aqua: 0x78b8b6,
            violet: 0xba8cc9,
        }
    }

    pub fn kanagawa_dragon() -> Self {
        Self {
            is_dark: true,
            bg_deep: 0x0d0c0c,
            bg_main: 0x181616,
            bg_sidebar: 0x1d1c19,
            bg_panel: 0x282727,
            bg_btn: 0x393836,
            bg_btn_hover: 0x464441,
            bg_btn_active: 0x31302e,
            border: 0x393836,
            border_subtle: 0x2a2826,
            fg: 0xdcd7ba,
            fg_muted: 0x727169,
            fg_subtle: 0x9e9b93,
            accent: 0x8ba4b0,
            accent_hover: 0x9fb8c4,
            accent_active: 0x7a929e,
            green: 0x87a987,
            yellow: 0xc4b28a,
            red: 0xc4746e,
            orange: 0xb6927b,
            aqua: 0x8ea4a2,
            violet: 0x8992a7,
        }
    }

    pub fn breeze_dark() -> Self {
        Self {
            is_dark: true,
            bg_deep: 0x141618,
            bg_main: 0x1b1e20,
            bg_sidebar: 0x202326,
            bg_panel: 0x232629,
            bg_btn: 0x31363b,
            bg_btn_hover: 0x3d4349,
            bg_btn_active: 0x292d31,
            border: 0x3b4045,
            border_subtle: 0x2c3034,
            fg: 0xeff0f1,
            fg_muted: 0x7f8c8d,
            fg_subtle: 0xbdc3c7,
            accent: 0x3daee9,
            accent_hover: 0x5bc0f4,
            accent_active: 0x2980b9,
            green: 0x2ecc71,
            yellow: 0xf67400,
            red: 0xda4453,
            orange: 0xf47750,
            aqua: 0x1abc9c,
            violet: 0x9b59b6,
        }
    }

    pub fn catppuccin_mocha() -> Self {
        Self {
            is_dark: true,
            bg_deep: 0x11111b,
            bg_main: 0x1e1e2e,
            bg_sidebar: 0x181825,
            bg_panel: 0x262637,
            bg_btn: 0x313244,
            bg_btn_hover: 0x45475a,
            bg_btn_active: 0x272836,
            border: 0x45475a,
            border_subtle: 0x2b2b3d,
            fg: 0xcdd6f4,
            fg_muted: 0x6c7086,
            fg_subtle: 0xa6adc8,
            accent: 0x89b4fa,
            accent_hover: 0xa6c8ff,
            accent_active: 0x74c7ec,
            green: 0xa6e3a1,
            yellow: 0xf9e2af,
            red: 0xf38ba8,
            orange: 0xfab387,
            aqua: 0x94e2d5,
            violet: 0xcba6f7,
        }
    }
}

#[inline]
pub fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}

#[inline]
pub fn hsla_alpha(hex_rgba: u32) -> Hsla {
    rgba(hex_rgba).into()
}

#[inline]
pub fn rgb_with_alpha(hex_rgb: u32, alpha_u8: u8) -> Hsla {
    rgba((hex_rgb << 8) | (alpha_u8 as u32)).into()
}

fn split_rgb(hex: u32) -> (u8, u8, u8) {
    (
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn blend_rgb(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let (ar, ag, ab) = split_rgb(a);
    let (br, bg, bb) = split_rgb(b);
    let lerp = |x: u8, y: u8| -> u8 {
        ((x as f32) + ((y as f32) - (x as f32)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    pack_rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
}

fn is_dark_rgb(hex: u32) -> bool {
    let (r, g, b) = split_rgb(hex);
    let luma = 0.2126 * (r as f32) + 0.7152 * (g as f32) + 0.0722 * (b as f32);
    luma < 140.0
}

fn parse_rgb_triple(raw: &str) -> Option<u32> {
    let mut parts = raw.split(',').map(|s| s.trim().parse::<u8>().ok());
    let r = parts.next()??;
    let g = parts.next()??;
    let b = parts.next()??;
    Some(pack_rgb(r, g, b))
}

/// Parses KDE Plasma's `~/.config/kdeglobals` INI color sections into a cohesive UI [`Palette`].
pub fn parse_kdeglobals_palette(contents: &str) -> Option<Palette> {
    let mut sections: HashMap<String, HashMap<String, u32>> = HashMap::new();
    let mut current_section = String::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=')
            && current_section.starts_with("Colors:")
            && let Some(color) = parse_rgb_triple(v)
        {
            sections
                .entry(current_section.clone())
                .or_default()
                .insert(k.trim().to_string(), color);
        }
    }

    let win = sections.get("Colors:Window")?;
    let bg_main = *win.get("BackgroundNormal")?;
    let is_dark = is_dark_rgb(bg_main);
    let bg_panel = win
        .get("BackgroundAlternate")
        .copied()
        .unwrap_or_else(|| blend_rgb(bg_main, if is_dark { 0xffffff } else { 0x000000 }, 0.08));

    let view = sections.get("Colors:View");
    let view_bg = view.and_then(|m| m.get("BackgroundNormal").copied()).unwrap_or(bg_main);
    let bg_deep = if view_bg != bg_main {
        blend_rgb(view_bg, 0x000000, if is_dark { 0.28 } else { 0.05 })
    } else {
        blend_rgb(bg_main, if is_dark { 0x000000 } else { 0x000000 }, if is_dark { 0.45 } else { 0.08 })
    };
    let bg_sidebar = view
        .and_then(|m| m.get("BackgroundAlternate").copied())
        .unwrap_or_else(|| blend_rgb(bg_main, bg_panel, 0.35));

    let btn = sections.get("Colors:Button");
    let bg_btn = btn
        .and_then(|m| m.get("BackgroundNormal").copied())
        .unwrap_or_else(|| blend_rgb(bg_panel, if is_dark { 0xffffff } else { 0x000000 }, 0.08));
    let bg_btn_hover = blend_rgb(bg_btn, if is_dark { 0xffffff } else { 0x000000 }, 0.07);
    let bg_btn_active = blend_rgb(bg_btn, 0x000000, 0.14);

    let sel = sections.get("Colors:Selection");
    let fg = win.get("ForegroundNormal").copied().unwrap_or(0xdcd7ba);
    let fg_muted = win
        .get("ForegroundInactive")
        .copied()
        .unwrap_or_else(|| blend_rgb(bg_main, fg, 0.45));
    let fg_subtle = blend_rgb(fg_muted, fg, 0.45);

    let accent = sel
        .and_then(|m| m.get("BackgroundNormal").copied())
        .or_else(|| win.get("DecorationFocus").copied())
        .unwrap_or(0x8ba4b0);
    let accent_hover = blend_rgb(accent, 0xffffff, 0.16);
    let accent_active = blend_rgb(accent, 0x000000, 0.14);

    let green = win.get("ForegroundPositive").copied().unwrap_or(0x87a987);
    let yellow = win.get("ForegroundNeutral").copied().unwrap_or(0xc4b28a);
    let red = win.get("ForegroundNegative").copied().unwrap_or(0xc4746e);
    let aqua = win.get("ForegroundLink").copied().unwrap_or(0x8ea4a2);
    let violet = win.get("ForegroundVisited").copied().unwrap_or(0x8992a7);
    let orange = blend_rgb(yellow, red, 0.35);

    Some(Palette {
        is_dark,
        bg_deep,
        bg_main,
        bg_sidebar,
        bg_panel,
        bg_btn,
        bg_btn_hover,
        bg_btn_active,
        border: bg_btn,
        border_subtle: blend_rgb(bg_main, bg_btn, 0.55),
        fg,
        fg_muted,
        fg_subtle,
        accent,
        accent_hover,
        accent_active,
        green,
        yellow,
        red,
        orange,
        aqua,
        violet,
    })
}

fn kdeglobals_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kdeglobals"))
}

pub fn load_palette(preset: ThemePreset) -> Palette {
    match preset {
        ThemePreset::VibesCrimson => Palette::vibes_crimson(),
        ThemePreset::PlasmaNative => kdeglobals_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|txt| parse_kdeglobals_palette(&txt))
            .unwrap_or_else(Palette::vibes_crimson),
        ThemePreset::KanagawaDragon => Palette::kanagawa_dragon(),
        ThemePreset::BreezeDark => Palette::breeze_dark(),
        ThemePreset::CatppuccinMocha => Palette::catppuccin_mocha(),
    }
}

pub fn apply_palette(cx: &mut App, p: &Palette) {
    Theme::change(
        if p.is_dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );

    Theme::update(cx, |theme| {
        theme.radius = px(6.);
        theme.radius_lg = px(8.);
        theme.font_size = px(13.5);
        theme.mono_font_size = px(12.5);

        let c = &mut theme.colors;
        c.background = hsla(p.bg_main);
        c.foreground = hsla(p.fg);
        c.border = hsla(p.border);
        c.window_border = hsla(p.border);
        c.muted = hsla(p.bg_panel);
        c.muted_foreground = hsla(p.fg_muted);
        c.accent = hsla(p.bg_panel);
        c.accent_foreground = hsla(p.fg);
        c.overlay = rgb_with_alpha(p.bg_deep, 0xcc);

        c.popover = hsla(p.bg_panel);
        c.popover_foreground = hsla(p.fg);
        c.group_box = hsla(p.bg_sidebar);
        c.group_box_foreground = hsla(p.fg);

        c.button = hsla(p.bg_btn);
        c.button_hover = hsla(p.bg_btn_hover);
        c.button_active = hsla(p.bg_btn_active);
        c.button_foreground = hsla(p.fg);

        c.button_secondary = hsla(p.bg_panel);
        c.button_secondary_hover = hsla(p.bg_btn);
        c.button_secondary_active = hsla(p.bg_btn_active);
        c.button_secondary_foreground = hsla(p.fg);

        let on_accent = hsla(if is_dark_rgb(p.accent) { 0xffffff } else { p.bg_main });
        c.primary = hsla(p.accent);
        c.primary_hover = hsla(p.accent_hover);
        c.primary_active = hsla(p.accent_active);
        c.primary_foreground = on_accent;

        c.button_primary = hsla(p.accent);
        c.button_primary_hover = hsla(p.accent_hover);
        c.button_primary_active = hsla(p.accent_active);
        c.button_primary_foreground = on_accent;

        c.secondary = hsla(p.bg_btn);
        c.secondary_hover = hsla(p.bg_btn_hover);
        c.secondary_active = hsla(p.bg_btn_active);
        c.secondary_foreground = hsla(p.fg);

        c.danger = hsla(p.red);
        c.danger_hover = hsla(blend_rgb(p.red, 0xffffff, 0.14));
        c.danger_active = hsla(blend_rgb(p.red, 0x000000, 0.14));
        c.danger_foreground = hsla(p.bg_main);

        c.success = hsla(p.green);
        c.success_hover = hsla(blend_rgb(p.green, 0xffffff, 0.14));
        c.success_active = hsla(blend_rgb(p.green, 0x000000, 0.14));
        c.success_foreground = hsla(p.bg_main);

        c.warning = hsla(p.yellow);
        c.warning_hover = hsla(blend_rgb(p.yellow, 0xffffff, 0.14));
        c.warning_active = hsla(blend_rgb(p.yellow, 0x000000, 0.14));
        c.warning_foreground = hsla(p.bg_main);

        c.sidebar = hsla(p.bg_sidebar);
        c.sidebar_border = hsla(p.border_subtle);
        c.sidebar_foreground = hsla(p.fg);
        c.sidebar_accent = hsla(p.bg_panel);
        c.sidebar_accent_foreground = hsla(p.fg);
        c.sidebar_primary = hsla(p.accent);
        c.sidebar_primary_foreground = on_accent;

        c.title_bar = hsla(p.bg_deep);
        c.title_bar_border = hsla(p.border_subtle);
        c.status_bar = hsla(p.bg_deep);
        c.status_bar_border = hsla(p.border_subtle);

        c.input = hsla(p.border);
        c.ring = hsla(p.accent);
        c.caret = hsla(p.accent);
        c.selection = rgb_with_alpha(p.accent, 0x44);

        c.link = hsla(p.aqua);
        c.link_hover = hsla(p.accent_hover);
        c.link_active = hsla(p.accent);

        c.list = hsla(p.bg_sidebar);
        c.list_even = hsla(p.bg_sidebar);
        c.list_head = hsla(p.bg_panel);
        c.list_hover = hsla(p.bg_panel);
        c.list_active = rgb_with_alpha(p.accent, 0x2e);
        c.list_active_border = hsla(p.accent);
        c.list_highlight_foreground = hsla(p.yellow);

        c.scrollbar = hsla_alpha(0x00000000);
        c.scrollbar_thumb = rgb_with_alpha(p.fg_muted, 0x66);
        c.scrollbar_thumb_hover = rgb_with_alpha(p.fg_subtle, 0xaa);

        c.red = hsla(p.red);
        c.green = hsla(p.green);
        c.blue = hsla(p.accent);
        c.yellow = hsla(p.yellow);
        c.magenta = hsla(p.violet);
        c.cyan = hsla(p.aqua);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kdeglobals_kanagawa_dragon_colors() {
        let sample = r#"
[Colors:Button]
BackgroundNormal=57,56,54

[Colors:Selection]
BackgroundNormal=139,164,176

[Colors:View]
BackgroundAlternate=29,28,25
BackgroundNormal=24,22,22

[Colors:Window]
BackgroundAlternate=40,39,39
BackgroundNormal=24,22,22
DecorationFocus=139,164,176
ForegroundInactive=114,113,105
ForegroundLink=143,161,178
ForegroundNegative=196,116,110
ForegroundNeutral=196,178,138
ForegroundNormal=220,215,186
ForegroundPositive=135,169,135
ForegroundVisited=137,146,167
"#;
        let pal = parse_kdeglobals_palette(sample).expect("kdeglobals should parse");
        assert!(pal.is_dark);
        assert_eq!(pal.bg_main, 0x181616);
        assert_eq!(pal.bg_sidebar, 0x1d1c19);
        assert_eq!(pal.bg_panel, 0x282727);
        assert_eq!(pal.bg_btn, 0x393836);
        assert_eq!(pal.fg, 0xdcd7ba);
        assert_eq!(pal.fg_muted, 0x727169);
        assert_eq!(pal.accent, 0x8ba4b0);
        assert_eq!(pal.green, 0x87a987);
        assert_eq!(pal.red, 0xc4746e);
    }

    #[test]
    fn loads_vibes_crimson_default_palette() {
        let pal = load_palette(ThemePreset::default());
        assert!(pal.is_dark);
        assert_eq!(ThemePreset::default(), ThemePreset::VibesCrimson);
        assert_eq!(pal.accent, 0xe04f54);
        assert_eq!(pal.bg_main, 0x1a1013);
    }
}
