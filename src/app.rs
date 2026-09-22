//! Application state and input handling, independent of the terminal. Every method takes
//! `now`, so the logic is testable with synthetic instants.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use chess::{Board, ChessMove, Color, Piece, Square};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::clock::{self, TimeControl};
use crate::engine::{Engine, Eval, Level, SearchEvent, SearchLimit};
use crate::game::{EngineMove, Game, GameSetup, Phase, StartPosition};
use crate::settings::Settings;
use crate::ui;

const NOTICE_TIME: Duration = Duration::from_secs(4);
/// Thinking time per move without a clock, and for hints.
const MOVE_TIME: Duration = Duration::from_secs(1);
/// Extra time before an unanswered search counts as a hung engine.
const TIMEOUT_GRACE: Duration = Duration::from_secs(5);
/// A second engine restart within this window falls back to the built-in engine.
const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// The engine accepts a draw when it is at most this much better (centipawns).
const DRAW_ACCEPT_CP: i32 = 50;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    Continue,
    Quit,
}

/// Actions that may ask for a second key press first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Resign,
    OfferDraw,
    Restart,
    NewGameSwap,
    SwitchMode,
}

impl Action {
    fn prompt(self) -> &'static str {
        match self {
            Action::Resign => "Press x again to resign",
            Action::OfferDraw => "Press d again to offer a draw",
            Action::Restart => "Press r again to restart",
            Action::NewGameSwap => "Press n again for a new game",
            Action::SwitchMode => "Press m again to switch mode",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Purpose {
    EngineMove,
    Hint,
}

/// A search in flight. Only valid while `version` matches the app's.
struct Pending {
    id: u64,
    version: u64,
    purpose: Purpose,
    rx: Receiver<SearchEvent>,
    deadline: Instant,
}

struct Notice {
    text: String,
    /// `None`: stays until the user acts.
    until: Option<Instant>,
}

/// Where a mouse drag started, and whether that square was already selected before.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Drag {
    from: Option<Square>,
    was_selected: bool,
}

pub struct App {
    pub game: Game,
    engine: Engine,
    settings: Settings,
    settings_path: Option<PathBuf>,
    /// Session start position (standard or `--fen`); every new game starts here.
    start: StartPosition,
    /// The current game's clock setting, to show when the next game's differs.
    game_clock: Option<TimeControl>,
    pending: Option<Pending>,
    next_id: u64,
    /// Bumped on every change to the game; results of older searches are ignored.
    version: u64,
    browse: Option<usize>,
    drag: Drag,
    confirm: Option<(Action, Instant)>,
    notice: Option<Notice>,
    engine_failed: bool,
    last_restart: Option<Instant>,
    help_open: bool,
    /// The engine's expected line and the position it starts from.
    engine_line: Option<(Board, Vec<ChessMove>)>,
    tick: u64,
}

fn promotion_piece(c: char) -> Option<Piece> {
    match c.to_ascii_lowercase() {
        'q' => Some(Piece::Queen),
        'r' => Some(Piece::Rook),
        'b' => Some(Piece::Bishop),
        'n' => Some(Piece::Knight),
        _ => None,
    }
}

/// When a search counts as hung: its expected thinking time plus a grace period.
fn search_deadline(started: Instant, limit: &SearchLimit, side: Color) -> Instant {
    let expected = match limit {
        SearchLimit::MoveTime(d) => *d,
        SearchLimit::Clock(times) => times.for_side(side),
    };
    started + expected + TIMEOUT_GRACE
}

/// Whether the engine playing `engine` accepts a draw at `eval` (White's point of view).
fn engine_accepts_draw(eval: Option<Eval>, engine: Color) -> bool {
    let sign = if engine == Color::White { 1 } else { -1 };
    match eval {
        Some(Eval::Cp(cp)) => cp * sign <= DRAW_ACCEPT_CP,
        Some(Eval::Mate(n)) => n * sign < 0,
        None => false,
    }
}

impl App {
    pub fn new(
        engine: Engine,
        settings: Settings,
        settings_path: Option<PathBuf>,
        start: StartPosition,
    ) -> App {
        let mut app = App {
            game: Game::new(GameSetup {
                start,
                engine_side: None,
                orientation: Color::White,
                time_control: None,
            }),
            engine,
            settings,
            settings_path,
            start,
            game_clock: None,
            pending: None,
            next_id: 0,
            version: 0,
            browse: None,
            drag: Drag::default(),
            confirm: None,
            notice: None,
            engine_failed: false,
            last_restart: None,
            help_open: false,
            engine_line: None,
            tick: 0,
        };
        app.new_game();
        app
    }

