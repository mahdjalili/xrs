use ratatui::style::Color;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ThemeColors {
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
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub cyan: String,
    pub magenta: String,

    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
}

fn default_mode() -> String { "dark".to_string() }
fn default_accent() -> String { "#4274bf".to_string() }
fn default_selection() -> String { "#2a4365".to_string() }
fn default_muted() -> String { "#718096".to_string() }
fn default_bg() -> String { "#1a202c".to_string() }
fn default_dark_bg() -> String { "#171923".to_string() }
fn default_darker_bg() -> String { "#0f1117".to_string() }
fn default_lighter_bg() -> String { "#2d3748".to_string() }
fn default_fg() -> String { "#e2e8f0".to_string() }
fn default_dark_fg() -> String { "#a0aec0".to_string() }
fn default_light_fg() -> String { "#edf2f7".to_string() }
fn default_bright_fg() -> String { "#f7fafc".to_string() }
fn default_red() -> String { "#e53e3e".to_string() }
fn default_green() -> String { "#38a169".to_string() }
fn default_yellow() -> String { "#d69e2e".to_string() }
fn default_blue() -> String { "#3182ce".to_string() }
fn default_cyan() -> String { "#319795".to_string() }
fn default_magenta() -> String { "#b83280".to_string() }
fn default_bright_red() -> String { "#fc8181".to_string() }
fn default_bright_green() -> String { "#68d391".to_string() }
fn default_bright_yellow() -> String { "#f6e05e".to_string() }

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            accent: default_accent(),
            selection: default_selection(),
            muted: default_muted(),
            background: default_bg(),
            dark_background: default_dark_bg(),
            darker_background: default_darker_bg(),
            lighter_background: default_lighter_bg(),
            foreground: default_fg(),
            dark_foreground: default_dark_fg(),
            light_foreground: default_light_fg(),
            bright_foreground: default_bright_fg(),
            red: default_red(),
            green: default_green(),
            yellow: default_yellow(),
            blue: default_blue(),
            cyan: default_cyan(),
            magenta: default_magenta(),
            bright_red: default_bright_red(),
            bright_green: default_bright_green(),
            bright_yellow: default_bright_yellow(),
        }
    }
}

impl ThemeColors {
    /// Reads a flat `key = "value"` TOML file such as Omarchy's `colors.toml`.
    /// As with a strict TOML deserializer, a known key holding something other
    /// than a string makes the whole file fall back to the defaults.
    fn from_toml(content: &str) -> Option<Self> {
        let mut values: HashMap<&str, String> = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                break;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=')?;
            let key = key.trim().trim_matches('"');
            if !KNOWN_KEYS.contains(&key) {
                continue;
            }
            if values.insert(key, toml_string(value.trim())?).is_some() {
                return None;
            }
        }
        let mut get = |key: &str, default: fn() -> String| values.remove(key).unwrap_or_else(default);
        Some(Self {
            mode: get("mode", default_mode),
            accent: get("accent", default_accent),
            selection: get("selection", default_selection),
            muted: get("muted", default_muted),
            background: get("background", default_bg),
            dark_background: get("dark_background", default_dark_bg),
            darker_background: get("darker_background", default_darker_bg),
            lighter_background: get("lighter_background", default_lighter_bg),
            foreground: get("foreground", default_fg),
            dark_foreground: get("dark_foreground", default_dark_fg),
            light_foreground: get("light_foreground", default_light_fg),
            bright_foreground: get("bright_foreground", default_bright_fg),
            red: get("red", default_red),
            green: get("green", default_green),
            yellow: get("yellow", default_yellow),
            blue: get("blue", default_blue),
            cyan: get("cyan", default_cyan),
            magenta: get("magenta", default_magenta),
            bright_red: get("bright_red", default_bright_red),
            bright_green: get("bright_green", default_bright_green),
            bright_yellow: get("bright_yellow", default_bright_yellow),
        })
    }
}

const KNOWN_KEYS: [&str; 21] = [
    "mode", "accent", "selection", "muted", "background", "dark_background", "darker_background",
    "lighter_background", "foreground", "dark_foreground", "light_foreground", "bright_foreground",
    "red", "green", "yellow", "blue", "cyan", "magenta", "bright_red", "bright_green", "bright_yellow",
];

/// A single-line TOML basic (`"..."`) or literal (`'...'`) string, optionally
/// followed by a comment.
fn toml_string(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let quote = chars.next().filter(|q| *q == '"' || *q == '\'')?;
    let mut out = String::new();
    loop {
        match chars.next()? {
            c if c == quote => break,
            '\\' if quote == '"' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'e' => out.push('\u{1b}'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'u' => out.push(char::from_u32(u32::from_str_radix(&chars.by_ref().take(4).collect::<String>(), 16).ok()?)?),
                'U' => out.push(char::from_u32(u32::from_str_radix(&chars.by_ref().take(8).collect::<String>(), 16).ok()?)?),
                _ => return None,
            },
            c => out.push(c),
        }
    }
    let rest = chars.as_str().trim_start();
    (rest.is_empty() || rest.starts_with('#')).then_some(out)
}

