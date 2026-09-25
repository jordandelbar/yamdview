//! One theme for everything yamdview draws: markdown text, code blocks and diagrams.
//! Roles follow the Dracula spec (https://draculatheme.com/spec); other themes fill
//! the same roles from the terminal's ANSI palette.

use merman::render::{
    HostThemeAppearance, HostThemeOutput, HostThemeProfile, HostThemeRoles, HostThemeRootBackground,
};
use ratatui::style::{Color, Modifier, Style};
use std::{collections::HashMap, process::Command};
use tui_markdown::{AlertKind, CodeTheme, StyleSheet};

pub type Rgb = [u8; 3];

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub selection: Rgb,
    pub comment: Rgb,
    pub red: Rgb,
    pub orange: Rgb,
    pub yellow: Rgb,
    pub green: Rgb,
    pub cyan: Rgb,
    pub purple: Rgb,
    pub pink: Rgb,
    /// Font family for diagram text; `monospace` when unknown.
    pub font: String,
}

fn hex(s: &str) -> Option<Rgb> {
    let n = u32::from_str_radix(s.trim().trim_start_matches('#'), 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

fn css(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// `t` of the way from `a` to `b`.
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8)
}

pub fn color(c: Rgb) -> Color {
    Color::Rgb(c[0], c[1], c[2])
}

impl Theme {
    pub fn dracula() -> Self {
        Self {
            background: [0x28, 0x2a, 0x36],
            foreground: [0xf8, 0xf8, 0xf2],
            selection: [0x44, 0x47, 0x5a],
            comment: [0x62, 0x72, 0xa4],
            red: [0xff, 0x55, 0x55],
            orange: [0xff, 0xb8, 0x6c],
            yellow: [0xf1, 0xfa, 0x8c],
            green: [0x50, 0xfa, 0x7b],
            cyan: [0x8b, 0xe9, 0xfd],
            purple: [0xbd, 0x93, 0xf9],
            pink: [0xff, 0x79, 0xc6],
            font: "monospace".into(),
        }
    }

    /// Theme of the running Ghostty, falling back to Dracula outside Ghostty.
    pub fn detect() -> Self {
        Self::from_ghostty(&ghostty_config()).unwrap_or_else(Self::dracula)
    }

    /// From `ghostty +show-config` output. Dracula themes get the exact spec colors
    /// (the ANSI palette has no orange); anything else maps roles onto ANSI slots,
    /// which is how Dracula itself lays them out (blue slot = purple, magenta = pink).
    fn from_ghostty(cfg: &HashMap<String, String>) -> Option<Self> {
        let get = |k: &str| cfg.get(k).and_then(|v| hex(v));
        let pal = |n: u8| get(&format!("palette{n}"));
        let font = cfg.get("font-family").cloned().unwrap_or_else(|| "monospace".into());
        if cfg.get("theme").is_some_and(|t| t.to_lowercase().contains("dracula")) {
            return Some(Self { font, ..Self::dracula() });
        }
        let (background, foreground) = (get("background")?, get("foreground")?);
        let (red, yellow) = (pal(1)?, pal(3)?);
        Some(Self {
            selection: get("selection-background").unwrap_or(mix(background, foreground, 0.15)),
            comment: pal(8)?,
            orange: mix(red, yellow, 0.5),
            green: pal(2)?,
            cyan: pal(6)?,
            purple: pal(4)?,
            pink: pal(5)?,
            background,
            foreground,
            red,
            yellow,
            font,
        })
    }

    pub fn is_dark(&self) -> bool {
        self.background.iter().map(|&c| u32::from(c)).sum::<u32>() < 3 * 128
    }

    /// Mermaid theme. Text is sized to the terminal cell height so it matches the
    /// surrounding text, and the background stays transparent so the terminal shows through.
    pub fn mermaid(&self, cell_h: u32) -> HostThemeProfile {
        let (bg, fg) = (self.background, self.foreground);
        let c = |rgb: Rgb| Some(css(rgb));
        let surface = mix(bg, self.purple, 0.12);
        HostThemeProfile::builder()
            .appearance(if self.is_dark() { HostThemeAppearance::Dark } else { HostThemeAppearance::Light })
            .font_family(format!("\"{}\", monospace", self.font))
            // ponytail: 0.7 of the cell height approximates the terminal font's px size.
            .font_size(format!("{}px", (cell_h as f32 * 0.7).round()))
            .roles(HostThemeRoles {
                canvas: c(bg),
                surface: c(surface),
                surface_alt: c(mix(bg, self.purple, 0.22)),
                surface_muted: c(mix(bg, fg, 0.05)),
                text: c(fg),
                subtle_text: c(mix(fg, bg, 0.35)),
                border: c(self.purple),
                line: c(mix(fg, bg, 0.3)),
                edge_label_background: c(bg),
                cluster_background: c(mix(bg, fg, 0.04)),
                cluster_border: c(self.comment),
                note_background: c(mix(bg, self.yellow, 0.15)),
                note_border: c(self.yellow),
                note_text: c(fg),
                actor_background: c(surface),
                actor_border: c(self.purple),
                actor_text: c(fg),
                activation_background: c(mix(bg, self.purple, 0.3)),
                activation_border: c(self.purple),
                error: c(self.red),
                warning: c(self.orange),
                success: c(self.green),
                ..HostThemeRoles::default()
            })
            .series_palette(
                [self.purple, self.green, self.pink, self.cyan, self.orange, self.yellow, self.red].map(css),
            )
            .output(HostThemeOutput {
                root_background: HostThemeRootBackground::Color("transparent".into()),
                ..HostThemeOutput::resvg_safe_editor()
            })
            .build()
    }

    /// Syntax highlighting for fenced code, as a TextMate theme with Dracula's token rules.
    pub fn code(&self) -> CodeTheme {
        let rule = |scope: &str, c: Rgb, font: &str| {
            format!(
                "<dict><key>scope</key><string>{scope}</string><key>settings</key><dict>\
                 <key>foreground</key><string>{}</string><key>fontStyle</key><string>{font}</string></dict></dict>",
                css(c)
            )
        };
        let rules = [
            rule("comment", self.comment, "italic"),
            rule("string, markup.inline.raw", self.yellow, ""),
            rule("constant.numeric, constant.language, constant.character", self.orange, ""),
            rule("keyword, storage, storage.type", self.pink, ""),
            rule("entity.name.function, support.function, meta.function-call", self.green, ""),
            rule("entity.name.type, entity.name.class, support.type, support.class", self.cyan, "italic"),
            rule("entity.other.inherited-class, string.regexp", self.cyan, ""),
            rule("variable.parameter, entity.name.type.parameter", self.orange, "italic"),
            rule("variable.language", self.purple, "italic"),
            rule("entity.name.tag", self.pink, ""),
            rule("entity.other.attribute-name", self.green, "italic"),
            rule("invalid", self.red, ""),
        ]
        .concat();
        let theme = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
             <key>name</key><string>yamdview</string><key>settings</key><array>\
             <dict><key>settings</key><dict><key>foreground</key><string>{}</string></dict></dict>\
             {rules}</array></dict></plist>",
            css(self.foreground)
        );
        CodeTheme::from_textmate(&theme).expect("generated theme is valid")
    }

    /// Bold and italic aren't themable in tui-markdown, so they're recolored after
    /// rendering: Dracula prose is bold orange, italic yellow. Spans that already have
    /// a color (headings, links, code) keep it.
    pub fn prose(&self, style: Style) -> Style {
        match style.fg {
            Some(_) => style,
            None if style.add_modifier.contains(Modifier::BOLD) => style.fg(color(self.orange)),
            None if style.add_modifier.contains(Modifier::ITALIC) => style.fg(color(self.yellow)),
            None => style,
        }
    }
}