    fn new_game(&mut self) {
        self.game_clock = self.settings.clock;
        self.game = Game::new(GameSetup {
            start: self.start,
            engine_side: (!self.settings.two_player).then_some(!self.settings.side),
            orientation: self.settings.side,
            time_control: self.game_clock,
        });
        self.engine_line = None;
        self.engine_failed = false;
        self.changed();
    }

    /// Call after every change to the game: invalidates running searches and ends browsing.
    fn changed(&mut self) {
        self.version += 1;
        self.browse = None;
        self.drop_pending();
    }

    fn drop_pending(&mut self) {
        if let Some(p) = self.pending.take() {
            self.engine.cancel(p.id);
        }
    }

    /// After a move: follow the engine's expected line if the move was its next one.
    fn after_move(&mut self) {
        let played = self.game.history.last().map(|p| (p.before, p.mv));
        self.engine_line = match (self.engine_line.take(), played) {
            (Some((board, line)), Some((before, mv)))
                if board == before && line.first() == Some(&mv) && line.len() > 1 =>
            {
                Some((self.game.board, line[1..].to_vec()))
            }
            _ => None,
        };
        self.changed();
    }

    fn set_notice(&mut self, text: impl Into<String>, now: Instant) {
        self.notice = Some(Notice {
            text: text.into(),
            until: Some(now + NOTICE_TIME),
        });
    }

    fn save_settings(&mut self, now: Instant) {
        if let Some(path) = &self.settings_path
            && let Err(e) = self.settings.save(path)
        {
            self.set_notice(format!("Settings not saved: {e}"), now);
        }
    }

    fn game_in_progress(&self) -> bool {
        !self.game.history.is_empty() && !self.game.game_over()
    }

    // ----- time-driven work -------------------------------------------------------------

    pub fn tick(&mut self, now: Instant) {
        self.tick = self.tick.wrapping_add(1);
        if self
            .notice
            .as_ref()
            .and_then(|n| n.until)
            .is_some_and(|until| now >= until)
        {
            self.notice = None;
        }
        if self.confirm.is_some_and(|(_, until)| now >= until) {
            self.confirm = None;
        }

        // 1. Search events; an engine move is judged at the time the engine answered.
        self.drain_pending(now);
        // 2. Clocks.
        if self.game.check_time(now) {
            self.changed();
        }
        // 3. A search that never answers means a hung engine.
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            self.drop_pending();
            self.restart_engine(now);
        }
        // 4. The engine's turn.
        if self.pending.is_none()
            && !self.engine_failed
            && !self.game.game_over()
            && !self.game.is_human_turn()
        {
            self.start_search(Purpose::EngineMove, now);
        }
    }

