//! Built-in engine: iterative-deepening alpha-beta (PVS) with a transposition table, quiescence
//! search, killer moves and a tapered PeSTO evaluation.

use std::time::{Duration, Instant};

use chess::{ALL_PIECES, Board, ChessMove, Color, EMPTY, MoveGen, Piece};

use crate::engine::{Eval, SearchRequest, SearchResult};

const INF: i32 = 32_000;
const MATE: i32 = 30_000;
/// Scores beyond this magnitude are mate scores.
const MATE_BOUND: i32 = MATE - 1_000;
const MAX_PLY: usize = 128;
const TT_SIZE: usize = 1 << 19;
/// Nodes between clock checks.
const CLOCK_INTERVAL: u64 = 2048;

// PeSTO evaluation (Rofchade values), from https://www.chessprogramming.org/PeSTO%27s_Evaluation_Function
// Indexed by `Piece::to_index()`; tables are laid out a8 = 0 .. h1 = 63 as printed there.
const MG_VALUE: [i32; 6] = [82, 337, 365, 477, 1025, 0];
const EG_VALUE: [i32; 6] = [94, 281, 297, 512, 936, 0];
const PHASE_INC: [i32; 6] = [0, 1, 1, 2, 4, 0];

#[rustfmt::skip]
const MG_PAWN: [i32; 64] = [
      0,   0,   0,   0,   0,   0,  0,   0,
     98, 134,  61,  95,  68, 126, 34, -11,
     -6,   7,  26,  31,  65,  56, 25, -20,
    -14,  13,   6,  21,  23,  12, 17, -23,
    -27,  -2,  -5,  12,  17,   6, 10, -25,
    -26,  -4,  -4, -10,   3,   3, 33, -12,
    -35,  -1, -20, -23, -15,  24, 38, -22,
      0,   0,   0,   0,   0,   0,  0,   0,
];
#[rustfmt::skip]
const EG_PAWN: [i32; 64] = [
      0,   0,   0,   0,   0,   0,   0,   0,
    178, 173, 158, 134, 147, 132, 165, 187,
     94, 100,  85,  67,  56,  53,  82,  84,
     32,  24,  13,   5,  -2,   4,  17,  17,
     13,   9,  -3,  -7,  -7,  -8,   3,  -1,
      4,   7,  -6,   1,   0,  -5,  -1,  -8,
     13,   8,   8,  10,  13,   0,   2,  -7,
      0,   0,   0,   0,   0,   0,   0,   0,
];
#[rustfmt::skip]
const MG_KNIGHT: [i32; 64] = [
    -167, -89, -34, -49,  61, -97, -15, -107,
     -73, -41,  72,  36,  23,  62,   7,  -17,
     -47,  60,  37,  65,  84, 129,  73,   44,
      -9,  17,  19,  53,  37,  69,  18,   22,
     -13,   4,  16,  13,  28,  19,  21,   -8,
     -23,  -9,  12,  10,  19,  17,  25,  -16,
     -29, -53, -12,  -3,  -1,  18, -14,  -19,
    -105, -21, -58, -33, -17, -28, -19,  -23,
];
#[rustfmt::skip]
const EG_KNIGHT: [i32; 64] = [
    -58, -38, -13, -28, -31, -27, -63, -99,
    -25,  -8, -25,  -2,  -9, -25, -24, -52,
    -24, -20,  10,   9,  -1,  -9, -19, -41,
    -17,   3,  22,  22,  22,  11,   8, -18,
    -18,  -6,  16,  25,  16,  17,   4, -18,
    -23,  -3,  -1,  15,  10,  -3, -20, -22,
    -42, -20, -10,  -5,  -2, -20, -23, -44,
    -29, -51, -23, -15, -22, -18, -50, -64,
];
#[rustfmt::skip]
const MG_BISHOP: [i32; 64] = [
    -29,   4, -82, -37, -25, -42,   7,  -8,
    -26,  16, -18, -13,  30,  59,  18, -47,
    -16,  37,  43,  40,  35,  50,  37,  -2,
     -4,   5,  19,  50,  37,  37,   7,  -2,
     -6,  13,  13,  26,  34,  12,  10,   4,
      0,  15,  15,  15,  14,  27,  18,  10,
      4,  15,  16,   0,   7,  21,  33,   1,
    -33,  -3, -14, -21, -13, -12, -39, -21,
];
#[rustfmt::skip]
const EG_BISHOP: [i32; 64] = [
    -14, -21, -11,  -8, -7,  -9, -17, -24,
     -8,  -4,   7, -12, -3, -13,  -4, -14,
      2,  -8,   0,  -1, -2,   6,   0,   4,
     -3,   9,  12,   9, 14,  10,   3,   2,
     -6,   3,  13,  19,  7,  10,  -3,  -9,
    -12,  -3,   8,  10, 13,   3,  -7, -15,
    -14, -18,  -7,  -1,  4,  -9, -15, -27,
    -23,  -9, -23,  -5, -9, -16,  -5, -17,
];
#[rustfmt::skip]
const MG_ROOK: [i32; 64] = [
     32,  42,  32,  51, 63,  9,  31,  43,
     27,  32,  58,  62, 80, 67,  26,  44,
     -5,  19,  26,  36, 17, 45,  61,  16,
    -24, -11,   7,  26, 24, 35,  -8, -20,
    -36, -26, -12,  -1,  9, -7,   6, -23,
    -45, -25, -16, -17,  3,  0,  -5, -33,
    -44, -16, -20,  -9, -1, 11,  -6, -71,
    -19, -13,   1,  17, 16,  7, -37, -26,
];
#[rustfmt::skip]
const EG_ROOK: [i32; 64] = [
    13, 10, 18, 15, 12,  12,   8,   5,
    11, 13, 13, 11, -3,   3,   8,   3,
     7,  7,  7,  5,  4,  -3,  -5,  -3,
     4,  3, 13,  1,  2,   1,  -1,   2,
     3,  5,  8,  4, -5,  -6,  -8, -11,
    -4,  0, -5, -1, -7, -12,  -8, -16,
    -6, -6,  0,  2, -9,  -9, -11,  -3,
    -9,  2,  3, -1, -5, -13,   4, -20,
];
#[rustfmt::skip]
const MG_QUEEN: [i32; 64] = [
    -28,   0,  29,  12,  59,  44,  43,  45,
    -24, -39,  -5,   1, -16,  57,  28,  54,
    -13, -17,   7,   8,  29,  56,  47,  57,
    -27, -27, -16, -16,  -1,  17,  -2,   1,
     -9, -26,  -9, -10,  -2,  -4,   3,  -3,
    -14,   2, -11,  -2,  -5,   2,  14,   5,
    -35,  -8,  11,   2,   8,  15,  -3,   1,
     -1, -18,  -9,  10, -15, -25, -31, -50,
];
#[rustfmt::skip]
const EG_QUEEN: [i32; 64] = [
     -9,  22,  22,  27,  27,  19,  10,  20,
    -17,  20,  32,  41,  58,  25,  30,   0,
    -20,   6,   9,  49,  47,  35,  19,   9,
      3,  22,  24,  45,  57,  40,  57,  36,
    -18,  28,  19,  47,  31,  34,  39,  23,
    -16, -27,  15,   6,   9,  17,  10,   5,
    -22, -23, -30, -16, -16, -23, -36, -32,
    -33, -28, -22, -43,  -5, -32, -20, -41,
];
#[rustfmt::skip]
const MG_KING: [i32; 64] = [
    -65,  23,  16, -15, -56, -34,   2,  13,
     29,  -1, -20,  -7,  -8,  -4, -38, -29,
     -9,  24,   2, -16, -20,   6,  22, -22,
    -17, -20, -12, -27, -30, -25, -14, -36,
    -49,  -1, -27, -39, -46, -44, -33, -51,
    -14, -14, -22, -46, -44, -30, -15, -27,
      1,   7,  -8, -64, -43, -16,   9,   8,
    -15,  36,  12, -54,   8, -28,  24,  14,
];
#[rustfmt::skip]
const EG_KING: [i32; 64] = [
    -74, -35, -18, -18, -11,  15,   4, -17,
    -12,  17,  14,  17,  17,  38,  23,  11,
     10,  17,  23,  15,  20,  45,  44,  13,
     -8,  22,  24,  27,  26,  33,  26,   3,
    -18,  -4,  21,  24,  27,  23,   9, -11,
    -19,  -3,  11,  21,  23,  16,   7,  -9,
    -27, -11,   4,  13,  14,   4,  -5, -17,
    -53, -34, -21, -11, -28, -14, -24, -43,
];