/// Markdown styles from Dracula's prose rules.
impl StyleSheet for Theme {
    fn heading(&self, level: u8) -> Style {
        let s = Style::new().fg(color(self.purple)).bold();
        if level == 1 { s.underlined() } else { s }
    }
    fn code(&self) -> Style {
        Style::new().fg(color(self.green))
    }
    fn link(&self) -> Style {
        Style::new().fg(color(self.cyan)).underlined()
    }
    fn blockquote(&self) -> Style {
        Style::new().fg(color(self.yellow)).italic()
    }
    fn heading_meta(&self) -> Style {
        Style::new().fg(color(self.comment))
    }
    fn metadata_block(&self) -> Style {
        Style::new().fg(color(self.comment))
    }
    fn html(&self) -> Style {
        Style::new().fg(color(self.comment))
    }
    fn table_header(&self) -> Style {
        Style::new().fg(color(self.purple)).bold()
    }
    fn table_border(&self) -> Style {
        Style::new().fg(color(self.comment))
    }
    fn footnote_ref(&self) -> Style {
        Style::new().fg(color(self.cyan))
    }
    fn alert(&self, kind: AlertKind) -> Style {
        let c = match kind {
            AlertKind::Note => self.cyan,
            AlertKind::Tip => self.green,
            AlertKind::Important => self.purple,
            AlertKind::Warning => self.orange,
            AlertKind::Caution => self.red,
        };
        Style::new().fg(color(c))
    }
}