    fn drain_pending(&mut self, now: Instant) {
        let Some(pending) = &self.pending else {
            return;
        };
        if pending.version != self.version {
            self.drop_pending();
            return;
        }
        let purpose = pending.purpose;
        let mut done = None;
        let mut dead = false;
        loop {
            match pending.rx.try_recv() {
                Ok(SearchEvent::Progress(info)) => {
                    self.game.eval = Some(info.eval);
                    self.game.eval_depth = Some(info.depth);
                    self.engine_line = Some((self.game.board, info.pv));
                }
                Ok(SearchEvent::Done { result, at }) => {
                    done = Some((result, at));
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    dead = true;
                    break;
                }
            }
        }
        if dead {
            self.pending = None;
            self.restart_engine(now);
            return;
        }
        let Some((result, at)) = done else {
            return;
        };
        self.pending = None;
        if let Some(r) = result {
            self.game.eval = Some(r.eval);
            self.game.eval_depth = Some(r.depth);
        }
        match (purpose, result) {
            (Purpose::Hint, Some(r)) => {
                self.game.set_hint(r.mv);
            }
            (Purpose::Hint, None) => {}
            (Purpose::EngineMove, Some(r)) => match self.game.apply_engine_move(r.mv, at) {
                EngineMove::Applied => self.after_move(),
                EngineMove::TooLate => self.changed(),
                EngineMove::Illegal => self.fail_engine(),
            },
            (Purpose::EngineMove, None) => self.fail_engine(),
        }
    }

    fn fail_engine(&mut self) {
        self.engine_failed = true;
        self.notice = Some(Notice {
            text: "Engine failed: u/r/n".to_string(),
            until: None,
        });
    }

    fn restart_engine(&mut self, now: Instant) {
        let unstable = self
            .last_restart
            .is_some_and(|t| now.saturating_duration_since(t) < RESTART_WINDOW);
        self.engine = if unstable {
            Engine::Builtin
        } else {
            Engine::detect()
        };
        self.last_restart = Some(now);
        let text = if unstable {
            "Engine unstable: built-in engine"
        } else {
            "Engine restarted"
        };
        self.set_notice(text, now);
    }

    fn start_search(&mut self, purpose: Purpose, now: Instant) {
        let id = self.next_id;
        self.next_id += 1;
        let (level, limit) = match purpose {
            Purpose::Hint => (Level::MAX, SearchLimit::MoveTime(MOVE_TIME)),
            Purpose::EngineMove => (
                self.settings.level,
                self.game
                    .clock
                    .as_ref()
                    .map_or(SearchLimit::MoveTime(MOVE_TIME), |c| {
                        SearchLimit::Clock(c.times(now))
                    }),
            ),
        };
        let deadline = search_deadline(now, &limit, self.game.board.side_to_move());
        let rx = self
            .engine
            .think(self.game.search_request(id, level, limit));
        self.pending = Some(Pending {
            id,
            version: self.version,
            purpose,
            rx,
            deadline,
        });
    }

    // ----- keyboard ---------------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent, now: Instant) -> Flow {
        if key.kind != KeyEventKind::Press {
            return Flow::Continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Flow::Quit;
        }
        if self.help_open {
            self.help_open = false;
            return Flow::Continue;
        }
        // A pending confirmation only survives the matching key.
        let confirming = self
            .confirm
            .take()
            .filter(|&(_, until)| now < until)
            .map(|(action, _)| action);

        if matches!(self.game.phase, Phase::Promoting { .. }) {
            match key.code {
                KeyCode::Char(c) => {
                    if let Some(piece) = promotion_piece(c)
                        && self.game.promote(piece, now)
                    {
                        self.after_move();
                    }
                }
                KeyCode::Esc => self.game.cancel(),
                _ => {}
            }
            return Flow::Continue;
        }