const MG_TABLE: [[i32; 64]; 6] = [MG_PAWN, MG_KNIGHT, MG_BISHOP, MG_ROOK, MG_QUEEN, MG_KING];
const EG_TABLE: [[i32; 64]; 6] = [EG_PAWN, EG_KNIGHT, EG_BISHOP, EG_ROOK, EG_QUEEN, EG_KING];

/// Tapered PeSTO evaluation from the side to move's point of view.
fn evaluate(board: &Board) -> i32 {
    let mut mg = [0i32; 2];
    let mut eg = [0i32; 2];
    let mut phase = 0;
    for piece in ALL_PIECES {
        let p = piece.to_index();
        for color in [Color::White, Color::Black] {
            for sq in *board.pieces(piece) & *board.color_combined(color) {
                // Tables are printed a8-first; chess squares are a1-first.
                let idx = match color {
                    Color::White => sq.to_index() ^ 56,
                    Color::Black => sq.to_index(),
                };
                mg[color.to_index()] += MG_VALUE[p] + MG_TABLE[p][idx];
                eg[color.to_index()] += EG_VALUE[p] + EG_TABLE[p][idx];
                phase += PHASE_INC[p];
            }
        }
    }
    let us = board.side_to_move().to_index();
    let them = 1 - us;
    let mg_phase = phase.min(24);
    ((mg[us] - mg[them]) * mg_phase + (eg[us] - eg[them]) * (24 - mg_phase)) / 24
}

