use std::str::FromStr;
use std::time::Instant;

use chess::{
    ALL_SQUARES, Board, BoardBuilder, BoardStatus, ChessMove, Color, EMPTY, File, MoveGen, Piece,
    Rank, Square,
};

use crate::clock::{Clock, TimeControl};
use crate::engine::{Eval, Level, SearchLimit, SearchRequest};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Selected(Square),
    Promoting { from: Square, to: Square },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Checkmate {
        winner: Color,
    },
    Stalemate,
    Repetition,
    FiftyMove,
    InsufficientMaterial,
    Resigned {
        loser: Color,
    },
    DrawAgreed,
    /// `draw`: the side that did not flag cannot mate.
    Timeout {
        loser: Color,
        draw: bool,
    },
}

/// Where a game starts, with the FEN move counters the chess crate does not keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StartPosition {
    pub board: Board,
    pub halfmove: u32,
    pub fullmove: u32,
}

impl Default for StartPosition {
    fn default() -> Self {
        StartPosition {
            board: Board::default(),
            halfmove: 0,
            fullmove: 1,
        }
    }
}

impl StartPosition {
    /// Parses a FEN; the two move counters are optional.
    pub fn from_fen(fen: &str) -> Result<StartPosition, String> {
        let fields: Vec<&str> = fen.split_whitespace().collect();
        if !(4..=6).contains(&fields.len()) {
            return Err(format!(
                "a FEN has 4 to 6 fields, this one has {}",
                fields.len()
            ));
        }
        // The crate splits on single spaces, so pass normalised fields.
        let builder = BoardBuilder::from_str(&fields[..4].join(" "))
            .map_err(|e| format!("invalid position: {e}"))?;
        // chess 3.2 reads out of bounds (undefined behaviour) while building a board without a
        // king for the side to move, before its own sanity check runs; reject that first.
        for color in [Color::White, Color::Black] {
            let kings = ALL_SQUARES
                .iter()
                .filter(|&&sq| builder[sq] == Some((Piece::King, color)))
                .count();
            if kings != 1 {
                return Err(format!(
                    "invalid position: {} must have exactly one king",
                    color_name(color)
                ));
            }
        }
        let board = Board::try_from(&builder).map_err(|e| format!("invalid position: {e}"))?;
        let counter = |i: usize, default: u32| -> Result<u32, String> {
            fields.get(i).map_or(Ok(default), |s| {
                s.parse().map_err(|_| format!("invalid move counter `{s}`"))
            })
        };
        let halfmove = counter(4, 0)?;
        let fullmove = counter(5, 1)?;
        if fullmove == 0 {
            return Err("the fullmove number starts at 1".to_string());
        }
        Ok(StartPosition {
            board,
            halfmove,
            fullmove,
        })
    }
}

pub struct GameSetup {
    pub start: StartPosition,
    /// `None`: two players, no engine.
    pub engine_side: Option<Color>,
    /// The side shown at the bottom.
    pub orientation: Color,
    pub time_control: Option<TimeControl>,
}

/// What happened to a move the engine sent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineMove {
    Applied,
    /// Not a legal move for the engine here.
    Illegal,
    /// The game ended first, e.g. the engine's time ran out before it answered.
    TooLate,
}

/// One played move and the position it was played from.
#[derive(Clone, Debug)]
pub struct Ply {
    pub before: Board,
    pub mv: ChessMove,
    pub san: String,
}

pub struct Game {
    pub board: Board,
    pub start: StartPosition,
    pub orientation: Color,
    pub engine_side: Option<Color>,
    pub cursor: (u8, u8), // (file 0..=7, rank 0..=7), absolute: rank 0 = rank 1
    pub phase: Phase,
    pub legal_targets: Vec<Square>,
    pub history: Vec<Ply>,
    /// Last engine score, White's point of view, and the depth it came from.
    pub eval: Option<Eval>,
    pub eval_depth: Option<u32>,
    /// Suggested move for the side to move.
    pub hint: Option<ChessMove>,
    pub clock: Option<Clock>,
    /// Endings that are not visible on the board: resignation, agreed draw, timeout.
    result: Option<Outcome>,
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "White",
        Color::Black => "Black",
    }
}

impl Game {
    pub fn new(setup: GameSetup) -> Self {
        Self {
            board: setup.start.board,
            start: setup.start,
            orientation: setup.orientation,
            engine_side: setup.engine_side,
            cursor: if setup.orientation == Color::White {
                (4, 1)
            } else {
                (4, 6)
            },
            phase: Phase::Idle,
            legal_targets: Vec::new(),
            history: Vec::new(),
            eval: None,
            eval_depth: None,
            hint: None,
            clock: setup.time_control.map(Clock::new),
            result: None,
        }
    }

    pub fn last_move(&self) -> Option<(Square, Square)> {
        self.history
            .last()
            .map(|p| (p.mv.get_source(), p.mv.get_dest()))
    }

    pub fn is_human_turn(&self) -> bool {
        self.engine_side != Some(self.board.side_to_move())
    }