pub fn parse_hex_color(hex: &str) -> (u8, u8, u8) {
    let clean = hex.trim().trim_start_matches('#');
    if clean.len() >= 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(200);
        let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(200);
        let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(200);
        (r, g, b)
    } else {
        (200, 200, 200)
    }
}

pub fn hex_to_ratatui(hex: &str) -> Color {
    let (r, g, b) = parse_hex_color(hex);
    Color::Rgb(r, g, b)
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub has_desktop_theme: bool,
    pub colors: ThemeColors,
    stamp: Option<FileStamp>,
}

/// Identifies one version of a file; Omarchy switches themes by repointing a
/// symlink, which changes the inode even if mtimes happen to match.
type FileStamp = (u64, u64, i64, i64, u64);

fn file_stamp(path: &std::path::Path) -> Option<FileStamp> {
    let m = fs::metadata(path).ok()?;
    Some((m.dev(), m.ino(), m.mtime(), m.mtime_nsec(), m.size()))
}

#[allow(dead_code)]
impl Theme {
    pub fn load() -> Self {
        let file = Self::file();
        let stamp = file_stamp(&file);
        if file.exists() {
            let colors = fs::read_to_string(&file)
                .ok()
                .and_then(|content| ThemeColors::from_toml(&content))
                .unwrap_or_default();
            Self { has_desktop_theme: true, colors, stamp }
        } else {
            Self { has_desktop_theme: false, colors: ThemeColors::default(), stamp }
        }
    }

    /// Reloads only if the theme file was switched or edited since last read.
    pub fn refresh(&mut self) {
        if file_stamp(&Self::file()) != self.stamp {
            *self = Self::load();
        }
    }

    fn file() -> PathBuf {
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".local/state/omarchy/current/theme/colors.toml")
    }

    pub fn accent(&self) -> Color {
        hex_to_ratatui(&self.colors.accent)
    }

    pub fn bg(&self) -> Color {
        if self.has_desktop_theme {
            hex_to_ratatui(&self.colors.background)
        } else {
            Color::Reset
        }
    }

    pub fn dark_bg(&self) -> Color {
        if self.has_desktop_theme {
            hex_to_ratatui(&self.colors.dark_background)
        } else {
            Color::Reset
        }
    }

    pub fn lighter_bg(&self) -> Color {
        if self.has_desktop_theme {
            hex_to_ratatui(&self.colors.lighter_background)
        } else {
            Color::Rgb(45, 55, 72)
        }
    }

    pub fn fg(&self) -> Color {
        if self.has_desktop_theme {
            hex_to_ratatui(&self.colors.foreground)
        } else {
            Color::Reset
        }
    }

    pub fn muted(&self) -> Color {
        hex_to_ratatui(&self.colors.muted)
    }

    pub fn green(&self) -> Color {
        hex_to_ratatui(&self.colors.green)
    }

    pub fn red(&self) -> Color {
        hex_to_ratatui(&self.colors.red)
    }

    pub fn yellow(&self) -> Color {
        hex_to_ratatui(&self.colors.yellow)
    }

    pub fn blue(&self) -> Color {
        hex_to_ratatui(&self.colors.blue)
    }

    pub fn cyan(&self) -> Color {
        hex_to_ratatui(&self.colors.cyan)
    }

    pub fn bright_fg(&self) -> Color {
        hex_to_ratatui(&self.colors.bright_foreground)
    }

    /// Text color for labels drawn on top of accent or status fills.
    pub fn on_accent(&self) -> Color {
        hex_to_ratatui(&self.colors.darker_background)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_omarchy_style_colors() {
        let c = ThemeColors::from_toml(
            "# Tokyo Night\naccent = \"#7aa2f7\"\nforeground = '#a9b1d6' # fg\n\"background\" = \"#1a1b26\"\ncolor0 = \"#32344a\"\nopacity = 0.9\n\n[extra]\nred = \"#000000\"\n",
        )
        .expect("valid");
        assert_eq!(c.accent, "#7aa2f7");
        assert_eq!(c.foreground, "#a9b1d6");
        assert_eq!(c.background, "#1a1b26");
        assert_eq!(c.red, default_red());
    }

    #[test]
    fn non_string_known_key_falls_back_to_defaults() {
        assert!(ThemeColors::from_toml("accent = 5\n").is_none());
        assert!(ThemeColors::from_toml("accent = \"#123456\" trailing\n").is_none());
    }

    #[test]
    fn basic_string_escapes() {
        assert_eq!(toml_string(r#""a\"b\u0041""#).as_deref(), Some("a\"bA"));
        assert_eq!(toml_string(r"'C:\path'").as_deref(), Some(r"C:\path"));
        assert_eq!(toml_string("\"open"), None);
    }
}