fn piece_value(piece: Piece) -> i32 {
    MG_VALUE[piece.to_index()]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, Copy)]
struct TtEntry {
    key: u64,
    mv: Option<ChessMove>,
    score: i32,
    depth: i32,
    bound: Bound,
}

/// Mate scores are stored relative to the node, not the root.
fn score_to_tt(score: i32, ply: usize) -> i32 {
    if score > MATE_BOUND {
        score + ply as i32
    } else if score < -MATE_BOUND {
        score - ply as i32
    } else {
        score
    }
}

fn score_from_tt(score: i32, ply: usize) -> i32 {
    if score > MATE_BOUND {
        score - ply as i32
    } else if score < -MATE_BOUND {
        score + ply as i32
    } else {
        score
    }
}

struct Searcher {
    tt: Vec<Option<TtEntry>>,
    killers: [[Option<ChessMove>; 2]; MAX_PLY],
    /// Hashes of earlier game positions, then of the current search path.
    path: Vec<u64>,
    deadline: Instant,
    nodes: u64,
    /// Clock checks are off until one full iteration has completed.
    can_stop: bool,
    stopped: bool,
    root_best: Option<ChessMove>,
}

impl Searcher {
    fn out_of_time(&mut self) -> bool {
        self.nodes += 1;
        if self.can_stop
            && self.nodes.is_multiple_of(CLOCK_INTERVAL)
            && Instant::now() >= self.deadline
        {
            self.stopped = true;
        }
        self.stopped
    }

    fn tt_probe(&self, key: u64) -> Option<TtEntry> {
        self.tt[key as usize % TT_SIZE].filter(|e| e.key == key)
    }

    fn tt_store(&mut self, entry: TtEntry) {
        self.tt[entry.key as usize % TT_SIZE] = Some(entry);
    }

    fn order(
        &self,
        board: &Board,
        moves: &mut [ChessMove],
        tt_move: Option<ChessMove>,
        ply: usize,
    ) {
        let killers = self.killers.get(ply).copied().unwrap_or([None; 2]);
        moves.sort_by_cached_key(|&mv| {
            let score = if Some(mv) == tt_move {
                1_000_000
            } else if let Some(victim) = board.piece_on(mv.get_dest()) {
                // MVV-LVA
                let attacker = board.piece_on(mv.get_source()).map_or(0, piece_value);
                100_000 + 10 * piece_value(victim) - attacker
            } else if mv.get_promotion() == Some(Piece::Queen) {
                90_000
            } else if Some(mv) == killers[0] {
                80_000
            } else if Some(mv) == killers[1] {
                79_000
            } else {
                0
            };
            -score
        });
    }