    pub fn outcome(&self) -> Option<Outcome> {
        if self.result.is_some() {
            return self.result;
        }
        match self.board.status() {
            BoardStatus::Checkmate => Some(Outcome::Checkmate {
                winner: !self.board.side_to_move(),
            }),
            BoardStatus::Stalemate => Some(Outcome::Stalemate),
            BoardStatus::Ongoing if insufficient_material(&self.board) => {
                Some(Outcome::InsufficientMaterial)
            }
            BoardStatus::Ongoing if self.halfmove_clock() >= 100 => Some(Outcome::FiftyMove),
            BoardStatus::Ongoing if self.repetitions() >= 3 => Some(Outcome::Repetition),
            BoardStatus::Ongoing => None,
        }
    }

    pub fn game_over(&self) -> bool {
        self.outcome().is_some()
    }

    /// Resignation, agreed draw and timeout cannot be taken back.
    pub fn is_final(&self) -> bool {
        self.result.is_some()
    }

    /// Plies since the last pawn move or capture, counting the start position's counter when
    /// nothing irreversible happened since.
    fn halfmove_clock(&self) -> usize {
        let quiet = self
            .history
            .iter()
            .rev()
            .take_while(|p| {
                p.before.piece_on(p.mv.get_source()) != Some(Piece::Pawn)
                    && p.before.piece_on(p.mv.get_dest()).is_none()
            })
            .count();
        if quiet == self.history.len() {
            quiet + self.start.halfmove as usize
        } else {
            quiet
        }
    }

    /// How often the current position has occurred, including now.
    fn repetitions(&self) -> usize {
        let hash = self.board.get_hash();
        1 + self
            .history
            .iter()
            .filter(|p| p.before.get_hash() == hash)
            .count()
    }

