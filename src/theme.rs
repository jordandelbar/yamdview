//! One theme for everything yamdview draws: markdown text, code blocks and diagrams.
//!
//! A theme is a set of roles (heading, keyword, string, ...), not a palette, because
//! themes disagree on what a color means: Dracula's keywords are pink, Catppuccin's
//! mauve. Each theme fills the roles from its own style guide.
//!
//! Loaded from `~/.config/yamdview/theme` (`role = #rrggbb` lines, see [`Theme::from_config`]),
//! else derived from Ghostty's palette, else Dracula.

use merman::render::{
    HostThemeAppearance, HostThemeOutput, HostThemeProfile, HostThemeRoles, HostThemeRootBackground,
};
use ratatui::style::{Color, Modifier, Style};
use std::{collections::HashMap, path::PathBuf, process::Command};
use tui_markdown::{AlertKind, CodeTheme, StyleSheet};

pub type Rgb = [u8; 3];

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub selection: Rgb,
    /// Rules, table borders, metadata.
    pub muted: Rgb,
    // Prose.
    pub heading: Rgb,
    pub bold: Rgb,
    pub italic: Rgb,
    /// Inline code.
    pub code: Rgb,
    pub link: Rgb,
    pub quote: Rgb,
    // Syntax highlighting.
    pub comment: Rgb,
    pub keyword: Rgb,
    pub string: Rgb,
    pub function: Rgb,
    /// Types and classes (config key `type`).
    pub ty: Rgb,
    pub number: Rgb,
    pub parameter: Rgb,
    // Diagrams and alerts.
    /// Diagram borders and node tint.
    pub accent: Rgb,
    /// Diagram notes.
    pub note: Rgb,
    pub info: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub error: Rgb,
    /// Font family for diagram text (config key `font-family`); `monospace` when unknown.
    pub font: String,
}