    fn negamax(&mut self, board: &Board, depth: i32, ply: usize, mut alpha: i32, beta: i32) -> i32 {
        if self.out_of_time() {
            return 0;
        }
        let hash = board.get_hash();
        if ply > 0 && self.path.contains(&hash) {
            return 0; // repetition
        }
        let in_check = *board.checkers() != EMPTY;
        let depth = if in_check { depth + 1 } else { depth };
        if depth <= 0 || ply >= MAX_PLY - 1 {
            return self.quiesce(board, ply, alpha, beta);
        }

        let tt_entry = self.tt_probe(hash);
        if let Some(e) = tt_entry
            && ply > 0
            && e.depth >= depth
        {
            let score = score_from_tt(e.score, ply);
            match e.bound {
                Bound::Exact => return score,
                Bound::Lower if score >= beta => return score,
                Bound::Upper if score <= alpha => return score,
                _ => {}
            }
        }

        let mut moves: Vec<ChessMove> = MoveGen::new_legal(board).collect();
        if moves.is_empty() {
            return if in_check { -MATE + ply as i32 } else { 0 };
        }
        self.order(board, &mut moves, tt_entry.and_then(|e| e.mv), ply);

        let orig_alpha = alpha;
        let mut best = -INF;
        let mut best_move = None;
        self.path.push(hash);
        for (i, &mv) in moves.iter().enumerate() {
            let next = board.make_move_new(mv);
            let score = if i == 0 {
                -self.negamax(&next, depth - 1, ply + 1, -beta, -alpha)
            } else {
                // Principal variation search: null window first, re-search if it may improve.
                let s = -self.negamax(&next, depth - 1, ply + 1, -alpha - 1, -alpha);
                if s > alpha && s < beta {
                    -self.negamax(&next, depth - 1, ply + 1, -beta, -alpha)
                } else {
                    s
                }
            };
            if self.stopped {
                self.path.pop();
                return 0;
            }
            if score > best {
                best = score;
                best_move = Some(mv);
                if score > alpha {
                    alpha = score;
                    if alpha >= beta {
                        if board.piece_on(mv.get_dest()).is_none() && ply < MAX_PLY {
                            let k = &mut self.killers[ply];
                            if k[0] != Some(mv) {
                                k[1] = k[0];
                                k[0] = Some(mv);
                            }
                        }
                        break;
                    }
                }
            }
        }
        self.path.pop();

        let bound = if best <= orig_alpha {
            Bound::Upper
        } else if best >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.tt_store(TtEntry {
            key: hash,
            mv: best_move,
            score: score_to_tt(best, ply),
            depth,
            bound,
        });
        if ply == 0 {
            self.root_best = best_move;
        }
        best
    }

    /// Searches captures only (all evasions when in check) until the position is quiet.
    fn quiesce(&mut self, board: &Board, ply: usize, mut alpha: i32, beta: i32) -> i32 {
        if self.out_of_time() {
            return 0;
        }
        let in_check = *board.checkers() != EMPTY;
        if ply >= MAX_PLY - 1 {
            return evaluate(board);
        }
        let mut movegen = MoveGen::new_legal(board);
        let mut best = if in_check {
            -MATE + ply as i32
        } else {
            let stand_pat = evaluate(board);
            if stand_pat >= beta {
                return stand_pat;
            }
            alpha = alpha.max(stand_pat);
            movegen.set_iterator_mask(*board.color_combined(!board.side_to_move()));
            stand_pat
        };
        let mut moves: Vec<ChessMove> = movegen.collect();
        if in_check && moves.is_empty() {
            return best; // checkmate
        }
        self.order(board, &mut moves, None, ply);
        for mv in moves {
            let score = -self.quiesce(&board.make_move_new(mv), ply + 1, -beta, -alpha);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    if alpha >= beta {
                        break;
                    }
                }
            }
        }
        best
    }
}

/// Score from the side to move's point of view, as an `Eval`.
fn to_eval(score: i32) -> Eval {
    if score > MATE_BOUND {
        Eval::Mate((MATE - score + 1) / 2)
    } else if score < -MATE_BOUND {
        Eval::Mate(-(MATE + score + 1) / 2)
    } else {
        Eval::Cp(score)
    }
}