    /// "You"/"AI" against the engine, the colour name with two players.
    fn who(&self, color: Color) -> &'static str {
        match self.engine_side {
            None => color_name(color),
            Some(engine) if engine == color => "AI",
            Some(_) => "You",
        }
    }

    fn wins(&self, color: Color) -> String {
        match self.engine_side {
            Some(engine) if engine != color => "you win!".to_string(),
            _ => format!("{} wins.", self.who(color)),
        }
    }

    pub fn status_text(&self) -> String {
        let stm = self.board.side_to_move();
        match self.outcome() {
            Some(Outcome::Checkmate { winner }) => format!("Checkmate — {}", self.wins(winner)),
            Some(Outcome::Stalemate) => "Draw — stalemate.".to_string(),
            Some(Outcome::Repetition) => "Draw — threefold repetition.".to_string(),
            Some(Outcome::FiftyMove) => "Draw — fifty-move rule.".to_string(),
            Some(Outcome::InsufficientMaterial) => "Draw — insufficient material.".to_string(),
            Some(Outcome::DrawAgreed) => "Draw agreed.".to_string(),
            Some(Outcome::Resigned { loser }) => {
                let verb = if self.who(loser) == "You" {
                    "resigned"
                } else {
                    "resigns"
                };
                format!("{} {verb} — {}", self.who(loser), self.wins(!loser))
            }
            Some(Outcome::Timeout { loser, draw }) => {
                let rest = if draw {
                    "draw (insufficient material).".to_string()
                } else {
                    self.wins(!loser)
                };
                format!("{} ran out of time — {rest}", self.who(loser))
            }
            None => {
                let check = *self.board.checkers() != EMPTY;
                match (self.engine_side, check) {
                    (None, true) => format!("Check! {} to move.", color_name(stm)),
                    (None, false) => format!("{} to move.", color_name(stm)),
                    (Some(_), true) if self.is_human_turn() => "Check! Your move.".to_string(),
                    (Some(_), true) => "Check!".to_string(),
                    (Some(_), false) if self.is_human_turn() => "Your move.".to_string(),
                    (Some(_), false) => "AI to move.".to_string(),
                }
            }
        }
    }

    /// Engine input: the starting position plus every move since.
    pub fn search_request(&self, id: u64, level: Level, limit: SearchLimit) -> SearchRequest {
        SearchRequest {
            id,
            root: self.history.first().map_or(self.board, |p| p.before),
            moves: self.history.iter().map(|p| p.mv).collect(),
            level,
            limit,
        }
    }

    pub fn cursor_square(&self) -> Square {
        Square::make_square(
            Rank::from_index(self.cursor.1 as usize),
            File::from_index(self.cursor.0 as usize),
        )
    }

    /// Moves the cursor in screen directions (up = towards the top of the board as shown).
    pub fn move_cursor(&mut self, df: i8, dr: i8) {
        let (df, dr) = if self.orientation == Color::White {
            (df, dr)
        } else {
            (-df, -dr)
        };
        let f = (self.cursor.0 as i8 + df).clamp(0, 7);
        let r = (self.cursor.1 as i8 + dr).clamp(0, 7);
        self.cursor = (f as u8, r as u8);
    }

    pub fn set_cursor(&mut self, sq: Square) {
        self.cursor = (
            sq.get_file().to_index() as u8,
            sq.get_rank().to_index() as u8,
        );
    }

    pub fn cancel(&mut self) {
        self.phase = Phase::Idle;
        self.legal_targets.clear();
    }

    /// The one place a finished game is wrapped up: stop the clock, clear any selection.
    fn finish_if_over(&mut self, now: Instant) {
        if self.game_over() {
            if let Some(clock) = &mut self.clock {
                clock.stop(now);
            }
            self.cancel();
            self.hint = None;
        }
    }

    /// Takes back the last move (two players) or back to the human's last move (engine).
    /// Returns false if there is nothing to take back or the game ended in a final way.
    pub fn undo(&mut self, now: Instant) -> bool {
        if self.is_final() {
            return false;
        }
        let index = match self.engine_side {
            None => self.history.len().checked_sub(1),
            Some(engine) => self
                .history
                .iter()
                .rposition(|p| p.before.side_to_move() != engine),
        };
        let Some(i) = index else {
            return false;
        };
        self.board = self.history[i].before;
        self.history.truncate(i);
        self.eval = None;
        self.eval_depth = None;
        self.hint = None;
        self.cancel();
        let to_move = (!self.history.is_empty()).then(|| self.board.side_to_move());
        if let Some(clock) = &mut self.clock {
            clock.on_undo(to_move, now);
        }
        self.finish_if_over(now);
        true
    }

    /// Enter pressed. Returns true if a human move was made.
    pub fn confirm(&mut self, now: Instant) -> bool {
        if self.game_over() || !self.is_human_turn() {
            return false;
        }
        let cur = self.cursor_square();
        match self.phase {
            Phase::Idle => {
                if self.is_own_piece(cur) {
                    self.select(cur);
                }
                false
            }
            Phase::Selected(from) => {
                if cur == from {
                    self.cancel();
                    return false;
                }
                if self.is_own_piece(cur) {
                    self.select(cur);
                    return false;
                }
                if !self.legal_targets.contains(&cur) {
                    return false;
                }
                let promotes = MoveGen::new_legal(&self.board).any(|m| {
                    m.get_source() == from && m.get_dest() == cur && m.get_promotion().is_some()
                });
                if promotes {
                    self.phase = Phase::Promoting { from, to: cur };
                    return false;
                }
                self.apply_move(ChessMove::new(from, cur, None), now);
                true
            }
            Phase::Promoting { .. } => false,
        }
    }

    /// Called during Promoting phase with chosen piece. Returns true if move applied.
    pub fn promote(&mut self, piece: Piece, now: Instant) -> bool {
        if self.game_over() {
            return false;
        }
        if let Phase::Promoting { from, to } = self.phase {
            let mv = ChessMove::new(from, to, Some(piece));
            if MoveGen::new_legal(&self.board).any(|m| m == mv) {
                self.apply_move(mv, now);
                return true;
            }
            self.cancel();
        }
        false
    }

    /// Plays the engine's move, judged at `at`, the moment the engine answered.
    pub fn apply_engine_move(&mut self, mv: ChessMove, at: Instant) -> EngineMove {
        if self.game_over() {
            return EngineMove::TooLate;
        }
        if self.is_human_turn() || !MoveGen::new_legal(&self.board).any(|m| m == mv) {
            return EngineMove::Illegal;
        }
        let mover = self.board.side_to_move();
        if self.clock.as_ref().and_then(|c| c.flagged(at)) == Some(mover) {
            self.flag(mover, at);
            return EngineMove::TooLate;
        }
        self.apply_move(mv, at);
        EngineMove::Applied
    }

    /// Ends the game if the running side's time is up. Returns true if it did.
    pub fn check_time(&mut self, now: Instant) -> bool {
        if self.game_over() {
            return false;
        }
        match self.clock.as_ref().and_then(|c| c.flagged(now)) {
            Some(loser) => {
                self.flag(loser, now);
                true
            }
            None => false,
        }
    }

    fn flag(&mut self, loser: Color, now: Instant) {
        self.result = Some(Outcome::Timeout {
            loser,
            draw: !side_can_mate(&self.board, !loser),
        });
        self.finish_if_over(now);
    }

    /// Against the engine the human resigns; with two players the side to move does.
    pub fn resign(&mut self, now: Instant) -> bool {
        if self.game_over() {
            return false;
        }
        let loser = match self.engine_side {
            Some(engine) => !engine,
            None => self.board.side_to_move(),
        };
        self.result = Some(Outcome::Resigned { loser });
        self.finish_if_over(now);
        true
    }

    pub fn agree_draw(&mut self, now: Instant) -> bool {
        if self.game_over() {
            return false;
        }
        self.result = Some(Outcome::DrawAgreed);
        self.finish_if_over(now);
        true
    }

    pub fn set_hint(&mut self, mv: ChessMove) -> bool {
        if self.game_over() || !self.is_human_turn() {
            return false;
        }
        self.hint = Some(mv);
        true
    }

    /// The position after `ply` moves (0 = start, `history.len()` = now) and the move that led
    /// to it.
    pub fn position_at(&self, ply: usize) -> (Board, Option<(Square, Square)>) {
        let ply = ply.min(self.history.len());
        let board = self.history.get(ply).map_or(self.board, |p| p.before);
        let last = ply.checked_sub(1).map(|i| {
            (
                self.history[i].mv.get_source(),
                self.history[i].mv.get_dest(),
            )
        });
        (board, last)
    }

    fn is_own_piece(&self, sq: Square) -> bool {
        self.board.color_on(sq) == Some(self.board.side_to_move())
    }

    fn select(&mut self, from: Square) {
        self.phase = Phase::Selected(from);
        let mut targets: Vec<Square> = MoveGen::new_legal(&self.board)
            .filter(|m| m.get_source() == from)
            .map(|m| m.get_dest())
            .collect();
        targets.sort();
        targets.dedup();
        self.legal_targets = targets;
    }

    fn apply_move(&mut self, mv: ChessMove, now: Instant) {
        let before = self.board;
        let notation = san(&before, mv);
        self.board = before.make_move_new(mv);
        self.history.push(Ply {
            before,
            mv,
            san: notation,
        });
        if let Some(clock) = &mut self.clock {
            clock.on_move(before.side_to_move(), now);
        }
        self.hint = None;
        self.cancel();
        self.finish_if_over(now);
    }
}

