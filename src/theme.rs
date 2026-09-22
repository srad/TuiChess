//! Colour schemes in the 16-colour VGA palette (blue slightly adjusted), for the MS-DOS look.

use ratatui::style::Color;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

pub const BLACK: Color = rgb(0x00, 0x00, 0x00);
/// The desktop blue of the reference screenshots, a little lighter than VGA's 0000AA.
pub const BLUE: Color = rgb(0x1A, 0x22, 0xC5);
pub const GREEN: Color = rgb(0x00, 0xAA, 0x00);
pub const CYAN: Color = rgb(0x00, 0xAA, 0xAA);
pub const RED: Color = rgb(0xAA, 0x00, 0x00);
pub const MAGENTA: Color = rgb(0xAA, 0x00, 0xAA);
pub const BROWN: Color = rgb(0xAA, 0x55, 0x00);
pub const LIGHT_GRAY: Color = rgb(0xAA, 0xAA, 0xAA);
pub const DARK_GRAY: Color = rgb(0x55, 0x55, 0x55);
pub const LIGHT_BLUE: Color = rgb(0x55, 0x55, 0xFF);
pub const LIGHT_CYAN: Color = rgb(0x55, 0xFF, 0xFF);
pub const LIGHT_RED: Color = rgb(0xFF, 0x55, 0x55);
pub const YELLOW: Color = rgb(0xFF, 0xFF, 0x55);
pub const WHITE: Color = rgb(0xFF, 0xFF, 0xFF);

/// Highlighted menu item and focused button, in every theme.
pub const SELECT_BG: Color = BLACK;
pub const SELECT_FG: Color = WHITE;
/// Drop shadow under dialogs and menus: the characters stay, dimmed.
pub const SHADOW_BG: Color = BLACK;
pub const SHADOW_FG: Color = DARK_GRAY;

/// Everything around the board.
pub struct Chrome {
    pub desktop: Color,
    pub text: Color,
    /// Values that should stand out, like the game status.
    pub value: Color,
    /// Box titles.
    pub title: Color,
    pub border: Color,
    /// Check and flagged clocks.
    pub alert: Color,
    pub dim: Color,
    /// Menu bar, status bar, dropdowns and buttons.
    pub bar: Color,
    pub bar_text: Color,
    /// Key letters on the bars, and notices on the status bar.
    pub hotkey: Color,
    pub dialog: Color,
    pub dialog_text: Color,
    pub dialog_title: Color,
}

/// Square colours are mid-tones so that both white and black pieces stay readable.
pub struct BoardColors {
    pub light: Color,
    pub dark: Color,
    pub last_light: Color,
    pub last_dark: Color,
    pub select: Color,
    pub check: Color,
    pub capture: Color,
    pub hint: Color,
    pub target: Color,
    pub cursor: Color,
    pub white_piece: Color,
    pub black_piece: Color,
}

pub struct Theme {
    pub name: &'static str,
    pub chrome: Chrome,
    pub board: BoardColors,
}

pub const THEMES: [Theme; 3] = [
    Theme {
        name: "Blue",
        chrome: Chrome {
            desktop: BLUE,
            text: WHITE,
            value: YELLOW,
            title: YELLOW,
            border: LIGHT_GRAY,
            alert: LIGHT_RED,
            dim: LIGHT_GRAY,
            bar: LIGHT_GRAY,
            bar_text: BLACK,
            hotkey: RED,
            dialog: LIGHT_CYAN,
            dialog_text: BLACK,
            dialog_title: BLUE,
        },
        board: BoardColors {
            light: CYAN,
            dark: LIGHT_BLUE,
            last_light: GREEN,
            last_dark: BROWN,
            select: MAGENTA,
            check: LIGHT_RED,
            capture: RED,
            hint: LIGHT_GRAY,
            target: BLACK,
            cursor: YELLOW,
            white_piece: WHITE,
            black_piece: BLACK,
        },
    },
    Theme {
        name: "Black",
        chrome: Chrome {
            desktop: BLACK,
            text: LIGHT_GRAY,
            value: YELLOW,
            title: YELLOW,
            border: CYAN,
            alert: LIGHT_RED,
            dim: DARK_GRAY,
            bar: CYAN,
            bar_text: BLACK,
            hotkey: BLUE,
            dialog: LIGHT_GRAY,
            dialog_text: BLACK,
            dialog_title: BLUE,
        },
        board: BoardColors {
            light: LIGHT_GRAY,
            dark: BROWN,
            last_light: CYAN,
            last_dark: LIGHT_BLUE,
            select: GREEN,
            check: LIGHT_RED,
            capture: RED,
            hint: MAGENTA,
            target: BLACK,
            cursor: YELLOW,
            white_piece: WHITE,
            black_piece: BLACK,
        },
    },
    // Grayscale chrome; the board keeps colours because its highlights carry meaning.
    Theme {
        name: "Mono",
        chrome: Chrome {
            desktop: BLACK,
            text: LIGHT_GRAY,
            value: WHITE,
            title: WHITE,
            border: LIGHT_GRAY,
            alert: WHITE,
            dim: DARK_GRAY,
            bar: LIGHT_GRAY,
            bar_text: BLACK,
            hotkey: WHITE,
            dialog: LIGHT_GRAY,
            dialog_text: BLACK,
            dialog_title: BLACK,
        },
        board: BoardColors {
            light: LIGHT_GRAY,
            dark: DARK_GRAY,
            last_light: CYAN,
            last_dark: BROWN,
            select: GREEN,
            check: LIGHT_RED,
            capture: RED,
            hint: MAGENTA,
            target: WHITE,
            cursor: YELLOW,
            white_piece: WHITE,
            black_piece: BLACK,
        },
    },
];

/// Index of the theme called `name`, or the first theme.
pub fn theme_index(name: &str) -> usize {
    THEMES.iter().position(|t| t.name == name).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of an RGB colour.
    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("themes use RGB colours only");
        };
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    /// WCAG contrast ratio; the order of the colours does not matter.
    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn both_piece_colours_stay_readable_on_every_square() {
        for theme in &THEMES {
            let b = &theme.board;
            let squares = [
                b.light,
                b.dark,
                b.last_light,
                b.last_dark,
                b.select,
                b.check,
                b.capture,
                b.hint,
            ];
            for bg in squares {
                for piece in [b.white_piece, b.black_piece] {
                    let ratio = contrast(bg, piece);
                    assert!(
                        ratio >= 2.0,
                        "{}: {bg:?} vs {piece:?} is {ratio:.2}",
                        theme.name
                    );
                }
            }
            for highlight in &squares[2..] {
                assert!(
                    *highlight != b.light && *highlight != b.dark,
                    "{}: {highlight:?} hides among the plain squares",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn theme_lookup_by_name() {
        assert_eq!(theme_index("Black"), 1);
        assert_eq!(theme_index("Ocean"), 0);
    }
}
