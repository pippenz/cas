//! Theme configuration and active theme management

use serde::{Deserialize, Serialize};

use crate::ui::theme::colors::ColorPalette;
use crate::ui::theme::palette::Palette;
use crate::ui::theme::styles::Styles;

/// Theme mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    HighContrast,
}

/// Theme variant selection (cosmetic flavor)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeVariant {
    #[default]
    Default,
    Minions,
}

impl std::fmt::Display for ThemeVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeVariant::Default => write!(f, "default"),
            ThemeVariant::Minions => write!(f, "minions"),
        }
    }
}

impl std::str::FromStr for ThemeVariant {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(ThemeVariant::Default),
            "minions" => Ok(ThemeVariant::Minions),
            _ => Err(format!("Unknown theme variant: {s}")),
        }
    }
}

impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThemeMode::Dark => write!(f, "dark"),
            ThemeMode::Light => write!(f, "light"),
            ThemeMode::HighContrast => write!(f, "high_contrast"),
        }
    }
}

impl std::str::FromStr for ThemeMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dark" => Ok(ThemeMode::Dark),
            "light" => Ok(ThemeMode::Light),
            "high_contrast" | "highcontrast" | "high-contrast" => Ok(ThemeMode::HighContrast),
            _ => Err(format!("Unknown theme mode: {s}")),
        }
    }
}

/// Theme configuration stored in .cas/config.toml
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Theme mode: dark, light, or high_contrast
    #[serde(default)]
    pub mode: ThemeMode,

    /// Theme variant: default or minions
    #[serde(default)]
    pub variant: ThemeVariant,
}

/// Active theme instance with computed styles
#[derive(Debug, Clone)]
pub struct ActiveTheme {
    pub mode: ThemeMode,
    pub variant: ThemeVariant,
    pub is_dark: bool,
    pub palette: Palette,
    pub styles: Styles,
}

impl ActiveTheme {
    /// Create theme from configuration
    pub fn from_config(config: &ThemeConfig) -> Self {
        Self::from_mode_and_variant(config.mode, config.variant)
    }

    /// Create theme from mode (default variant)
    pub fn from_mode(mode: ThemeMode) -> Self {
        Self::from_mode_and_variant(mode, ThemeVariant::Default)
    }

    /// Create theme from mode and variant
    pub fn from_mode_and_variant(mode: ThemeMode, variant: ThemeVariant) -> Self {
        let (base_colors, is_dark) = match mode {
            ThemeMode::Dark => (ColorPalette::dark(), true),
            ThemeMode::Light => (ColorPalette::light(), false),
            ThemeMode::HighContrast => (ColorPalette::high_contrast(), true),
        };

        let colors = match variant {
            ThemeVariant::Default => base_colors,
            ThemeVariant::Minions => ColorPalette::minions(is_dark),
        };

        let palette = Palette::from_colors(colors, is_dark);
        let styles = Styles::from_palette(&palette);

        Self {
            mode,
            variant,
            is_dark,
            palette,
            styles,
        }
    }

    /// Create default dark theme
    pub fn default_dark() -> Self {
        Self::from_mode(ThemeMode::Dark)
    }

    /// Create default light theme
    pub fn default_light() -> Self {
        Self::from_mode(ThemeMode::Light)
    }

    /// Create high contrast theme
    pub fn high_contrast() -> Self {
        Self::from_mode(ThemeMode::HighContrast)
    }

    /// Auto-detect the terminal background and create the appropriate theme.
    ///
    /// Uses OSC 11 terminal query, COLORFGBG env var, or defaults to dark.
    pub fn detect() -> Self {
        let mode = super::detect::detect_background_theme();
        Self::from_mode(mode)
    }

    /// Resolve the theme from optional config, falling back to auto-detection.
    ///
    /// When config is `Some`, the explicit mode is used (no detection).
    /// When config is `None`, auto-detection runs (OSC 11 → COLORFGBG → dark).
    pub fn resolve(config: Option<&ThemeConfig>) -> Self {
        match config {
            Some(cfg) => Self::from_config(cfg),
            None => Self::detect(),
        }
    }

    /// Check if the minions variant is active
    pub fn is_minions(&self) -> bool {
        self.variant == ThemeVariant::Minions
    }
}

impl Default for ActiveTheme {
    fn default() -> Self {
        Self::detect()
    }
}
