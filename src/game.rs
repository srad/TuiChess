use chess::{Board, BoardStatus, ChessMove, Color, EMPTY, File, MoveGen, Piece, Rank, Square};

use crate::engine::{Eval, SearchRequest};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Selected(Square),
    Promoting { from: Square, to: Square },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Checkmate { winner: Color },
    Stalemate,
    Repetition,
    FiftyMove,
    InsufficientMaterial,
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
    pub human: Color,
    pub cursor: (u8, u8), // (file 0..=7, rank 0..=7), absolute: rank 0 = rank 1
    pub phase: Phase,
    pub legal_targets: Vec<Square>,
    pub history: Vec<Ply>,
    /// Last engine score, White's point of view.
    pub eval: Option<Eval>,
}

impl Game {
    pub fn new(human: Color) -> Self {
        Self {
            board: Board::default(),
            human,
            cursor: if human == Color::White {
                (4, 1)
            } else {
                (4, 6)
            },
            phase: Phase::Idle,
            legal_targets: Vec::new(),
            history: Vec::new(),
            eval: None,
        }
    }

    pub fn restart(&mut self) {
        *self = Game::new(self.human);
    }

    pub fn last_move(&self) -> Option<(Square, Square)> {
        self.history
            .last()
            .map(|p| (p.mv.get_source(), p.mv.get_dest()))
    }

    pub fn is_human_turn(&self) -> bool {
        self.board.side_to_move() == self.human
    }