/// Best move for the current position of `req`, searching for about `limit`.
/// Returns `None` when there is no legal move.
pub fn search(req: &SearchRequest, limit: Duration) -> Option<SearchResult> {
    let (board, history) = req.replay();
    let first = MoveGen::new_legal(&board).next()?;
    let mut searcher = Searcher {
        tt: vec![None; TT_SIZE],
        killers: [[None; 2]; MAX_PLY],
        path: history,
        deadline: Instant::now() + limit,
        nodes: 0,
        can_stop: false,
        stopped: false,
        root_best: None,
    };
    let mut result = SearchResult {
        mv: first,
        eval: to_eval(evaluate(&board)).for_white(board.side_to_move()),
        depth: 0,
    };
    for depth in 1..MAX_PLY as i32 {
        let score = searcher.negamax(&board, depth, 0, -INF, INF);
        if searcher.stopped {
            break; // keep the last completed iteration
        }
        if let Some(mv) = searcher.root_best {
            result = SearchResult {
                mv,
                eval: to_eval(score).for_white(board.side_to_move()),
                depth: depth as u32,
            };
        }
        searcher.can_stop = true;
        if score.abs() > MATE_BOUND || Instant::now() >= searcher.deadline {
            break;
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    const LIMIT: Duration = Duration::from_millis(300);

    fn best(fen: &str) -> (Board, SearchResult) {
        let board = Board::from_str(fen).unwrap();
        let req = SearchRequest {
            root: board,
            moves: Vec::new(),
        };
        (board, search(&req, LIMIT).expect("must find a move"))
    }

    #[test]
    fn best_move_is_legal_from_start() {
        let board = Board::default();
        let req = SearchRequest {
            root: board,
            moves: Vec::new(),
        };
        let res = search(&req, LIMIT).expect("must find a move");
        assert!(MoveGen::new_legal(&board).any(|m| m == res.mv));
        assert!(res.depth >= 1);
    }

    #[test]
    fn finds_mate_in_one() {
        // 1.e4 e5 2.Bc4 Nc6 3.Qh5 Nf6 4.Qxf7#
        let (board, res) =
            best("r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 0 4");
        assert_eq!(
            board.make_move_new(res.mv).status(),
            chess::BoardStatus::Checkmate
        );
        assert_eq!(res.eval, Eval::Mate(1));
    }

    #[test]
    fn black_mate_reported_from_white_view() {
        // Fool's mate setup: 1.f3 e5 2.g4, Black plays Qh4#.
        let (_, res) = best("rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2");
        assert_eq!(res.mv, ChessMove::from_str("d8h4").unwrap());
        assert_eq!(res.eval, Eval::Mate(-1));
    }

    #[test]
    fn takes_hanging_queen() {
        let (board, res) = best("7k/8/8/3q4/8/3Q4/8/K7 w - - 0 1");
        let after = board.make_move_new(res.mv);
        assert_eq!(after.piece_on(chess::Square::D5), Some(Piece::Queen));
        assert_eq!(after.color_on(chess::Square::D5), Some(Color::White));
    }

    #[test]
    fn no_move_when_mated() {
        let board =
            Board::from_str("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
                .unwrap();
        let req = SearchRequest {
            root: board,
            moves: Vec::new(),
        };
        assert!(search(&req, LIMIT).is_none());
    }

    #[test]
    fn start_position_is_balanced() {
        assert_eq!(evaluate(&Board::default()), 0);
    }

    #[test]
    fn mirrored_positions_evaluate_equally() {
        // Same position with colours swapped and ranks mirrored, other side to move.
        let a = Board::from_str("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
            .unwrap();
        let b = Board::from_str("rnbqkb1r/pppp1ppp/5n2/4p3/4P3/2N5/PPPP1PPP/R1BQKBNR b KQkq - 2 3")
            .unwrap();
        assert_eq!(evaluate(&a), evaluate(&b));
        assert_ne!(evaluate(&a), 0);
    }

    #[test]
    fn prefers_developed_knight() {
        // Knight on f3 beats knight on h3 for White in PeSTO.
        let f3 =
            Board::from_str("rnbqkbnr/pppppppp/8/8/8/5N2/PPPPPPPP/RNBQKB1R b KQkq - 1 1").unwrap();
        let h3 =
            Board::from_str("rnbqkbnr/pppppppp/8/8/8/7N/PPPPPPPP/RNBQKB1R b KQkq - 1 1").unwrap();
        // Black to move, so a better White position scores lower.
        assert!(evaluate(&f3) < evaluate(&h3));
    }
}