fn hex(s: &str) -> Option<Rgb> {
    let s = s.trim().strip_prefix('#')?;
    let n = u32::from_str_radix(s, 16).ok().filter(|_| s.len() == 6)?;
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

/// The eleven Dracula colors, the palette every non-configured theme is built from.
struct Palette {
    background: Rgb,
    foreground: Rgb,
    selection: Rgb,
    comment: Rgb,
    red: Rgb,
    orange: Rgb,
    yellow: Rgb,
    green: Rgb,
    cyan: Rgb,
    purple: Rgb,
    pink: Rgb,
}

impl Palette {
    /// Dracula's role assignments: its spec for code (https://draculatheme.com/spec) and
    /// its VS Code theme for markdown, which the spec doesn't cover.
    fn roles(self, font: String) -> Theme {
        Theme {
            background: self.background,
            foreground: self.foreground,
            selection: self.selection,
            muted: self.comment,
            heading: self.purple,
            bold: self.orange,
            italic: self.yellow,
            code: self.green,
            link: self.cyan,
            quote: self.yellow,
            comment: self.comment,
            keyword: self.pink,
            string: self.yellow,
            function: self.green,
            ty: self.cyan,
            number: self.orange,
            parameter: self.orange,
            accent: self.purple,
            note: self.yellow,
            info: self.cyan,
            success: self.green,
            warning: self.orange,
            error: self.red,
            font,
        }
    }
}

impl Theme {
    pub fn dracula() -> Self {
        Palette {
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
        }
        .roles("monospace".into())
    }

    /// The config file if there is one, else the running Ghostty's palette, else Dracula.
    /// A config file that is present but incomplete is reported rather than half-used.
    pub fn detect() -> Self {
        if let Some(path) = config_path().filter(|p| p.exists()) {
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| Self::from_config(&s)) {
                Ok(theme) => return theme,
                Err(e) => eprintln!("yamdview: ignoring {}: {e}", path.display()),
            }
        }
        Self::from_ghostty(&parse(&ghostty_config())).unwrap_or_else(Self::dracula)
    }

    /// Every role as a `role = #rrggbb` line, plus an optional `font-family = Name` line.
    pub fn from_config(config: &str) -> Result<Self, String> {
        let cfg = parse(config);
        let get = |k: &str| {
            let v = cfg.get(k).ok_or(format!("missing `{k}`"))?;
            hex(v).ok_or(format!("`{k} = {v}` is not a #rrggbb color"))
        };
        Ok(Self {
            background: get("background")?,
            foreground: get("foreground")?,
            selection: get("selection")?,
            muted: get("muted")?,
            heading: get("heading")?,
            bold: get("bold")?,
            italic: get("italic")?,
            code: get("code")?,
            link: get("link")?,
            quote: get("quote")?,
            comment: get("comment")?,
            keyword: get("keyword")?,
            string: get("string")?,
            function: get("function")?,
            ty: get("type")?,
            number: get("number")?,
            parameter: get("parameter")?,
            accent: get("accent")?,
            note: get("note")?,
            info: get("info")?,
            success: get("success")?,
            warning: get("warning")?,
            error: get("error")?,
            font: cfg.get("font-family").cloned().unwrap_or_else(|| "monospace".into()),
        })
    }

    /// From `ghostty +show-config` output. A Dracula theme gets the exact spec colors
    /// (the ANSI palette has no orange); anything else fills Dracula's palette from ANSI
    /// slots, which is how Dracula itself lays them out (blue slot = purple, magenta = pink).
    fn from_ghostty(cfg: &HashMap<String, String>) -> Option<Self> {
        let get = |k: &str| cfg.get(k).and_then(|v| hex(v));
        let pal = |n: u8| get(&format!("palette{n}"));
        let font = cfg.get("font-family").cloned().unwrap_or_else(|| "monospace".into());
        if cfg.get("theme").is_some_and(|t| t.to_lowercase().contains("dracula")) {
            return Some(Self { font, ..Self::dracula() });
        }
        let (background, foreground) = (get("background")?, get("foreground")?);
        let (red, yellow) = (pal(1)?, pal(3)?);
        let palette = Palette {
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
        };
        Some(palette.roles(font))
    }

    pub fn is_dark(&self) -> bool {
        self.background.iter().map(|&c| u32::from(c)).sum::<u32>() < 3 * 128
    }

    /// Mermaid theme. Text is sized to the terminal cell height so it matches the
    /// surrounding text, and the background stays transparent so the terminal shows through.
    pub fn mermaid(&self, cell_h: u32) -> HostThemeProfile {
        let (bg, fg) = (self.background, self.foreground);
        let c = |rgb: Rgb| Some(css(rgb));
        let surface = mix(bg, self.accent, 0.12);
        HostThemeProfile::builder()
            .appearance(if self.is_dark() { HostThemeAppearance::Dark } else { HostThemeAppearance::Light })
            .font_family(format!("\"{}\", monospace", self.font))
            // ponytail: 0.7 of the cell height approximates the terminal font's px size.
            .font_size(format!("{}px", (cell_h as f32 * 0.7).round()))
            .roles(HostThemeRoles {
                canvas: c(bg),
                surface: c(surface),
                surface_alt: c(mix(bg, self.accent, 0.22)),
                surface_muted: c(mix(bg, fg, 0.05)),
                text: c(fg),
                subtle_text: c(mix(fg, bg, 0.35)),
                border: c(self.accent),
                line: c(mix(fg, bg, 0.3)),
                edge_label_background: c(bg),
                cluster_background: c(mix(bg, fg, 0.04)),
                cluster_border: c(self.muted),
                note_background: c(mix(bg, self.note, 0.15)),
                note_border: c(self.note),
                note_text: c(fg),
                actor_background: c(surface),
                actor_border: c(self.accent),
                actor_text: c(fg),
                activation_background: c(mix(bg, self.accent, 0.3)),
                activation_border: c(self.accent),
                error: c(self.error),
                warning: c(self.warning),
                success: c(self.success),
                ..HostThemeRoles::default()
            })
            .series_palette(
                [self.accent, self.success, self.keyword, self.info, self.number, self.note, self.error].map(css),
            )
            .output(HostThemeOutput {
                root_background: HostThemeRootBackground::Color("transparent".into()),
                ..HostThemeOutput::resvg_safe_editor()
            })
            .build()
    }

    /// Syntax highlighting for fenced code, as a TextMate theme.
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
            rule("string", self.string, ""),
            rule("constant.numeric, constant.language, constant.character", self.number, ""),
            rule("keyword, storage, storage.type, entity.name.tag", self.keyword, ""),
            rule("entity.name.function, support.function, meta.function-call", self.function, ""),
            rule("entity.name.type, entity.name.class, support.type, support.class", self.ty, ""),
            rule("variable.parameter", self.parameter, "italic"),
            rule("invalid", self.error, ""),
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
    /// rendering. Spans that already have a color (headings, links, code) keep it.
    pub fn prose(&self, style: Style) -> Style {
        match style.fg {
            Some(_) => style,
            None if style.add_modifier.contains(Modifier::BOLD) => style.fg(color(self.bold)),
            None if style.add_modifier.contains(Modifier::ITALIC) => style.fg(color(self.italic)),
            None => style,
        }
    }
}