/// Standard algebraic notation of `mv` played from `board`, e.g. "Nbd2", "exd5", "e8=Q+",
/// "O-O", "Qh4#".
pub fn san(board: &Board, mv: ChessMove) -> String {
    let from = mv.get_source();
    let to = mv.get_dest();
    let piece = board.piece_on(from).unwrap_or(Piece::Pawn);
    let after = board.make_move_new(mv);
    let suffix = match after.status() {
        BoardStatus::Checkmate => "#",
        _ if *after.checkers() != EMPTY => "+",
        _ => "",
    };

    // Castling: king moving two files
    if piece == Piece::King
        && (to.get_file().to_index() as i8 - from.get_file().to_index() as i8).abs() == 2
    {
        let castle = if to.get_file().to_index() > from.get_file().to_index() {
            "O-O"
        } else {
            "O-O-O"
        };
        return format!("{castle}{suffix}");
    }

    // Capture: occupied target, or en-passant (pawn, diagonal, empty target)
    let capture =
        board.piece_on(to).is_some() || (piece == Piece::Pawn && from.get_file() != to.get_file());

    let mut s = String::new();
    match piece {
        Piece::Pawn => {
            if capture {
                s.push(file_char(from));
            }
        }
        _ => {
            s.push(piece_letter(piece));
            s.push_str(&disambiguation(board, piece, from, to));
        }
    }
    if capture {
        s.push('x');
    }
    s.push_str(&to.to_string());
    if let Some(p) = mv.get_promotion() {
        s.push('=');
        s.push(piece_letter(p));
    }
    s.push_str(suffix);
    s
}

/// File, rank, or both of `from` when another `piece` can also reach `to`.
fn disambiguation(board: &Board, piece: Piece, from: Square, to: Square) -> String {
    let rivals: Vec<Square> = MoveGen::new_legal(board)
        .filter(|m| {
            m.get_dest() == to
                && m.get_source() != from
                && board.piece_on(m.get_source()) == Some(piece)
        })
        .map(|m| m.get_source())
        .collect();
    if rivals.is_empty() {
        return String::new();
    }
    let shares_file = rivals.iter().any(|s| s.get_file() == from.get_file());
    let shares_rank = rivals.iter().any(|s| s.get_rank() == from.get_rank());
    match (shares_file, shares_rank) {
        (false, _) => file_char(from).to_string(),
        (true, false) => rank_char(from).to_string(),
        (true, true) => from.to_string(),
    }
}

fn file_char(sq: Square) -> char {
    (b'a' + sq.get_file().to_index() as u8) as char
}

fn rank_char(sq: Square) -> char {
    (b'1' + sq.get_rank().to_index() as u8) as char
}

pub fn piece_letter(piece: Piece) -> char {
    match piece {
        Piece::Pawn => 'P',
        Piece::Knight => 'N',
        Piece::Bishop => 'B',
        Piece::Rook => 'R',
        Piece::Queen => 'Q',
        Piece::King => 'K',
    }
}

/// No sequence of legal moves can mate: bare kings plus at most one knight, or only bishops
/// all on one square colour.
fn insufficient_material(board: &Board) -> bool {
    let heavy =
        *board.pieces(Piece::Pawn) | *board.pieces(Piece::Rook) | *board.pieces(Piece::Queen);
    if heavy != EMPTY {
        return false;
    }
    let knights = board.pieces(Piece::Knight).popcnt();
    let bishops = *board.pieces(Piece::Bishop);
    match knights {
        0 => {
            let mut colors = bishops
                .into_iter()
                .map(|sq| (sq.get_file().to_index() + sq.get_rank().to_index()) % 2);
            match colors.next() {
                None => true,
                Some(first) => colors.all(|c| c == first),
            }
        }
        1 => bishops == EMPTY,
        _ => false,
    }
}

/// Whether `color` has enough material to mate at all: more than a bare king or a king with a
/// single minor piece. The usual rule for deciding a timeout between loss and draw.
fn side_can_mate(board: &Board, color: Color) -> bool {
    let pieces: Vec<Piece> = board
        .color_combined(color)
        .into_iter()
        .filter_map(|sq| board.piece_on(sq))
        .filter(|&p| p != Piece::King)
        .collect();
    !matches!(pieces.as_slice(), [] | [Piece::Knight | Piece::Bishop])
}

/// Material in pawns (P=1, N=B=3, R=5, Q=9).
pub fn material(board: &Board, color: Color) -> i32 {
    board
        .color_combined(color)
        .into_iter()
        .map(|sq| match board.piece_on(sq) {
            Some(Piece::Pawn) => 1,
            Some(Piece::Knight | Piece::Bishop) => 3,
            Some(Piece::Rook) => 5,
            Some(Piece::Queen) => 9,
            _ => 0,
        })
        .sum()
}

