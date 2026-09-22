//! Preferences remembered between runs, stored as `key=value` lines.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chess::Color;

use crate::clock::{self, TimeControl};
use crate::engine::Level;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Theme name; an unknown name falls back to the first theme when used.
    pub theme: String,
    /// Your colour against the engine; the bottom side in two-player mode.
    pub side: Color,
    pub level: Level,
    pub two_player: bool,
    pub clock: Option<TimeControl>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "Wood".to_string(),
            side: Color::White,
            level: Level::DEFAULT,
            two_player: false,
            clock: None,
        }
    }
}

/// `<config dir>/TuiChess/settings.txt`, if the platform has a config directory.
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("TuiChess").join("settings.txt"))
}

impl Settings {
    /// Missing or unreadable file: defaults.
    pub fn load(path: &Path) -> Settings {
        fs::read_to_string(path)
            .map(|text| Settings::parse(&text))
            .unwrap_or_default()
    }

    /// Unknown keys are ignored; an invalid value keeps that key's default.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "theme" if !value.is_empty() => s.theme = value.to_string(),
                "side" => match value {
                    "white" => s.side = Color::White,
                    "black" => s.side = Color::Black,
                    _ => {}
                },
                "level" => {
                    if let Ok(n) = value.parse::<u8>()
                        && (Level::MIN.get()..=Level::MAX.get()).contains(&n)
                    {
                        s.level = Level::new(n);
                    }
                }
                "mode" => match value {
                    "engine" => s.two_player = false,
                    "two-player" => s.two_player = true,
                    _ => {}
                },
                "clock" => {
                    if let Some(preset) = clock::parse_preset(value) {
                        s.clock = preset;
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let side = match self.side {
            Color::White => "white",
            Color::Black => "black",
        };
        let mode = if self.two_player {
            "two-player"
        } else {
            "engine"
        };
        format!(
            "theme={}\nside={side}\nlevel={}\nmode={mode}\nclock={}\n",
            self.theme,
            self.level.get(),
            clock::preset_name(self.clock)
        )
    }

    /// Writes via a temp file and rename, so a crash never leaves a half-written file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&tmp, self.to_text())?;
        fs::rename(&tmp, path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            theme: "Ocean".to_string(),
            side: Color::Black,
            level: Level::new(7),
            two_player: true,
            clock: clock::PRESETS[3],
        };
        assert_eq!(Settings::parse(&s.to_text()), s);

        let dir = std::env::temp_dir().join(format!("tuichess-settings-{}", std::process::id()));
        let path = dir.join("settings.txt");
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_values_fall_back_per_key() {
        let s = Settings::parse(
            "theme=Forest\nside=purple\nlevel=99\nmode=chaos\nclock=7+7\nfoo=bar\ngarbage line\n",
        );
        let d = Settings::default();
        assert_eq!(s.theme, "Forest");
        assert_eq!(s.side, d.side);
        assert_eq!(s.level, d.level);
        assert_eq!(s.two_player, d.two_player);
        assert_eq!(s.clock, d.clock);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let path = std::env::temp_dir().join("tuichess-definitely-missing/settings.txt");
        assert_eq!(Settings::load(&path), Settings::default());
    }
}