        match key.code {
            KeyCode::Esc => {
                if self.browse.is_some() {
                    self.browse = None;
                } else {
                    self.game.cancel();
                }
            }
            KeyCode::Left | KeyCode::Char('h') => self.cursor(-1, 0),
            KeyCode::Right | KeyCode::Char('l') => self.cursor(1, 0),
            KeyCode::Up | KeyCode::Char('k') => self.cursor(0, 1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor(0, -1),
            KeyCode::Enter => {
                if self.browse.is_some() {
                    self.browse = None;
                } else if self.game.confirm(now) {
                    self.after_move();
                }
            }
            KeyCode::F(1) | KeyCode::Char('?') => self.help_open = true,
            KeyCode::Home => {
                if !self.game.history.is_empty() {
                    self.browse = Some(0);
                }
            }
            KeyCode::End => self.browse = None,
            KeyCode::Char(c) => match c.to_ascii_lowercase() {
                'q' => return Flow::Quit,
                'u' => self.undo(now),
                'r' => self.request(Action::Restart, confirming, now),
                'n' => self.request(Action::NewGameSwap, confirming, now),
                'm' => self.request(Action::SwitchMode, confirming, now),
                'x' => self.request(Action::Resign, confirming, now),
                'd' => self.request(Action::OfferDraw, confirming, now),
                'f' => self.game.orientation = !self.game.orientation,
                '+' | '=' => self.change_level(self.settings.level.stronger(), now),
                '-' => self.change_level(self.settings.level.weaker(), now),
                's' => self.hint(now),
                'c' => self.cycle_clock(now),
                't' => self.cycle_theme(now),
                ',' => self.browse_back(),
                '.' => self.browse_forward(),
                _ => {}
            },
            _ => {}
        }
        Flow::Continue
    }

    fn cursor(&mut self, df: i8, dr: i8) {
        self.browse = None;
        self.game.move_cursor(df, dr);
    }

    fn request(&mut self, action: Action, confirming: Option<Action>, now: Instant) {
        let needs_confirm = match action {
            Action::Resign | Action::OfferDraw => {
                if self.game.game_over() {
                    self.set_notice("The game is over", now);
                    return;
                }
                true
            }
            Action::Restart | Action::NewGameSwap | Action::SwitchMode => self.game_in_progress(),
        };
        if needs_confirm && confirming != Some(action) {
            self.confirm = Some((action, now + NOTICE_TIME));
            self.set_notice(action.prompt(), now);
            return;
        }
        self.notice = None;
        match action {
            Action::Restart => self.new_game(),
            Action::NewGameSwap => {
                self.settings.side = !self.settings.side;
                self.save_settings(now);
                self.new_game();
            }
            Action::SwitchMode => {
                self.settings.two_player = !self.settings.two_player;
                self.save_settings(now);
                self.new_game();
            }
            Action::Resign => {
                if self.game.resign(now) {
                    self.changed();
                }
            }
            Action::OfferDraw => match self.game.engine_side {
                None => {
                    if self.game.agree_draw(now) {
                        self.changed();
                    }
                }
                Some(engine) => {
                    if engine_accepts_draw(self.game.eval, engine) {
                        self.game.agree_draw(now);
                        self.changed();
                        self.set_notice("AI accepts the draw", now);
                    } else {
                        self.set_notice("AI declines the draw", now);
                    }
                }
            },
        }
    }

    fn undo(&mut self, now: Instant) {
        if self.game.is_final() {
            self.set_notice("Game over: r for a new one", now);
        } else if self.game.undo(now) {
            self.engine_line = None;
            self.engine_failed = false;
            self.notice = None;
            self.changed();
        } else {
            self.set_notice("Nothing to undo", now);
        }
    }

    fn hint(&mut self, now: Instant) {
        let reason = if self.game.game_over() {
            Some("The game is over")
        } else if !self.game.is_human_turn() || self.pending.is_some() {
            Some("Wait for the engine")
        } else if self.browse.is_some() {
            Some("End browsing first")
        } else {
            None
        };
        match reason {
            Some(text) => self.set_notice(text, now),
            None => self.start_search(Purpose::Hint, now),
        }
    }

    fn change_level(&mut self, level: Level, now: Instant) {
        self.settings.level = level;
        self.save_settings(now);
        let strength = match (&self.engine, level.elo()) {
            (Engine::Uci(_), Some(elo)) => format!(" (~{elo} Elo)"),
            (Engine::Uci(_), None) => " (full)".to_string(),
            (Engine::Builtin, _) => String::new(),
        };
        let scope = if self.settings.two_player {
            ", vs engine"
        } else {
            ""
        };
        self.set_notice(format!("Level {}/8{strength}{scope}", level.get()), now);
    }

