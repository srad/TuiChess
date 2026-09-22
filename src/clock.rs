//! Time controls and the chess clock. Every method takes `now`, so tests use synthetic instants.

use std::time::{Duration, Instant};

use chess::Color;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimeControl {
    pub base: Duration,
    pub increment: Duration,
}

impl TimeControl {
    const fn new(minutes: u64, increment_secs: u64) -> TimeControl {
        TimeControl {
            base: Duration::from_secs(minutes * 60),
            increment: Duration::from_secs(increment_secs),
        }
    }

    pub fn name(self) -> String {
        format!("{}+{}", self.base.as_secs() / 60, self.increment.as_secs())
    }
}

/// Selectable clock settings; `None` means no clock.
pub const PRESETS: [Option<TimeControl>; 6] = [
    None,
    Some(TimeControl::new(1, 0)),
    Some(TimeControl::new(3, 2)),
    Some(TimeControl::new(5, 3)),
    Some(TimeControl::new(10, 5)),
    Some(TimeControl::new(15, 10)),
];

pub fn preset_name(preset: Option<TimeControl>) -> String {
    preset.map_or_else(|| "off".to_string(), TimeControl::name)
}

pub fn parse_preset(name: &str) -> Option<Option<TimeControl>> {
    PRESETS.into_iter().find(|&p| preset_name(p) == name)
}

/// The preset after `current`, wrapping around.
pub fn next_preset(current: Option<TimeControl>) -> Option<TimeControl> {
    let i = PRESETS.iter().position(|&p| p == current).unwrap_or(0);
    PRESETS[(i + 1) % PRESETS.len()]
}

/// Both sides' remaining time, as sent to an engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClockTimes {
    pub white: Duration,
    pub black: Duration,
    pub increment: Duration,
}

impl ClockTimes {
    pub fn for_side(&self, color: Color) -> Duration {
        match color {
            Color::White => self.white,
            Color::Black => self.black,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Clock {
    remaining: [Duration; 2],
    increment: Duration,
    /// The side whose time is running, and since when. Nothing runs before the first move.
    running: Option<(Color, Instant)>,
}

impl Clock {
    pub fn new(tc: TimeControl) -> Clock {
        Clock {
            remaining: [tc.base; 2],
            increment: tc.increment,
            running: None,
        }
    }

    pub fn running(&self) -> Option<Color> {
        self.running.map(|(color, _)| color)
    }

    pub fn remaining(&self, color: Color, now: Instant) -> Duration {
        let stored = self.remaining[color.to_index()];
        match self.running {
            Some((c, since)) if c == color => {
                stored.saturating_sub(now.saturating_duration_since(since))
            }
            _ => stored,
        }
    }

    /// `mover` just moved: charge its thinking time, add the increment, start the opponent.
    pub fn on_move(&mut self, mover: Color, now: Instant) {
        let left = self.remaining(mover, now);
        self.remaining[mover.to_index()] = left + self.increment;
        self.running = Some((!mover, now));
    }

    /// A move was taken back: the running side's current thinking time is discarded (neither
    /// charged nor refunded), then `to_move`'s time runs; `None` leaves the clock stopped.
    pub fn on_undo(&mut self, to_move: Option<Color>, now: Instant) {
        self.running = to_move.map(|color| (color, now));
    }

    /// Charges the running side and stops the clock.
    pub fn stop(&mut self, now: Instant) {
        if let Some((color, _)) = self.running {
            self.remaining[color.to_index()] = self.remaining(color, now);
        }
        self.running = None;
    }

    /// The side whose time has run out, if any. Only the running side can flag.
    pub fn flagged(&self, now: Instant) -> Option<Color> {
        self.running()
            .filter(|&color| self.remaining(color, now).is_zero())
    }

    pub fn times(&self, now: Instant) -> ClockTimes {
        ClockTimes {
            white: self.remaining(Color::White, now),
            black: self.remaining(Color::Black, now),
            increment: self.increment,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREE_TWO: TimeControl = TimeControl::new(3, 2);

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn first_move_is_untimed_and_gets_increment() {
        let t0 = Instant::now();
        let mut clock = Clock::new(THREE_TWO);
        assert_eq!(clock.running(), None);
        assert_eq!(clock.remaining(Color::White, t0 + secs(100)), secs(180));
        clock.on_move(Color::White, t0 + secs(100));
        assert_eq!(clock.remaining(Color::White, t0 + secs(100)), secs(182));
        assert_eq!(clock.running(), Some(Color::Black));
    }

    #[test]
    fn thinking_time_is_charged_to_the_mover() {
        let t0 = Instant::now();
        let mut clock = Clock::new(THREE_TWO);
        clock.on_move(Color::White, t0);
        clock.on_move(Color::Black, t0 + secs(10));
        assert_eq!(
            clock.remaining(Color::Black, t0 + secs(10)),
            secs(180 - 10 + 2)
        );
        assert_eq!(clock.remaining(Color::White, t0 + secs(15)), secs(182 - 5));
    }

    #[test]
    fn undo_discards_running_time() {
        let t0 = Instant::now();
        let mut clock = Clock::new(THREE_TWO);
        clock.on_move(Color::White, t0);
        clock.on_undo(Some(Color::White), t0 + secs(30)); // Black thought 30 s, then takeback
        assert_eq!(clock.remaining(Color::Black, t0 + secs(30)), secs(180));
        assert_eq!(clock.remaining(Color::White, t0 + secs(40)), secs(182 - 10));
        clock.on_undo(None, t0 + secs(50));
        assert_eq!(clock.running(), None);
        assert_eq!(clock.remaining(Color::White, t0 + secs(99)), secs(182));
    }

    #[test]
    fn flag_and_stop() {
        let t0 = Instant::now();
        let mut clock = Clock::new(TimeControl::new(1, 0));
        clock.on_move(Color::White, t0);
        assert_eq!(clock.flagged(t0 + secs(59)), None);
        assert_eq!(clock.flagged(t0 + secs(60)), Some(Color::Black));
        clock.stop(t0 + secs(30));
        assert_eq!(clock.flagged(t0 + secs(999)), None);
        assert_eq!(clock.remaining(Color::Black, t0 + secs(999)), secs(30));
    }

    #[test]
    fn times_and_presets() {
        let t0 = Instant::now();
        let mut clock = Clock::new(THREE_TWO);
        clock.on_move(Color::White, t0);
        let times = clock.times(t0 + secs(20));
        assert_eq!(times.white, secs(182));
        assert_eq!(times.black, secs(160));
        assert_eq!(times.for_side(Color::Black), secs(160));
        assert_eq!(times.increment, secs(2));

        assert_eq!(parse_preset("3+2"), Some(Some(THREE_TWO)));
        assert_eq!(parse_preset("off"), Some(None));
        assert_eq!(parse_preset("7+7"), None);
        assert_eq!(next_preset(None), PRESETS[1]);
        assert_eq!(next_preset(PRESETS[5]), None);
    }
}