    pub fn outcome(&self) -> Option<Outcome> {
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

    /// Plies since the last pawn move or capture.
    fn halfmove_clock(&self) -> usize {
        self.history
            .iter()
            .rev()
            .take_while(|p| {
                p.before.piece_on(p.mv.get_source()) != Some(Piece::Pawn)
                    && p.before.piece_on(p.mv.get_dest()).is_none()
            })
            .count()
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

    pub fn status_text(&self) -> String {
        let text = match self.outcome() {
            Some(Outcome::Checkmate { winner }) if winner == self.human => "Checkmate — you win!",
            Some(Outcome::Checkmate { .. }) => "Checkmate — AI wins.",
            Some(Outcome::Stalemate) => "Draw — stalemate.",
            Some(Outcome::Repetition) => "Draw — threefold repetition.",
            Some(Outcome::FiftyMove) => "Draw — fifty-move rule.",
            Some(Outcome::InsufficientMaterial) => "Draw — insufficient material.",
            None if *self.board.checkers() != EMPTY && self.is_human_turn() => "Check! Your move.",
            None if *self.board.checkers() != EMPTY => "Check!",
            None if self.is_human_turn() => "Your move.",
            None => "AI to move.",
        };
        text.to_string()
    }

    /// Input for the engine: the starting position plus every move since.
    pub fn search_request(&self) -> SearchRequest {
        SearchRequest {
            root: self.history.first().map_or(self.board, |p| p.before),
            moves: self.history.iter().map(|p| p.mv).collect(),
        }
    }

    pub fn cursor_square(&self) -> Square {
        Square::make_square(
            Rank::from_index(self.cursor.1 as usize),
            File::from_index(self.cursor.0 as usize),
        )
    }

    /// Moves the cursor in screen directions (up = away from the human player).
    pub fn move_cursor(&mut self, df: i8, dr: i8) {
        let (df, dr) = if self.human == Color::White {
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

    /// Takes back moves until the human's last move is undone. Returns false if there is none.
    pub fn undo(&mut self) -> bool {
        let Some(i) = self
            .history
            .iter()
            .rposition(|p| p.before.side_to_move() == self.human)
        else {
            return false;
        };
        self.board = self.history[i].before;
        self.history.truncate(i);
        self.eval = None;
        self.cancel();
        true
    }

    /// Enter pressed. Returns true if a human move was made.
    pub fn confirm(&mut self) -> bool {
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
                self.apply_move(ChessMove::new(from, cur, None));
                true
            }
            Phase::Promoting { .. } => false,
        }
    }

    /// Called during Promoting phase with chosen piece. Returns true if move applied.
    pub fn promote(&mut self, piece: Piece) -> bool {
        if let Phase::Promoting { from, to } = self.phase {
            let mv = ChessMove::new(from, to, Some(piece));
            if MoveGen::new_legal(&self.board).any(|m| m == mv) {
                self.apply_move(mv);
                return true;
            }
            self.cancel();
        }
        false
    }

    /// Applies an engine move if it is still the AI's turn in the searched position.
    pub fn apply_engine_move(&mut self, searched: &Board, mv: ChessMove) -> bool {
        if *searched != self.board
            || self.is_human_turn()
            || !MoveGen::new_legal(&self.board).any(|m| m == mv)
        {
            return false;
        }
        self.apply_move(mv);
        true
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

    fn apply_move(&mut self, mv: ChessMove) {
        let notation = self.notate(&mv);
        let before = self.board;
        self.board = before.make_move_new(mv);
        let suffix = match self.board.status() {
            BoardStatus::Checkmate => "#",
            _ if *self.board.checkers() != EMPTY => "+",
            _ => "",
        };
        self.history.push(Ply {
            before,
            mv,
            san: format!("{notation}{suffix}"),
        });
        self.cancel();
    }

    /// Standard algebraic notation without the check suffix, e.g. "Nbd2", "exd5", "e8=Q", "O-O".
    /// Called BEFORE the move is applied.
    fn notate(&self, mv: &ChessMove) -> String {
        let from = mv.get_source();
        let to = mv.get_dest();
        let piece = self.board.piece_on(from).unwrap_or(Piece::Pawn);

        // Castling: king moving two files
        if piece == Piece::King
            && (to.get_file().to_index() as i8 - from.get_file().to_index() as i8).abs() == 2
        {
            return if to.get_file().to_index() > from.get_file().to_index() {
                "O-O".to_string()
            } else {
                "O-O-O".to_string()
            };
        }

        // Capture: occupied target, or en-passant (pawn, diagonal, empty target)
        let capture = self.board.piece_on(to).is_some()
            || (piece == Piece::Pawn && from.get_file() != to.get_file());

        let mut s = String::new();
        match piece {
            Piece::Pawn => {
                if capture {
                    s.push(file_char(from));
                }
            }
            _ => {
                s.push(piece_letter(piece));
                s.push_str(&self.disambiguation(piece, from, to));
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
        s
    }

    /// File, rank, or both of `from` when another `piece` can also reach `to`.
    fn disambiguation(&self, piece: Piece, from: Square, to: Square) -> String {
        let rivals: Vec<Square> = MoveGen::new_legal(&self.board)
            .filter(|m| {
                m.get_dest() == to
                    && m.get_source() != from
                    && self.board.piece_on(m.get_source()) == Some(piece)
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
mod tests {
    use super::*;
    use std::str::FromStr;

    fn sq(f: u8, r: u8) -> Square {
        Square::make_square(Rank::from_index(r as usize), File::from_index(f as usize))
    }

    fn white_game() -> Game {
        Game::new(Color::White)
    }

    fn game_at(fen: &str, cursor: (u8, u8)) -> Game {
        let mut g = white_game();
        g.board = Board::from_str(fen).unwrap();
        g.cursor = cursor;
        g
    }

    /// (file, rank), both 0..=7.
    type Coord = (u8, u8);

    /// Plays moves through the cursor UI (human) or `apply_engine_move` (AI).
    fn play(g: &mut Game, moves: &[(Coord, Coord)]) {
        for &(a, b) in moves {
            if g.is_human_turn() {
                g.cursor = a;
                g.confirm();
                g.cursor = b;
                assert!(g.confirm(), "human move {a:?}->{b:?} rejected");
            } else {
                let before = g.board;
                assert!(
                    g.apply_engine_move(&before, ChessMove::new(sq(a.0, a.1), sq(b.0, b.1), None))
                );
            }
        }
    }

    #[test]
    fn select_own_piece_shows_targets() {
        let mut g = white_game();
        g.cursor = (4, 1); // e2
        assert!(!g.confirm()); // select, not a move
        assert_eq!(g.phase, Phase::Selected(sq(4, 1)));
        assert!(g.legal_targets.contains(&sq(4, 2))); // e3
        assert!(g.legal_targets.contains(&sq(4, 3))); // e4
    }

    #[test]
    fn confirm_target_applies_move() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm(); // select e2
        g.cursor = (4, 3); // e4
        assert!(g.confirm()); // move applied
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
        assert!(!g.confirm());
        assert_eq!(g.phase, Phase::Idle);
        assert_eq!(g.board, Board::default());
    }

    #[test]
    fn enter_on_own_other_piece_reselects() {
        let mut g = white_game();
        g.cursor = (4, 1); // e2 pawn
        g.confirm();
        g.cursor = (3, 1); // d2 pawn
        assert!(!g.confirm());
        assert_eq!(g.phase, Phase::Selected(sq(3, 1)));
    }

    #[test]
    fn enter_on_selected_square_deselects() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm();
        assert!(!g.confirm()); // same square again
        assert_eq!(g.phase, Phase::Idle);
        assert!(g.legal_targets.is_empty());
    }

    #[test]
    fn cancel_clears_selection() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm();
        g.cancel();
        assert_eq!(g.phase, Phase::Idle);
        assert!(g.legal_targets.is_empty());
    }

    #[test]
    fn illegal_target_rejected() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm(); // select e2
        g.cursor = (4, 4); // e5 is not a legal pawn move
        assert!(!g.confirm());
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
    fn cursor_directions_flip_for_black() {
        let mut g = Game::new(Color::Black);
        assert_eq!(g.cursor, (4, 6)); // e7
        g.move_cursor(1, 0); // screen right = towards the a-file
        g.move_cursor(0, 1); // screen up = towards rank 1
        assert_eq!(g.cursor, (3, 5));
    }

    #[test]
    fn promotion_flow() {
        let mut g = game_at("7k/P7/8/8/8/8/8/K7 w - - 0 1", (0, 6)); // a7
        g.confirm(); // select pawn
        g.cursor = (0, 7); // a8
        assert!(!g.confirm()); // enters promotion phase, no move yet
        assert_eq!(
            g.phase,
            Phase::Promoting {
                from: sq(0, 6),
                to: sq(0, 7)
            }
        );
        assert!(g.promote(Piece::Queen));
        assert_eq!(g.board.piece_on(sq(0, 7)), Some(Piece::Queen));
        assert_eq!(g.board.color_on(sq(0, 7)), Some(Color::White));
        assert_eq!(g.phase, Phase::Idle);
        assert_eq!(g.history.last().unwrap().san, "a8=Q+");
    }

    #[test]
    fn promotion_choice_is_legality_checked() {
        let mut g = game_at("7k/P7/8/8/8/8/8/K7 w - - 0 1", (0, 6));
        g.confirm();
        g.cursor = (0, 7);
        g.confirm();
        // all four promotions legal here
        assert!(g.promote(Piece::Knight));
        assert_eq!(g.board.piece_on(sq(0, 7)), Some(Piece::Knight));
    }

    #[test]
    fn input_ignored_on_ai_turn() {
        let mut g = Game::new(Color::Black); // White (AI) to move
        g.cursor = (4, 6);
        assert!(!g.confirm());
        assert_eq!(g.phase, Phase::Idle);
    }

    #[test]
    fn stale_engine_move_rejected() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3))]); // 1.e4
        let searched = g.board;
        g.undo();
        // Human's turn again: the result for the old position must not apply.
        assert!(!g.apply_engine_move(&searched, ChessMove::new(sq(4, 6), sq(4, 4), None)));
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
        assert!(!g.confirm());
    }

    #[test]
    fn check_status_text() {
        // White king e1 in check along file by black rook e2
        let g = game_at("4k3/8/8/8/8/8/4r3/4K3 w - - 0 1", (0, 0));
        assert!(!g.game_over());
        assert!(g.status_text().contains("Check"));
    }

    #[test]
    fn restart_resets_state_and_keeps_side() {
        let mut g = Game::new(Color::Black);
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]);
        g.restart();
        assert_eq!(g.board, Board::default());
        assert_eq!(g.phase, Phase::Idle);
        assert!(g.history.is_empty());
        assert_eq!(g.human, Color::Black);
    }

    #[test]
    fn undo_takes_back_human_and_ai_move() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]); // 1.e4 e5
        assert!(g.undo());
        assert_eq!(g.board, Board::default());
        assert!(g.history.is_empty());
        assert!(!g.undo());
    }

    #[test]
    fn undo_while_ai_thinking_takes_back_one_ply() {
        let mut g = white_game();
        play(
            &mut g,
            &[((4, 1), (4, 3)), ((4, 6), (4, 4)), ((6, 0), (5, 2))],
        ); // 1.e4 e5 2.Nf3
        assert!(g.undo());
        assert_eq!(g.history.len(), 2);
        assert!(g.is_human_turn());
    }

    #[test]
    fn undo_as_black_needs_a_human_move() {
        let mut g = Game::new(Color::Black);
        play(&mut g, &[((4, 1), (4, 3))]); // AI 1.e4
        assert!(!g.undo());
        assert_eq!(g.history.len(), 1);
        play(&mut g, &[((4, 6), (4, 4)), ((6, 0), (5, 2))]); // 1...e5 2.Nf3
        assert!(g.undo());
        assert_eq!(g.history.len(), 1);
        assert!(g.is_human_turn());
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
    fn fifty_move_rule() {
        // 100 quiet plies (a rook move, repeated) — the fifty-move rule is checked before
        // repetition, so identical plies are enough to exercise it.
        let board = Board::from_str("7k/8/8/8/8/8/8/KR6 w - - 0 1").unwrap();
        let ply = Ply {
            before: board,
            mv: ChessMove::new(sq(1, 0), sq(1, 1), None),
            san: String::new(),
        };
        let mut g = white_game();
        g.board = board;
        g.history = vec![ply; 100];
        assert_eq!(g.outcome(), Some(Outcome::FiftyMove));
        g.history.truncate(99);
        assert_ne!(g.outcome(), Some(Outcome::FiftyMove));
        // A pawn move resets the clock.
        let mut h = white_game();
        play(
            &mut h,
            &[((6, 0), (5, 2)), ((6, 7), (5, 5)), ((4, 1), (4, 3))],
        );
        assert_eq!(h.halfmove_clock(), 0);
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
    fn move_history_notation() {
        let mut g = white_game();
        play(
            &mut g,
            &[((4, 1), (4, 3)), ((4, 6), (4, 4)), ((6, 0), (5, 2))],
        ); // 1.e4 e5 2.Nf3
        let sans: Vec<&str> = g.history.iter().map(|p| p.san.as_str()).collect();
        assert_eq!(sans, vec!["e4", "e5", "Nf3"]);
        g.restart();
        assert!(g.history.is_empty());
    }

    #[test]
    fn notation_capture() {
        // white pawn e4 can capture black pawn d5: exd5
        let mut g = game_at(
            "rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1",
            (4, 3), // e4
        );
        g.confirm();
        g.cursor = (3, 4); // d5
        assert!(g.confirm());
        assert_eq!(g.history.last().unwrap().san, "exd5");
    }

    #[test]
    fn notation_castling() {
        let mut g = game_at(
            "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            (4, 0),
        );
        g.confirm(); // king e1
        g.cursor = (6, 0); // g1
        assert!(g.confirm());
        assert_eq!(g.history.last().unwrap().san, "O-O");
    }

    #[test]
    fn notation_disambiguation() {
        // Knights b1 and f3 both reach d2: Nbd2
        let mut g = game_at("4k3/8/8/8/8/5N2/8/1N2K3 w - - 0 1", (1, 0));
        g.confirm();
        g.cursor = (3, 1);
        assert!(g.confirm());
        assert_eq!(g.history.last().unwrap().san, "Nbd2");

        // Rooks a1 and a5 both reach a3: R1a3
        let mut g = game_at("4k3/8/8/R7/8/8/8/R3K3 w - - 0 1", (0, 0));
        g.confirm();
        g.cursor = (0, 2);
        assert!(g.confirm());
        assert_eq!(g.history.last().unwrap().san, "R1a3");

        // Queens d1, d3 and f1 all reach f3; d1 shares a file with d3 and a rank with f1: Qd1f3
        let mut g = game_at("4k3/8/8/8/8/3Q4/8/3Q1QK1 w - - 0 1", (3, 0));
        g.confirm();
        g.cursor = (5, 2);
        assert!(g.confirm());
        assert_eq!(g.history.last().unwrap().san, "Qd1f3");
    }

    #[test]
    fn search_request_replays_to_current_board() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]);
        let req = g.search_request();
        assert_eq!(req.root, Board::default());
        assert_eq!(req.replay().0, g.board);
    }
}
