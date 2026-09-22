//! The menu bar's contents. Drawing lives in `ui`, input handling in `app`.

use crate::clock::{self, TimeControl};
use crate::engine::{EngineChoice, Level};
use crate::settings::Settings;
use crate::theme::THEMES;

/// Everything a menu item can do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Restart,
    SwapSides,
    Undo,
    Hint,
    OfferDraw,
    Resign,
    Flip,
    Quit,
    Mode { two_player: bool },
    Level(Level),
    Clock(Option<TimeControl>),
    Engine(EngineChoice),
    Theme(usize),
    Keys,
    About,
}

pub struct Item {
    pub label: String,
    /// Chooses the item while its menu is open; its first occurrence in `label` is highlighted.
    pub key: Option<char>,
    /// The key that does the same with the menu closed, shown on the right.
    pub shortcut: Option<&'static str>,
    /// Draw a separator line above this item.
    pub separator_before: bool,
    /// `Some` for radio items: whether this one is the current choice.
    pub checked: Option<bool>,
    pub command: Command,
}

impl Item {
    fn action(label: &str, key: char, shortcut: &'static str, command: Command) -> Item {
        Item {
            label: label.to_string(),
            key: Some(key),
            shortcut: Some(shortcut),
            separator_before: false,
            checked: None,
            command,
        }
    }

    fn radio(label: String, key: Option<char>, checked: bool, command: Command) -> Item {
        Item {
            label,
            key,
            shortcut: None,
            separator_before: false,
            checked: Some(checked),
            command,
        }
    }

    fn after_separator(self) -> Item {
        Item {
            separator_before: true,
            ..self
        }
    }
}

pub struct Menu {
    pub title: &'static str,
    /// Opens the menu together with Alt; highlighted in `title`.
    pub key: char,
    pub items: Vec<Item>,
}

impl Menu {
    /// The item to highlight when the menu opens: the current choice in a menu of choices,
    /// else the first.
    pub fn initial_item(&self) -> usize {
        if !self.items.iter().all(|item| item.checked.is_some()) {
            return 0;
        }
        self.items
            .iter()
            .position(|item| item.checked == Some(true))
            .unwrap_or(0)
    }

    pub fn item_for_key(&self, key: char) -> Option<usize> {
        let key = key.to_ascii_lowercase();
        self.items.iter().position(|item| item.key == Some(key))
    }
}

/// What the menus show.
pub struct Context<'a> {
    pub settings: &'a Settings,
    /// Whether the running engine is a UCI engine, which may differ from the setting.
    pub uci: bool,
    pub engine_name: &'a str,
}

/// First letter of `label` not in `used`, which then includes it.
fn free_key(label: &str, used: &mut Vec<char>) -> Option<char> {
    let key = label
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .find(|c| c.is_ascii_alphabetic() && !used.contains(c))?;
    used.push(key);
    Some(key)
}