    fn cycle_clock(&mut self, now: Instant) {
        self.settings.clock = clock::next_preset(self.settings.clock);
        self.save_settings(now);
        let text = format!(
            "Clock {} next game",
            clock::preset_name(self.settings.clock)
        );
        self.set_notice(text, now);
    }

    fn cycle_theme(&mut self, now: Instant) {
        let next = (ui::theme_index(&self.settings.theme) + 1) % ui::THEMES.len();
        self.settings.theme = ui::THEMES[next].name.to_string();
        self.save_settings(now);
    }

    fn browse_back(&mut self) {
        let len = self.game.history.len();
        self.browse = match self.browse {
            None if len > 0 => Some(len - 1),
            None => None,
            Some(ply) => Some(ply.saturating_sub(1)),
        };
    }

    fn browse_forward(&mut self) {
        let len = self.game.history.len();
        self.browse = self
            .browse
            .and_then(|ply| (ply + 1 < len).then_some(ply + 1));
    }

    // ----- mouse ------------------------------------------------------------------------

    pub fn on_mouse(&mut self, event: MouseEvent, area: Rect, now: Instant) -> Flow {
        let pos = Position {
            x: event.column,
            y: event.row,
        };
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_down(pos, area, now),
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.drag.from.is_some()
                    && let Some(sq) = ui::square_at(area, self.game.orientation, pos)
                {
                    self.game.set_cursor(sq);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.mouse_up(pos, area, now),
            _ => {}
        }
        Flow::Continue
    }

    fn mouse_down(&mut self, pos: Position, area: Rect, now: Instant) {
        self.drag = Drag::default(); // a lost Up never leaves a stale drag
        self.confirm = None;
        if self.help_open {
            self.help_open = false;
            return;
        }
        if matches!(self.game.phase, Phase::Promoting { .. }) {
            if let Some(piece) = ui::promotion_choice_at(area, pos)
                && self.game.promote(piece, now)
            {
                self.after_move();
            }
            return;
        }
        if self.browse.is_some() {
            self.browse = None;
            return;
        }
        let Some(sq) = ui::square_at(area, self.game.orientation, pos) else {
            return;
        };
        self.game.set_cursor(sq);
        if self.game.phase == Phase::Selected(sq) {
            // Deselect only on release, so this can still start a drag.
            self.drag = Drag {
                from: Some(sq),
                was_selected: true,
            };
        } else if self.game.confirm(now) {
            self.after_move();
        } else if self.game.phase == Phase::Selected(sq) {
            self.drag.from = Some(sq);
        }
    }

    fn mouse_up(&mut self, pos: Position, area: Rect, now: Instant) {
        let drag = std::mem::take(&mut self.drag);
        let Some(from) = drag.from else {
            return;
        };
        match ui::square_at(area, self.game.orientation, pos) {
            Some(to) if to != from => {
                if self.game.legal_targets.contains(&to) {
                    self.game.set_cursor(to);
                    if self.game.confirm(now) {
                        self.after_move();
                    }
                } else {
                    self.game.set_cursor(from); // snap back
                }
            }
            Some(_) if drag.was_selected => self.game.cancel(),
            _ => {}
        }
    }

    // ----- view -------------------------------------------------------------------------