/// Ghostty's resolved config: `key = value` lines; palette lines are `palette = N=#rrggbb`.
fn ghostty_config() -> HashMap<String, String> {
    let Ok(out) = Command::new("ghostty").arg("+show-config").output() else {
        return HashMap::new();
    };
    parse_ghostty(&String::from_utf8_lossy(&out.stdout))
}

fn parse_ghostty(config: &str) -> HashMap<String, String> {
    config
        .lines()
        .filter_map(|l| l.split_once(" = "))
        .map(|(k, v)| match (k, v.split_once('=')) {
            ("palette", Some((n, c))) => (format!("palette{n}"), c.to_string()),
            _ => (k.to_string(), v.to_string()),
        })
        // First wins: font-family can repeat for fallbacks.
        .fold(HashMap::new(), |mut m, (k, v)| {
            m.entry(k).or_insert(v);
            m
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghostty_dracula_uses_exact_spec_colors() {
        let cfg = parse_ghostty("font-family = JetBrainsMono Nerd Font\nfont-family = Noto\ntheme = dracula\n");
        let t = Theme::from_ghostty(&cfg).unwrap();
        assert_eq!(t, Theme { font: "JetBrainsMono Nerd Font".into(), ..Theme::dracula() });
    }

    #[test]
    fn other_themes_map_roles_from_ansi_palette() {
        let cfg = parse_ghostty(
            "theme = gruvbox\nbackground = #282828\nforeground = #ebdbb2\npalette = 1=#cc241d\n\
             palette = 2=#98971a\npalette = 3=#d79921\npalette = 4=#458588\npalette = 5=#b16286\n\
             palette = 6=#689d6a\npalette = 8=#928374\n",
        );
        let t = Theme::from_ghostty(&cfg).unwrap();
        assert_eq!((t.purple, t.pink, t.comment), ([0x45, 0x85, 0x88], [0xb1, 0x62, 0x86], [0x92, 0x83, 0x74]));
        assert_eq!(t.orange, mix(t.red, t.yellow, 0.5));
        assert!(t.is_dark());
        // Missing palette entries mean we can't trust it: fall back to Dracula.
        assert!(Theme::from_ghostty(&parse_ghostty("background = #000000\n")).is_none());
    }

    #[test]
    fn generated_code_theme_parses() {
        Theme::dracula().code();
    }
}