/// Pieces of `color` that are missing from the board (i.e. captured).
pub fn captured_pieces(board: &Board, color: Color) -> Vec<(Piece, u8)> {
    let start: [(Piece, u8); 5] = [
        (Piece::Pawn, 8),
        (Piece::Knight, 2),
        (Piece::Bishop, 2),
        (Piece::Rook, 2),
        (Piece::Queen, 1),
    ];
    let mut out = Vec::new();
    for (piece, start_count) in start {
        let on_board = board
            .color_combined(color)
            .into_iter()
            .filter(|&sq| board.piece_on(sq) == Some(piece))
            .count() as u8;
        if on_board < start_count {
            out.push((piece, start_count - on_board));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::clock::PRESETS;
    use std::time::Duration;

    fn sq(f: u8, r: u8) -> Square {
        Square::make_square(Rank::from_index(r as usize), File::from_index(f as usize))
    }

    fn t() -> Instant {
        Instant::now()
    }

    pub(crate) fn game(engine_side: Option<Color>, fen: Option<&str>) -> Game {
        let start = fen.map_or_else(StartPosition::default, |f| {
            StartPosition::from_fen(f).unwrap()
        });
        Game::new(GameSetup {
            start,
            engine_side,
            orientation: engine_side.map_or(Color::White, |e| !e),
            time_control: None,
        })
    }

    pub(crate) fn white_game() -> Game {
        game(Some(Color::Black), None)
    }

    fn game_at(fen: &str, cursor: (u8, u8)) -> Game {
        let mut g = game(Some(Color::Black), Some(fen));
        g.cursor = cursor;
        g
    }

    /// (file, rank), both 0..=7.
    type Coord = (u8, u8);

    /// Plays moves through the cursor UI (human) or `apply_engine_move` (AI).
    pub(crate) fn play(g: &mut Game, moves: &[(Coord, Coord)]) {
        for &(a, b) in moves {
            if g.is_human_turn() {
                g.cursor = a;
                g.confirm(t());
                g.cursor = b;
                assert!(g.confirm(t()), "human move {a:?}->{b:?} rejected");
            } else {
                let mv = ChessMove::new(sq(a.0, a.1), sq(b.0, b.1), None);
                assert_eq!(g.apply_engine_move(mv, t()), EngineMove::Applied);
            }
        }
    }

    #[test]
    fn select_own_piece_shows_targets() {
        let mut g = white_game();
        g.cursor = (4, 1); // e2
        assert!(!g.confirm(t())); // select, not a move
        assert_eq!(g.phase, Phase::Selected(sq(4, 1)));
        assert!(g.legal_targets.contains(&sq(4, 2))); // e3
        assert!(g.legal_targets.contains(&sq(4, 3))); // e4
    }

    #[test]
    fn confirm_target_applies_move() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm(t()); // select e2
        g.cursor = (4, 3); // e4
        assert!(g.confirm(t())); // move applied
        assert_eq!(g.board.side_to_move(), Color::Black);
        assert_eq!(g.board.piece_on(sq(4, 3)), Some(Piece::Pawn));
        assert_eq!(g.phase, Phase::Idle);
        assert!(g.legal_targets.is_empty());
        assert_eq!(g.last_move(), Some((sq(4, 1), sq(4, 3))));
    }

    #[test]
    fn confirm_empty_square_does_nothing() {
        let mut g = white_game();
        g.cursor = (3, 3);
        assert!(!g.confirm(t()));
        assert_eq!(g.phase, Phase::Idle);
        assert_eq!(g.board, Board::default());
    }

    #[test]
    fn enter_on_own_other_piece_reselects() {
        let mut g = white_game();
        g.cursor = (4, 1); // e2 pawn
        g.confirm(t());
        g.cursor = (3, 1); // d2 pawn
        assert!(!g.confirm(t()));
        assert_eq!(g.phase, Phase::Selected(sq(3, 1)));
    }

    #[test]
    fn enter_on_selected_square_deselects() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm(t());
        assert!(!g.confirm(t())); // same square again
        assert_eq!(g.phase, Phase::Idle);
        assert!(g.legal_targets.is_empty());
    }

    #[test]
    fn illegal_target_rejected() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm(t()); // select e2
        g.cursor = (4, 4); // e5 is not a legal pawn move
        assert!(!g.confirm(t()));
        assert_eq!(g.phase, Phase::Selected(sq(4, 1)));
        assert_eq!(g.board, Board::default());
    }

    #[test]
    fn cursor_stays_on_board() {
        let mut g = white_game();
        for _ in 0..10 {
            g.move_cursor(-1, 0);
        }
        assert_eq!(g.cursor.0, 0);
        for _ in 0..10 {
            g.move_cursor(0, 1);
        }
        assert_eq!(g.cursor.1, 7);
        for _ in 0..10 {
            g.move_cursor(1, 0);
        }
        assert_eq!(g.cursor.0, 7);
        for _ in 0..10 {
            g.move_cursor(0, -1);
        }
        assert_eq!(g.cursor.1, 0);
    }

    #[test]
    fn cursor_directions_follow_orientation() {
        let mut g = game(Some(Color::White), None); // human Black, board flipped
        assert_eq!(g.cursor, (4, 6)); // e7
        g.move_cursor(1, 0); // screen right = towards the a-file
        g.move_cursor(0, 1); // screen up = towards rank 1
        assert_eq!(g.cursor, (3, 5));
        g.orientation = Color::White; // flipped view
        g.move_cursor(1, 0);
        assert_eq!(g.cursor, (4, 5));
    }

    #[test]
    fn promotion_flow() {
        let mut g = game_at("7k/P7/8/8/8/8/8/K7 w - - 0 1", (0, 6)); // a7
        g.confirm(t()); // select pawn
        g.cursor = (0, 7); // a8
        assert!(!g.confirm(t())); // enters promotion phase, no move yet
        assert_eq!(
            g.phase,
            Phase::Promoting {
                from: sq(0, 6),
                to: sq(0, 7)
            }
        );
        assert!(g.promote(Piece::Queen, t()));
        assert_eq!(g.board.piece_on(sq(0, 7)), Some(Piece::Queen));
        assert_eq!(g.board.color_on(sq(0, 7)), Some(Color::White));
        assert_eq!(g.phase, Phase::Idle);
        assert_eq!(g.history.last().unwrap().san, "a8=Q+");
    }

    #[test]
    fn promotion_choice_is_legality_checked() {
        let mut g = game_at("7k/P7/8/8/8/8/8/K7 w - - 0 1", (0, 6));
        g.confirm(t());
        g.cursor = (0, 7);
        g.confirm(t());
        // all four promotions legal here
        assert!(g.promote(Piece::Knight, t()));
        assert_eq!(g.board.piece_on(sq(0, 7)), Some(Piece::Knight));
    }

    #[test]
    fn input_ignored_on_ai_turn() {
        let mut g = game(Some(Color::White), None); // White (AI) to move
        g.cursor = (4, 6);
        assert!(!g.confirm(t()));
        assert_eq!(g.phase, Phase::Idle);
    }

    #[test]
    fn engine_move_on_human_turn_is_illegal() {
        let mut g = white_game();
        let mv = ChessMove::new(sq(4, 1), sq(4, 3), None);
        assert_eq!(g.apply_engine_move(mv, t()), EngineMove::Illegal);
        assert_eq!(g.board, Board::default());
    }

    #[test]
    fn game_over_detected_on_checkmate() {
        // Fool's mate: 1.f3 e5 2.g4 Qh4#
        let mut g = game_at(
            "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
            (0, 0),
        );
        assert_eq!(
            g.outcome(),
            Some(Outcome::Checkmate {
                winner: Color::Black
            })
        );
        assert!(g.status_text().contains("AI wins"));
        // no input accepted afterwards
        g.cursor = (4, 1);
        assert!(!g.confirm(t()));
    }

    #[test]
    fn check_status_text() {
        // White king e1 in check along file by black rook e2
        let g = game_at("4k3/8/8/8/8/8/4r3/4K3 w - - 0 1", (0, 0));
        assert!(!g.game_over());
        assert!(g.status_text().contains("Check"));
    }

    #[test]
    fn two_player_wording_and_undo() {
        let mut g = game(None, None);
        assert_eq!(g.status_text(), "White to move.");
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]); // 1.e4 e5, both by hand
        assert_eq!(g.status_text(), "White to move.");
        assert!(g.undo(t()));
        assert_eq!(g.history.len(), 1, "two players undo one ply");
        assert_eq!(g.status_text(), "Black to move.");

        // Fool's mate by hand
        let mut g = game(None, None);
        play(
            &mut g,
            &[
                ((5, 1), (5, 2)),
                ((4, 6), (4, 4)),
                ((6, 1), (6, 3)),
                ((3, 7), (7, 3)),
            ],
        );
        assert_eq!(g.status_text(), "Checkmate — Black wins.");
    }

    #[test]
    fn undo_takes_back_human_and_ai_move() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]); // 1.e4 e5
        assert!(g.undo(t()));
        assert_eq!(g.board, Board::default());
        assert!(g.history.is_empty());
        assert!(!g.undo(t()));
    }

    #[test]
    fn undo_while_ai_thinking_takes_back_one_ply() {
        let mut g = white_game();
        play(
            &mut g,
            &[((4, 1), (4, 3)), ((4, 6), (4, 4)), ((6, 0), (5, 2))],
        ); // 1.e4 e5 2.Nf3
        assert!(g.undo(t()));
        assert_eq!(g.history.len(), 2);
        assert!(g.is_human_turn());
    }

    #[test]
    fn undo_as_black_needs_a_human_move() {
        let mut g = game(Some(Color::White), None);
        play(&mut g, &[((4, 1), (4, 3))]); // AI 1.e4
        assert!(!g.undo(t()));
        assert_eq!(g.history.len(), 1);
        play(&mut g, &[((4, 6), (4, 4)), ((6, 0), (5, 2))]); // 1...e5 2.Nf3
        assert!(g.undo(t()));
        assert_eq!(g.history.len(), 1);
        assert!(g.is_human_turn());
    }

    #[test]
    fn resign_and_agreed_draw_are_final() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]);
        assert!(g.resign(t()));
        assert_eq!(
            g.outcome(),
            Some(Outcome::Resigned {
                loser: Color::White
            })
        );
        assert_eq!(g.status_text(), "You resigned — AI wins.");
        assert!(!g.undo(t()), "a resignation cannot be taken back");
        assert!(!g.resign(t()));

        let mut g = game(None, None);
        play(&mut g, &[((4, 1), (4, 3))]);
        assert!(g.resign(t())); // Black, the side to move, resigns
        assert_eq!(g.status_text(), "Black resigns — White wins.");

        let mut g = white_game();
        assert!(g.agree_draw(t()));
        assert_eq!(g.status_text(), "Draw agreed.");
        assert!(!g.set_hint(ChessMove::new(sq(4, 1), sq(4, 3), None)));
    }

    #[test]
    fn engine_flag_is_judged_at_answer_time() {
        let t0 = Instant::now();
        let setup = |fen: Option<&str>| GameSetup {
            start: fen.map_or_else(StartPosition::default, |f| {
                StartPosition::from_fen(f).unwrap()
            }),
            engine_side: Some(Color::Black),
            orientation: Color::White,
            time_control: PRESETS[1], // 1+0
        };

        // Answer inside the minute: applied even if handled later.
        let mut g = Game::new(setup(None));
        g.cursor = (4, 1);
        g.confirm(t0);
        g.cursor = (4, 3);
        assert!(g.confirm(t0)); // 1.e4 (untimed); Black's minute starts at t0
        let e5 = ChessMove::new(sq(4, 6), sq(4, 4), None);
        assert_eq!(
            g.apply_engine_move(e5, t0 + Duration::from_secs(59)),
            EngineMove::Applied
        );

        // Answer after the minute: too late, Black loses on time.
        let mut g = Game::new(setup(None));
        g.cursor = (4, 1);
        g.confirm(t0);
        g.cursor = (4, 3);
        g.confirm(t0);
        assert_eq!(
            g.apply_engine_move(e5, t0 + Duration::from_secs(61)),
            EngineMove::TooLate
        );
        assert_eq!(
            g.outcome(),
            Some(Outcome::Timeout {
                loser: Color::Black,
                draw: false
            })
        );
        assert_eq!(g.status_text(), "AI ran out of time — you win!");
        assert!(!g.undo(t0), "a timeout cannot be taken back");

        // Lone knight cannot mate: Black's flag is a draw.
        let mut g = Game::new(setup(Some("k7/8/8/8/8/8/8/KN4q1 w - - 0 1")));
        g.cursor = (0, 0);
        g.confirm(t0);
        g.cursor = (0, 1);
        assert!(g.confirm(t0)); // Ka2
        assert!(!g.check_time(t0 + Duration::from_secs(30)));
        assert!(g.check_time(t0 + Duration::from_secs(60)));
        assert_eq!(
            g.outcome(),
            Some(Outcome::Timeout {
                loser: Color::Black,
                draw: true
            })
        );
        assert_eq!(
            g.status_text(),
            "AI ran out of time — draw (insufficient material)."
        );
    }

    #[test]
    fn promotion_refused_after_timeout() {
        let t0 = Instant::now();
        let mut g = Game::new(GameSetup {
            start: StartPosition::from_fen("7k/P7/8/8/8/8/8/K7 b - - 0 1").unwrap(),
            engine_side: None,
            orientation: Color::White,
            time_control: PRESETS[1],
        });
        play(&mut g, &[((7, 7), (6, 7))]); // Kg8 starts White's clock
        g.cursor = (0, 6);
        g.confirm(t0);
        g.cursor = (0, 7);
        g.confirm(t0);
        assert!(matches!(g.phase, Phase::Promoting { .. }));
        assert!(g.check_time(Instant::now() + Duration::from_secs(61)));
        assert_eq!(g.phase, Phase::Idle, "the popup closes when the game ends");
        assert!(!g.promote(Piece::Queen, t0));
    }

    #[test]
    fn threefold_repetition_is_a_draw() {
        let mut g = white_game();
        let shuffle = [
            ((6, 0), (5, 2)), // Nf3
            ((6, 7), (5, 5)), // Nf6
            ((5, 2), (6, 0)), // Ng1
            ((5, 5), (6, 7)), // Ng8
        ];
        play(&mut g, &shuffle);
        assert_eq!(g.outcome(), None); // start position seen twice
        play(&mut g, &shuffle);
        assert_eq!(g.outcome(), Some(Outcome::Repetition));
        assert!(g.status_text().contains("repetition"));
    }

    #[test]
    fn fifty_move_rule_counts_fen_halfmoves() {
        // 99 quiet half-moves already in the FEN; one more quiet move ends the game.
        let mut g = game(Some(Color::Black), Some("7k/8/8/8/8/8/8/KR6 w - - 99 60"));
        assert_eq!(g.start.fullmove, 60);
        assert_eq!(g.outcome(), None);
        play(&mut g, &[((1, 0), (1, 1))]); // Rb2
        assert_eq!(g.outcome(), Some(Outcome::FiftyMove));

        // A pawn move resets the clock.
        let mut h = white_game();
        play(
            &mut h,
            &[((6, 0), (5, 2)), ((6, 7), (5, 5)), ((4, 1), (4, 3))],
        );
        assert_eq!(h.halfmove_clock(), 0);
    }

    #[test]
    fn fen_parsing() {
        let messy = StartPosition::from_fen("  8/8/8/8/8/8/8/K6k   b  -  -  ").unwrap();
        assert_eq!(messy.board.side_to_move(), Color::Black);
        assert_eq!((messy.halfmove, messy.fullmove), (0, 1));
        let full = StartPosition::from_fen("8/8/8/8/8/8/8/K6k w - - 12 40").unwrap();
        assert_eq!((full.halfmove, full.fullmove), (12, 40));
        assert!(StartPosition::from_fen("8/8/8/8/8/8/8/K6k w - - 0 0").is_err());
        assert!(StartPosition::from_fen("8/8/8/8/8/8/8/K6k w - - x 1").is_err());
        assert!(StartPosition::from_fen("8/8/8 w").is_err());
        assert!(
            StartPosition::from_fen("8/8/8/8/8/8/8/8 w - -").is_err(),
            "no kings"
        );
        assert!(
            StartPosition::from_fen("8/8/8/8/8/8/8/K7 b - -").is_err(),
            "no black king"
        );
        assert!(
            StartPosition::from_fen("k7/8/8/8/8/8/8/KK6 w - -").is_err(),
            "two kings"
        );
    }

    #[test]
    fn insufficient_material_cases() {
        let dead = |fen: &str| insufficient_material(&Board::from_str(fen).unwrap());
        assert!(dead("7k/8/8/8/8/8/8/K7 w - - 0 1")); // K v K
        assert!(dead("7k/8/8/8/8/8/8/KN6 w - - 0 1")); // K+N v K
        assert!(dead("7k/8/8/8/8/8/8/KB6 w - - 0 1")); // K+B v K
        assert!(dead("6bk/8/8/8/8/8/8/KB6 w - - 0 1")); // bishops b1, g8: both light squares
        assert!(!dead("5b1k/8/8/8/8/8/8/KB6 w - - 0 1")); // b1 light, f8 dark
        assert!(!dead("7k/8/8/8/8/8/8/KNN5 w - - 0 1"));
        assert!(!dead("7k/8/8/8/8/8/P7/K7 w - - 0 1"));
    }

    #[test]
    fn captured_pieces_counts() {
        let b = Board::default();
        assert!(captured_pieces(&b, Color::White).is_empty());
        assert!(captured_pieces(&b, Color::Black).is_empty());
        // black is missing exactly one pawn (a7) and one knight (g8)
        let b2 =
            Board::from_str("rnbqkb1r/1ppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1").unwrap();
        let caps = captured_pieces(&b2, Color::Black);
        assert_eq!(caps, vec![(Piece::Pawn, 1), (Piece::Knight, 1)]);
        assert_eq!(material(&b2, Color::White) - material(&b2, Color::Black), 4);
    }

    #[test]
    fn move_history_notation_and_positions() {
        let mut g = white_game();
        play(
            &mut g,
            &[((4, 1), (4, 3)), ((4, 6), (4, 4)), ((6, 0), (5, 2))],
        ); // 1.e4 e5 2.Nf3
        let sans: Vec<&str> = g.history.iter().map(|p| p.san.as_str()).collect();
        assert_eq!(sans, vec!["e4", "e5", "Nf3"]);

        assert_eq!(g.position_at(0), (Board::default(), None));
        let (after_e4, last) = g.position_at(1);
        assert_eq!(after_e4.side_to_move(), Color::Black);
        assert_eq!(last, Some((sq(4, 1), sq(4, 3))));
        assert_eq!(g.position_at(3).0, g.board);
        assert_eq!(g.position_at(99).0, g.board);
    }

    #[test]
    fn notation_capture() {
        // white pawn e4 can capture black pawn d5: exd5
        let mut g = game_at(
            "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1",
            (4, 3), // e4
        );
        g.confirm(t());
        g.cursor = (3, 4); // d5
        assert!(g.confirm(t()));
        assert_eq!(g.history.last().unwrap().san, "exd5");
    }

    #[test]
    fn notation_castling() {
        let mut g = game_at(
            "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            (4, 0),
        );
        g.confirm(t()); // king e1
        g.cursor = (6, 0); // g1
        assert!(g.confirm(t()));
        assert_eq!(g.history.last().unwrap().san, "O-O");
    }

    #[test]
    fn notation_disambiguation() {
        let check = |fen: &str, from: Square, to: Square, expected: &str| {
            let board = Board::from_str(fen).unwrap();
            assert_eq!(san(&board, ChessMove::new(from, to, None)), expected);
        };
        // Knights b1 and f3 both reach d2
        check(
            "4k3/8/8/8/8/5N2/8/1N2K3 w - - 0 1",
            Square::B1,
            Square::D2,
            "Nbd2",
        );
        // Rooks a1 and a5 both reach a3
        check(
            "4k3/8/8/R7/8/8/8/R3K3 w - - 0 1",
            Square::A1,
            Square::A3,
            "R1a3",
        );
        // Queens d1, d3 and f1 all reach f3; d1 shares a file with d3 and a rank with f1
        check(
            "4k3/8/8/8/8/3Q4/8/3Q1QK1 w - - 0 1",
            Square::D1,
            Square::F3,
            "Qd1f3",
        );
    }

    #[test]
    fn search_request_replays_to_current_board() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]);
        let req = g.search_request(7, Level::MAX, SearchLimit::MoveTime(Duration::ZERO));
        assert_eq!(req.id, 7);
        assert_eq!(req.root, Board::default());
        assert_eq!(req.replay().0, g.board);
    }
}
