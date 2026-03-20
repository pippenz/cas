//! Base color definitions for CAS themes

use ratatui::style::Color;

/// Base color palette - inspired by Linear/Raycast muted aesthetics
#[derive(Debug, Clone)]
pub struct ColorPalette {
    // Grayscale
    pub gray_50: Color,
    pub gray_100: Color,
    pub gray_200: Color,
    pub gray_300: Color,
    pub gray_400: Color,
    pub gray_500: Color,
    pub gray_600: Color,
    pub gray_700: Color,
    pub gray_800: Color,
    pub gray_900: Color,

    // Primary accent (teal - ops console)
    pub primary_100: Color,
    pub primary_200: Color,
    pub primary_300: Color,
    pub primary_400: Color,
    pub primary_500: Color,

    // Status colors
    pub success: Color,
    pub success_dim: Color,
    pub warning: Color,
    pub warning_dim: Color,
    pub error: Color,
    pub error_dim: Color,
    pub info: Color,
    pub info_dim: Color,

    // Specialty colors
    pub purple: Color,
    pub purple_dim: Color,
    pub cyan: Color,
    pub cyan_dim: Color,
    pub orange: Color,
    pub orange_dim: Color,
}

impl ColorPalette {
    /// Linear-inspired dark theme
    pub fn dark() -> Self {
        Self {
            // Modern muted grays (slightly blue-tinted)
            gray_50: Color::Rgb(250, 250, 252),
            gray_100: Color::Rgb(228, 229, 235),
            gray_200: Color::Rgb(198, 200, 210),
            gray_300: Color::Rgb(160, 162, 175),
            gray_400: Color::Rgb(120, 123, 138),
            gray_500: Color::Rgb(90, 93, 107),
            gray_600: Color::Rgb(60, 63, 75),
            gray_700: Color::Rgb(40, 43, 53),
            gray_800: Color::Rgb(28, 30, 38),
            gray_900: Color::Rgb(18, 19, 24),

            // Teal accent (ops console)
            primary_100: Color::Rgb(160, 240, 230),
            primary_200: Color::Rgb(120, 220, 210),
            primary_300: Color::Rgb(80, 200, 190),
            primary_400: Color::Rgb(40, 180, 170),
            primary_500: Color::Rgb(30, 140, 135),

            // Status - muted for dark theme
            success: Color::Rgb(80, 200, 120),
            success_dim: Color::Rgb(40, 100, 60),
            warning: Color::Rgb(240, 160, 60),
            warning_dim: Color::Rgb(120, 80, 30),
            error: Color::Rgb(230, 90, 90),
            error_dim: Color::Rgb(115, 45, 45),
            info: Color::Rgb(70, 170, 230),
            info_dim: Color::Rgb(35, 85, 115),

            // Specialty colors
            purple: Color::Rgb(180, 130, 255),
            purple_dim: Color::Rgb(90, 65, 128),
            cyan: Color::Rgb(80, 200, 210),
            cyan_dim: Color::Rgb(40, 100, 105),
            orange: Color::Rgb(230, 150, 80),
            orange_dim: Color::Rgb(115, 75, 40),
        }
    }

    /// Light theme variant
    pub fn light() -> Self {
        Self {
            // Inverted grays for light background
            gray_50: Color::Rgb(18, 19, 24),
            gray_100: Color::Rgb(28, 30, 38),
            gray_200: Color::Rgb(40, 43, 53),
            gray_300: Color::Rgb(60, 63, 75),
            gray_400: Color::Rgb(90, 93, 107),
            gray_500: Color::Rgb(120, 123, 138),
            gray_600: Color::Rgb(160, 162, 175),
            gray_700: Color::Rgb(198, 200, 210),
            gray_800: Color::Rgb(228, 229, 235),
            gray_900: Color::Rgb(250, 250, 252),

            // Same accent colors (teal)
            primary_100: Color::Rgb(160, 240, 230),
            primary_200: Color::Rgb(120, 220, 210),
            primary_300: Color::Rgb(80, 200, 190),
            primary_400: Color::Rgb(40, 180, 170),
            primary_500: Color::Rgb(30, 140, 135),

            // More saturated for light background
            success: Color::Rgb(40, 160, 80),
            success_dim: Color::Rgb(200, 240, 210),
            warning: Color::Rgb(210, 140, 50),
            warning_dim: Color::Rgb(255, 245, 200),
            error: Color::Rgb(200, 60, 60),
            error_dim: Color::Rgb(255, 220, 220),
            info: Color::Rgb(50, 150, 210),
            info_dim: Color::Rgb(220, 235, 255),

            purple: Color::Rgb(130, 80, 200),
            purple_dim: Color::Rgb(240, 230, 255),
            cyan: Color::Rgb(40, 160, 170),
            cyan_dim: Color::Rgb(220, 250, 252),
            orange: Color::Rgb(200, 120, 50),
            orange_dim: Color::Rgb(255, 240, 220),
        }
    }

    /// Minions theme variant - yellow primary, denim blue secondary
    pub fn minions(is_dark: bool) -> Self {
        let base = if is_dark { Self::dark() } else { Self::light() };
        Self {
            // Override primary accent from teal to Minion yellow
            primary_100: Color::Rgb(255, 245, 157), // Light banana
            primary_200: Color::Rgb(255, 235, 59),  // Bright yellow
            primary_300: Color::Rgb(255, 213, 0),   // Minion yellow
            primary_400: Color::Rgb(255, 193, 7),   // Amber accent
            primary_500: Color::Rgb(255, 160, 0),   // Deep amber

            // Override info to denim blue (overalls)
            info: Color::Rgb(65, 105, 225),     // Royal blue / denim
            info_dim: Color::Rgb(33, 53, 113),  // Dark denim

            // Override cyan to goggle silver
            cyan: Color::Rgb(192, 200, 210),    // Goggle silver
            cyan_dim: Color::Rgb(96, 100, 105), // Dark goggle

            // Keep everything else from the base
            ..base
        }
    }

    /// High contrast accessibility variant
    pub fn high_contrast() -> Self {
        Self {
            gray_50: Color::White,
            gray_100: Color::White,
            gray_200: Color::Rgb(220, 220, 220),
            gray_300: Color::Rgb(180, 180, 180),
            gray_400: Color::Rgb(140, 140, 140),
            gray_500: Color::Rgb(100, 100, 100),
            gray_600: Color::Rgb(60, 60, 60),
            gray_700: Color::Rgb(40, 40, 40),
            gray_800: Color::Rgb(20, 20, 20),
            gray_900: Color::Black,

            primary_100: Color::Rgb(255, 255, 100),
            primary_200: Color::Rgb(255, 255, 80),
            primary_300: Color::Rgb(255, 255, 60),
            primary_400: Color::Yellow,
            primary_500: Color::Rgb(200, 200, 0),

            success: Color::Rgb(0, 255, 0),
            success_dim: Color::Rgb(0, 100, 0),
            warning: Color::Rgb(255, 255, 0),
            warning_dim: Color::Rgb(100, 100, 0),
            error: Color::Rgb(255, 50, 50),
            error_dim: Color::Rgb(100, 0, 0),
            info: Color::Rgb(0, 200, 255),
            info_dim: Color::Rgb(0, 80, 100),

            purple: Color::Magenta,
            purple_dim: Color::Rgb(100, 0, 100),
            cyan: Color::Cyan,
            cyan_dim: Color::Rgb(0, 100, 100),
            orange: Color::Rgb(255, 165, 0),
            orange_dim: Color::Rgb(100, 65, 0),
        }
    }
}