    pub fn view(&self, now: Instant) -> ui::View<'_> {
        ui::View {
            theme: &ui::THEMES[ui::theme_index(&self.settings.theme)],
            tick: self.tick,
            now,
            thinking: self.pending.as_ref().map(|p| match p.purpose {
                Purpose::EngineMove => "AI thinking…",
                Purpose::Hint => "Finding a hint…",
            }),
            engine_name: self.engine.name(),
            level: self.settings.level,
            browse: self.browse,
            next_clock: (self.settings.clock != self.game_clock).then_some(self.settings.clock),
            notice: self.notice.as_ref().map(|n| n.text.as_str()),
            help_open: self.help_open,
            engine_line: self
                .engine_line
                .as_ref()
                .map(|(board, line)| (board, line.as_slice())),
            settings_path: self.settings_path.as_ref().and_then(|p| p.to_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ClockTimes;
    use crossterm::event::KeyEventState;

    fn app(settings: Settings) -> App {
        App::new(Engine::Builtin, settings, None, StartPosition::default())
    }

    fn press(app: &mut App, code: KeyCode, now: Instant) -> Flow {
        app.on_key(
            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
            now,
        )
    }

    fn keys(app: &mut App, text: &str, now: Instant) {
        for c in text.chars() {
            press(app, KeyCode::Char(c), now);
        }
    }

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };

    fn mouse(app: &mut App, kind: MouseEventKind, sq: Square, now: Instant) {
        let pos = ui::square_center(AREA, app.game.orientation, sq);
        app.on_mouse(
            MouseEvent {
                kind,
                column: pos.x,
                row: pos.y,
                modifiers: KeyModifiers::NONE,
            },
            AREA,
            now,
        );
    }

    fn click(app: &mut App, sq: Square, now: Instant) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), sq, now);
        mouse(app, MouseEventKind::Up(MouseButton::Left), sq, now);
    }

    /// Two players, so no engine moves interfere.
    fn two_player_app() -> App {
        app(Settings {
            two_player: true,
            ..Settings::default()
        })
    }

    #[test]
    fn resign_needs_a_second_press() {
        let t0 = Instant::now();
        let mut a = two_player_app();
        keys(&mut a, "x", t0);
        assert!(!a.game.game_over());
        keys(&mut a, "t", t0); // another key cancels
        keys(&mut a, "x", t0);
        assert!(!a.game.game_over());
        a.tick(t0 + Duration::from_secs(5)); // expires
        keys(&mut a, "x", t0 + Duration::from_secs(5));
        assert!(!a.game.game_over());
        keys(&mut a, "x", t0 + Duration::from_secs(6));
        assert!(a.game.status_text().contains("resigns"));
    }

    #[test]
    fn restart_confirms_only_during_a_game() {
        let t0 = Instant::now();
        let mut a = two_player_app();
        keys(&mut a, "f", t0);
        assert_eq!(a.game.orientation, Color::Black);
        keys(&mut a, "r", t0); // nothing played yet: immediate
        assert_eq!(a.game.orientation, Color::White);
        click(&mut a, Square::E2, t0);
        click(&mut a, Square::E4, t0);
        assert_eq!(a.game.history.len(), 1);
        keys(&mut a, "r", t0);
        assert_eq!(a.game.history.len(), 1, "first press only asks");
        keys(&mut a, "r", t0);
        assert!(a.game.history.is_empty());
    }

    #[test]
    fn draw_offer_depends_on_engine_eval() {
        let t0 = Instant::now();
        let mut a = app(Settings::default()); // engine plays Black
        keys(&mut a, "dd", t0);
        assert!(!a.game.game_over(), "no eval yet: declined");
        a.game.eval = Some(Eval::Cp(-200)); // Black (engine) is 2 pawns up
        keys(&mut a, "dd", t0);
        assert!(!a.game.game_over());
        a.game.eval = Some(Eval::Cp(10));
        keys(&mut a, "dd", t0);
        assert_eq!(a.game.status_text(), "Draw agreed.");

        assert!(engine_accepts_draw(Some(Eval::Mate(3)), Color::Black));
        assert!(!engine_accepts_draw(Some(Eval::Mate(3)), Color::White));
    }

    #[test]
    fn clock_change_waits_for_the_next_game() {
        let t0 = Instant::now();
        let mut a = two_player_app();
        click(&mut a, Square::E2, t0);
        click(&mut a, Square::E4, t0);
        keys(&mut a, "c", t0);
        assert_eq!(a.game.history.len(), 1, "c never resets the game");
        assert!(a.game.clock.is_none());
        assert_eq!(a.view(t0).next_clock, Some(clock::PRESETS[1]));
        keys(&mut a, "rr", t0);
        assert!(a.game.clock.is_some());
        assert_eq!(a.view(t0).next_clock, None);
    }

    #[test]
    fn browsing_and_reset_on_move() {
        let t0 = Instant::now();
        let mut a = two_player_app();
        keys(&mut a, ",", t0);
        assert_eq!(a.browse, None, "nothing to browse");
        for (from, to) in [(Square::E2, Square::E4), (Square::E7, Square::E5)] {
            click(&mut a, from, t0);
            click(&mut a, to, t0);
        }
        keys(&mut a, ",", t0);
        assert_eq!(a.browse, Some(1));
        keys(&mut a, ",,", t0);
        assert_eq!(a.browse, Some(0));
        keys(&mut a, ".", t0);
        assert_eq!(a.browse, Some(1));
        keys(&mut a, ".", t0);
        assert_eq!(a.browse, None);
        press(&mut a, KeyCode::Home, t0);
        assert_eq!(a.browse, Some(0));
        click(&mut a, Square::G1, t0); // a click returns to live, nothing else
        assert_eq!(a.browse, None);
        assert_eq!(a.game.phase, Phase::Idle);
        keys(&mut a, ",", t0);
        click(&mut a, Square::G1, t0);
        click(&mut a, Square::G1, t0);
        click(&mut a, Square::F3, t0);
        assert_eq!(a.game.history.len(), 3);
        assert_eq!(a.browse, None, "a move ends browsing");
    }

    #[test]
    fn help_swallows_the_next_key() {
        let t0 = Instant::now();
        let mut a = two_player_app();
        keys(&mut a, "?", t0);
        assert!(a.help_open);
        assert_eq!(press(&mut a, KeyCode::Char('q'), t0), Flow::Continue);
        assert!(!a.help_open);
        assert_eq!(press(&mut a, KeyCode::Char('q'), t0), Flow::Quit);
    }

    #[test]
    fn mouse_click_and_drag() {
        let t0 = Instant::now();
        let mut a = two_player_app();
        // click-click
        click(&mut a, Square::E2, t0);
        assert_eq!(a.game.phase, Phase::Selected(Square::E2));
        click(&mut a, Square::E4, t0);
        assert_eq!(a.game.history.len(), 1);
        // drag
        mouse(
            &mut a,
            MouseEventKind::Down(MouseButton::Left),
            Square::E7,
            t0,
        );
        mouse(
            &mut a,
            MouseEventKind::Drag(MouseButton::Left),
            Square::E6,
            t0,
        );
        assert_eq!(a.game.cursor_square(), Square::E6);
        mouse(
            &mut a,
            MouseEventKind::Up(MouseButton::Left),
            Square::E5,
            t0,
        );
        assert_eq!(a.game.history.len(), 2);
        // drag to a non-target snaps back and keeps the selection
        mouse(
            &mut a,
            MouseEventKind::Down(MouseButton::Left),
            Square::G1,
            t0,
        );
        mouse(
            &mut a,
            MouseEventKind::Up(MouseButton::Left),
            Square::G4,
            t0,
        );
        assert_eq!(a.game.phase, Phase::Selected(Square::G1));
        assert_eq!(a.game.cursor_square(), Square::G1);
        // clicking the selected piece again deselects
        click(&mut a, Square::G1, t0);
        assert_eq!(a.game.phase, Phase::Idle);
        // dragging an already selected piece moves it
        click(&mut a, Square::G1, t0);
        mouse(
            &mut a,
            MouseEventKind::Down(MouseButton::Left),
            Square::G1,
            t0,
        );
        mouse(
            &mut a,
            MouseEventKind::Up(MouseButton::Left),
            Square::F3,
            t0,
        );
        assert_eq!(a.game.history.len(), 3);
    }

    #[test]
    fn mouse_promotion() {
        let t0 = Instant::now();
        let mut a = App::new(
            Engine::Builtin,
            Settings {
                two_player: true,
                ..Settings::default()
            },
            None,
            StartPosition::from_fen("7k/P7/8/8/8/8/8/K7 w - - 0 1").unwrap(),
        );
        mouse(
            &mut a,
            MouseEventKind::Down(MouseButton::Left),
            Square::A7,
            t0,
        );
        mouse(
            &mut a,
            MouseEventKind::Up(MouseButton::Left),
            Square::A8,
            t0,
        );
        assert!(matches!(a.game.phase, Phase::Promoting { .. }));
        let knight = ui::promotion_cell(AREA, Piece::Knight);
        a.on_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: knight.x,
                row: knight.y,
                modifiers: KeyModifiers::NONE,
            },
            AREA,
            t0,
        );
        assert_eq!(a.game.board.piece_on(Square::A8), Some(Piece::Knight));
    }

    #[test]
    fn stale_search_from_previous_game_is_dropped() {
        let t0 = Instant::now();
        // Human Black: the engine (White) starts searching at once.
        let mut a = app(Settings {
            side: Color::Black,
            level: Level::new(1),
            ..Settings::default()
        });
        a.tick(t0);
        assert!(a.pending.is_some());
        keys(&mut a, "n", t0); // no move played yet: immediate new game, human White
        assert!(a.pending.is_none());
        std::thread::sleep(Duration::from_millis(150)); // the old search finishes meanwhile
        a.tick(t0);
        assert!(a.game.history.is_empty(), "old result not applied");
        assert!(!a.engine_failed);
        assert!(a.game.is_human_turn());
    }

    #[test]
    fn engine_plays_its_move() {
        let t0 = Instant::now();
        let mut a = app(Settings {
            level: Level::new(1),
            ..Settings::default()
        });
        click(&mut a, Square::E2, t0);
        click(&mut a, Square::E4, t0);
        a.tick(t0);
        let deadline = Instant::now() + Duration::from_secs(5);
        while a.game.history.len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            a.tick(Instant::now());
        }
        assert_eq!(a.game.history.len(), 2);
        assert!(a.game.eval.is_some());
    }

    #[test]
    fn hint_suggests_a_legal_move() {
        let t0 = Instant::now();
        let mut a = two_player_app();
        keys(&mut a, "s", t0);
        assert!(a.pending.is_some());
        let deadline = Instant::now() + Duration::from_secs(5);
        while a.game.hint.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            a.tick(Instant::now());
        }
        let hint = a.game.hint.expect("a hint");
        assert!(chess::MoveGen::new_legal(&a.game.board).any(|m| m == hint));
    }

    #[test]
    fn deadline_and_restart_guard() {
        let t0 = Instant::now();
        let limit = SearchLimit::MoveTime(Duration::from_secs(1));
        assert_eq!(
            search_deadline(t0, &limit, Color::White),
            t0 + Duration::from_secs(6)
        );
        let limit = SearchLimit::Clock(ClockTimes {
            white: Duration::from_secs(10),
            black: Duration::from_secs(99),
            increment: Duration::ZERO,
        });
        assert_eq!(
            search_deadline(t0, &limit, Color::White),
            t0 + Duration::from_secs(15)
        );

        let mut a = two_player_app();
        a.last_restart = Some(t0);
        a.restart_engine(t0 + Duration::from_secs(10));
        assert!(matches!(a.engine, Engine::Builtin));
        assert_eq!(a.view(t0).notice, Some("Engine unstable: built-in engine"));
    }
}