pub fn build(ctx: Context) -> Vec<Menu> {
    let s = ctx.settings;

    let game = vec![
        Item::action("Restart", 'r', "R", Command::Restart),
        Item::action("New game, swap sides", 'n', "N", Command::SwapSides),
        Item::action("Undo", 'u', "U", Command::Undo),
        Item::action("Suggest move", 's', "S", Command::Hint),
        Item::action("Offer draw", 'd', "D", Command::OfferDraw),
        Item::action("Resign", 'x', "X", Command::Resign),
        Item::action("Flip board", 'f', "F", Command::Flip),
        Item::radio(
            "Versus computer".to_string(),
            Some('v'),
            !s.two_player,
            Command::Mode { two_player: false },
        )
        .after_separator(),
        Item::radio(
            "Two players".to_string(),
            Some('p'),
            s.two_player,
            Command::Mode { two_player: true },
        ),
        Item::action("Quit", 'q', "Q", Command::Quit).after_separator(),
    ];

    let level = (Level::MIN.get()..=Level::MAX.get())
        .map(|n| {
            let level = Level::new(n);
            let label = match (ctx.uci, level.elo()) {
                (true, Some(elo)) => format!("Level {n}  ~{elo} Elo"),
                (true, None) => format!("Level {n}  Full strength"),
                (false, _) => format!("Level {n}"),
            };
            let key = char::from_digit(u32::from(n), 10);
            Item::radio(label, key, s.level == level, Command::Level(level))
        })
        .collect();

    let clock = clock::PRESETS
        .into_iter()
        .map(|preset| {
            let label = match preset {
                None => "No clock".to_string(),
                Some(tc) => tc.name(),
            };
            Item::radio(label, None, s.clock == preset, Command::Clock(preset))
        })
        .collect();

    let mut used = Vec::new();
    let stockfish = if ctx.uci {
        ctx.engine_name
    } else {
        "Stockfish"
    };
    let engine = [
        (stockfish, ctx.uci, EngineChoice::Stockfish),
        ("Built-in", !ctx.uci, EngineChoice::Builtin),
    ]
    .into_iter()
    .map(|(label, checked, choice)| {
        let key = free_key(label, &mut used);
        Item::radio(label.to_string(), key, checked, Command::Engine(choice))
    })
    .collect();

    let mut used = Vec::new();
    let current = crate::theme::theme_index(&s.theme);
    let theme = THEMES
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let key = free_key(t.name, &mut used);
            Item::radio(t.name.to_string(), key, i == current, Command::Theme(i))
        })
        .collect();

    let help = vec![
        Item::action("Keys", 'k', "F1", Command::Keys),
        Item {
            label: "About".to_string(),
            key: Some('a'),
            shortcut: None,
            separator_before: false,
            checked: None,
            command: Command::About,
        },
    ];

    vec![
        Menu {
            title: "Game",
            key: 'g',
            items: game,
        },
        Menu {
            title: "Level",
            key: 'l',
            items: level,
        },
        Menu {
            title: "Clock",
            key: 'c',
            items: clock,
        },
        Menu {
            title: "Engine",
            key: 'e',
            items: engine,
        },
        Menu {
            title: "Theme",
            key: 't',
            items: theme,
        },
        Menu {
            title: "Help",
            key: 'h',
            items: help,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menus(settings: &Settings, uci: bool) -> Vec<Menu> {
        build(Context {
            settings,
            uci,
            engine_name: "Stockfish 19",
        })
    }

    #[test]
    fn keys_are_unique_and_in_labels() {
        for menu in menus(&Settings::default(), true) {
            assert!(menu.title.to_ascii_lowercase().contains(menu.key));
            let keys: Vec<char> = menu.items.iter().filter_map(|i| i.key).collect();
            for (i, key) in keys.iter().enumerate() {
                assert!(!keys[i + 1..].contains(key), "{}: {key} twice", menu.title);
            }
        }
    }

    #[test]
    fn radio_marks_follow_settings_and_engine() {
        let settings = Settings {
            level: Level::new(8),
            two_player: true,
            ..Settings::default()
        };
        fn checked(menu: &Menu) -> Vec<&str> {
            menu.items
                .iter()
                .filter(|i| i.checked == Some(true))
                .map(|i| i.label.as_str())
                .collect()
        }
        let m = menus(&settings, false);
        assert_eq!(checked(&m[0]), ["Two players"]);
        assert_eq!(checked(&m[1]), ["Level 8"]);
        assert_eq!(checked(&m[2]), ["No clock"]);
        assert_eq!(
            checked(&m[3]),
            ["Built-in"],
            "the running engine, not the setting"
        );
        assert_eq!(checked(&m[4]), ["Blue"]);
        assert_eq!(m[1].initial_item(), 7);
        assert_eq!(m[0].initial_item(), 0, "Game mixes actions and choices");

        let m = menus(&Settings::default(), true);
        assert_eq!(m[1].items[0].label, "Level 1  ~1320 Elo");
        assert_eq!(m[1].items[7].label, "Level 8  Full strength");
        assert_eq!(checked(&m[3]), ["Stockfish 19"]);
        assert_eq!(m[5].item_for_key('A'), Some(1));
    }
}