/// Markdown styles.
impl StyleSheet for Theme {
    fn heading(&self, level: u8) -> Style {
        let s = Style::new().fg(color(self.heading)).bold();
        if level == 1 { s.underlined() } else { s }
    }
    fn code(&self) -> Style {
        Style::new().fg(color(self.code))
    }
    fn link(&self) -> Style {
        Style::new().fg(color(self.link)).underlined()
    }
    fn blockquote(&self) -> Style {
        Style::new().fg(color(self.quote)).italic()
    }
    fn heading_meta(&self) -> Style {
        Style::new().fg(color(self.muted))
    }
    fn metadata_block(&self) -> Style {
        Style::new().fg(color(self.muted))
    }
    fn html(&self) -> Style {
        Style::new().fg(color(self.muted))
    }
    fn table_header(&self) -> Style {
        Style::new().fg(color(self.heading)).bold()
    }
    fn table_border(&self) -> Style {
        Style::new().fg(color(self.muted))
    }
    fn footnote_ref(&self) -> Style {
        Style::new().fg(color(self.link))
    }
    fn alert(&self, kind: AlertKind) -> Style {
        let c = match kind {
            AlertKind::Note => self.info,
            AlertKind::Tip => self.success,
            AlertKind::Important => self.accent,
            AlertKind::Warning => self.warning,
            AlertKind::Caution => self.error,
        };
        Style::new().fg(color(c))
    }
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("yamdview/theme"))
}

/// Ghostty's resolved config text, empty outside Ghostty.
fn ghostty_config() -> String {
    Command::new("ghostty")
        .arg("+show-config")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// `key = value` lines, the format of both Ghostty's config and ours. Ghostty's
/// `palette = N=#rrggbb` lines become `paletteN` keys.
fn parse(config: &str) -> HashMap<String, String> {
    config
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .filter(|(k, _)| !k.is_empty() && !k.starts_with('#'))
        .map(|(k, v)| match (k, v.split_once('=')) {
            ("palette", Some((n, c))) => (format!("palette{}", n.trim()), c.trim().to_string()),
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

    const DRACULA_CONFIG: &str = "\
        # comments and blank lines are fine\n\n\
        background = #282a36\nforeground = #f8f8f2\nselection = #44475a\nmuted = #6272a4\n\
        heading = #bd93f9\nbold = #ffb86c\nitalic = #f1fa8c\ncode = #50fa7b\nlink = #8be9fd\n\
        quote = #f1fa8c\ncomment = #6272a4\nkeyword = #ff79c6\nstring = #f1fa8c\n\
        function = #50fa7b\ntype = #8be9fd\nnumber = #ffb86c\nparameter = #ffb86c\n\
        accent = #bd93f9\nnote = #f1fa8c\ninfo = #8be9fd\nsuccess = #50fa7b\n\
        warning = #ffb86c\nerror = #ff5555\nfont-family = JetBrainsMono Nerd Font\n";

    #[test]
    fn config_file_sets_every_role() {
        let t = Theme::from_config(DRACULA_CONFIG).unwrap();
        assert_eq!(t, Theme { font: "JetBrainsMono Nerd Font".into(), ..Theme::dracula() });
    }

    #[test]
    fn config_file_errors_name_the_role() {
        let missing = DRACULA_CONFIG.replace("keyword = #ff79c6\n", "");
        assert_eq!(Theme::from_config(&missing).unwrap_err(), "missing `keyword`");
        let bad = DRACULA_CONFIG.replace("#ff79c6", "pink");
        assert_eq!(Theme::from_config(&bad).unwrap_err(), "`keyword = pink` is not a #rrggbb color");
    }

    #[test]
    fn ghostty_dracula_uses_exact_spec_colors() {
        let cfg = parse("font-family = JetBrainsMono Nerd Font\nfont-family = Noto\ntheme = dracula\n");
        let t = Theme::from_ghostty(&cfg).unwrap();
        assert_eq!(t, Theme { font: "JetBrainsMono Nerd Font".into(), ..Theme::dracula() });
    }

    #[test]
    fn other_ghostty_themes_fill_dracula_roles_from_ansi() {
        let cfg = parse(
            "theme = gruvbox\nbackground = #282828\nforeground = #ebdbb2\npalette = 1=#cc241d\n\
             palette = 2=#98971a\npalette = 3=#d79921\npalette = 4=#458588\npalette = 5=#b16286\n\
             palette = 6=#689d6a\npalette = 8=#928374\n",
        );
        let t = Theme::from_ghostty(&cfg).unwrap();
        assert_eq!((t.heading, t.keyword, t.comment), ([0x45, 0x85, 0x88], [0xb1, 0x62, 0x86], [0x92, 0x83, 0x74]));
        assert_eq!(t.number, mix(t.error, t.string, 0.5));
        assert!(t.is_dark());
        // Missing palette entries mean we can't trust it: fall back to Dracula.
        assert!(Theme::from_ghostty(&parse("background = #000000\n")).is_none());
    }

    #[test]
    fn generated_code_theme_parses() {
        Theme::dracula().code();
    }
}
