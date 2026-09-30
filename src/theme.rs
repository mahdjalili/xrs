use std::fs;
use std::path::PathBuf;
use ratatui::style::Color;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ThemeColors {
    #[serde(default = "default_mode")]
    pub mode: String,

    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_selection")]
    pub selection: String,
    #[serde(default = "default_muted")]
    pub muted: String,

    #[serde(default = "default_bg")]
    pub background: String,
    #[serde(default = "default_dark_bg")]
    pub dark_background: String,
    #[serde(default = "default_darker_bg")]
    pub darker_background: String,
    #[serde(default = "default_lighter_bg")]
    pub lighter_background: String,

    #[serde(default = "default_fg")]
    pub foreground: String,
    #[serde(default = "default_dark_fg")]
    pub dark_foreground: String,
    #[serde(default = "default_light_fg")]
    pub light_foreground: String,
    #[serde(default = "default_bright_fg")]
    pub bright_foreground: String,

    #[serde(default = "default_red")]
    pub red: String,
    #[serde(default = "default_green")]
    pub green: String,
    #[serde(default = "default_yellow")]
    pub yellow: String,
    #[serde(default = "default_blue")]
    pub blue: String,
    #[serde(default = "default_cyan")]
    pub cyan: String,
    #[serde(default = "default_magenta")]
    pub magenta: String,

    #[serde(default = "default_bright_red")]
    pub bright_red: String,
    #[serde(default = "default_bright_green")]
    pub bright_green: String,
    #[serde(default = "default_bright_yellow")]
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
}

#[allow(dead_code)]
impl Theme {
    pub fn load() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let state_theme_file = home.join(".local/state/omarchy/current/theme/colors.toml");

        if state_theme_file.exists() {
            let colors = fs::read_to_string(&state_theme_file)
                .ok()
                .and_then(|content| toml::from_str::<ThemeColors>(&content).ok())
                .unwrap_or_default();

            Self {
                has_desktop_theme: true,
                colors,
            }
        } else {
            Self {
                has_desktop_theme: false,
                colors: ThemeColors::default(),
            }
        }
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
