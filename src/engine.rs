//! Engine front-end: the built-in search, or an external UCI engine (e.g. Stockfish) when one
//! is available. Set `CHESS_ENGINE` to an engine path, or to `builtin` to skip detection.

use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use chess::{Board, ChessMove, Color, MoveGen};

use crate::ai;

const BUILTIN_TIME: Duration = Duration::from_millis(1500);
const UCI_MOVETIME_MS: u64 = 1000;
const UCI_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

/// Position score from White's point of view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Eval {
    /// Centipawns; positive favours White.
    Cp(i32),
    /// Moves to mate; positive means White mates.
    Mate(i32),
}

impl Eval {
    /// Converts a score given from the side to move's point of view to White's.
    pub fn for_white(self, side_to_move: Color) -> Eval {
        if side_to_move == Color::White {
            return self;
        }
        match self {
            Eval::Cp(cp) => Eval::Cp(-cp),
            Eval::Mate(n) => Eval::Mate(-n),
        }
    }
}

/// A game to search: the starting position plus every move played since.
#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub root: Board,
    pub moves: Vec<ChessMove>,
}

impl SearchRequest {
    /// The current position and the hashes of every earlier position in the game.
    pub fn replay(&self) -> (Board, Vec<u64>) {
        let mut board = self.root;
        let mut hashes = Vec::with_capacity(self.moves.len());
        for &mv in &self.moves {
            hashes.push(board.get_hash());
            board = board.make_move_new(mv);
        }
        (board, hashes)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchResult {
    pub mv: ChessMove,
    pub eval: Eval,
    pub depth: u32,
}

pub enum Engine {
    Builtin,
    Uci(UciEngine),
}

impl Engine {
    pub fn detect() -> Engine {
        let path = match std::env::var("CHESS_ENGINE") {
            Ok(v) if v.eq_ignore_ascii_case("builtin") => return Engine::Builtin,
            Ok(v) => v,
            Err(_) => "stockfish".to_string(),
        };
        match UciEngine::start(&path) {
            Some(engine) => Engine::Uci(engine),
            None => Engine::Builtin,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Engine::Builtin => "built-in",
            Engine::Uci(uci) => &uci.name,
        }
    }

    /// Starts searching `req`; the result arrives on the returned channel.
    /// `None` means the position has no legal move.
    pub fn think(&self, req: SearchRequest) -> Receiver<Option<SearchResult>> {
        let (tx, rx) = mpsc::channel();
        match self {
            Engine::Builtin => {
                thread::spawn(move || {
                    let _ = tx.send(ai::search(&req, BUILTIN_TIME));
                });
            }
            Engine::Uci(uci) => {
                // Worker gone (engine thread died) drops `tx`; main sees a disconnect.
                let _ = uci.requests.send((req, tx));
            }
        }
        rx
    }
}

type Request = (SearchRequest, Sender<Option<SearchResult>>);

pub struct UciEngine {
    name: String,
    requests: Sender<Request>,
    child: Child,
}

impl UciEngine {
    /// Spawns the engine and completes the UCI handshake, or returns `None`.
    fn start(path: &str) -> Option<UciEngine> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        let (ready_tx, ready_rx) = mpsc::channel();
        let (req_tx, req_rx) = mpsc::channel::<Request>();

        thread::spawn(move || {
            let mut io = UciIo { stdin, stdout };
            match io.handshake() {
                Ok(name) => {
                    let _ = ready_tx.send(name);
                }
                Err(_) => return,
            }
            // One request at a time, so a stale search never interleaves with a new one.
            for (req, reply) in req_rx {
                let result = io
                    .search(&req)
                    .unwrap_or_else(|_| ai::search(&req, BUILTIN_TIME));
                let _ = reply.send(result);
            }
        });

        match ready_rx.recv_timeout(UCI_HANDSHAKE_TIMEOUT) {
            Ok(name) => Some(UciEngine {
                name,
                requests: req_tx,
                child,
            }),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                None
            }
        }
    }
}

impl Drop for UciEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct UciIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl UciIo {
    fn send(&mut self, cmd: &str) -> io::Result<()> {
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()
    }

    fn read_line(&mut self) -> io::Result<String> {
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(line)
    }

    /// Returns the engine's name.
    fn handshake(&mut self) -> io::Result<String> {
        self.send("uci")?;
        let mut name = "UCI engine".to_string();
        loop {
            let line = self.read_line()?;
            if let Some(n) = line.trim().strip_prefix("id name ") {
                name = n.to_string();
            }
            if line.trim() == "uciok" {
                break;
            }
        }
        self.send("isready")?;
        while self.read_line()?.trim() != "readyok" {}
        Ok(name)
    }

    fn search(&mut self, req: &SearchRequest) -> io::Result<Option<SearchResult>> {
        let mut position = format!("position fen {}", req.root);
        if !req.moves.is_empty() {
            position.push_str(" moves");
            for mv in &req.moves {
                position.push_str(&format!(" {mv}"));
            }
        }
        self.send(&position)?;
        self.send(&format!("go movetime {UCI_MOVETIME_MS}"))?;

        let (board, _) = req.replay();
        let mut last_info: Option<(u32, Eval)> = None;
        let best = loop {
            let line = self.read_line()?;
            if let Some(info) = parse_info(&line) {
                last_info = Some(info);
            }
            if let Some(best) = parse_bestmove(&line) {
                break best.and_then(|s| ChessMove::from_str(s).ok());
            }
        };
        let Some(mv) = best else {
            return Ok(None);
        };
        if !MoveGen::new_legal(&board).any(|m| m == mv) {
            return Ok(None);
        }
        let (depth, eval) = last_info.unwrap_or((0, Eval::Cp(0)));
        Ok(Some(SearchResult {
            mv,
            eval: eval.for_white(board.side_to_move()),
            depth,
        }))
    }
}

/// Parses `info ... depth D ... score cp|mate N ...`; the score is from the side to move's
/// point of view. Lines without a score yield `None`.
fn parse_info(line: &str) -> Option<(u32, Eval)> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "info" {
        return None;
    }
    let mut depth = 0;
    let mut eval = None;
    while let Some(tok) = tokens.next() {
        match tok {
            "depth" => depth = tokens.next()?.parse().ok()?,
            "score" => {
                eval = match (tokens.next()?, tokens.next()?.parse().ok()?) {
                    ("cp", v) => Some(Eval::Cp(v)),
                    ("mate", v) => Some(Eval::Mate(v)),
                    _ => None,
                }
            }
            // Only the move list follows `pv`.
            "pv" => break,
            _ => {}
        }
    }
    Some((depth, eval?))
}

/// Parses `bestmove <move> [ponder <move>]`. Outer `None`: not a bestmove line.
/// Inner `None`: the engine reported no move (`(none)`).
fn parse_bestmove(line: &str) -> Option<Option<&str>> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "bestmove" {
        return None;
    }
    Some(tokens.next().filter(|m| *m != "(none)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_info_cp_and_mate() {
        assert_eq!(
            parse_info("info depth 12 seldepth 18 multipv 1 score cp -34 nodes 1000 pv e2e4 e7e5"),
            Some((12, Eval::Cp(-34)))
        );
        assert_eq!(
            parse_info("info depth 20 score mate 3 pv h5f7"),
            Some((20, Eval::Mate(3)))
        );
        assert_eq!(
            parse_info("info depth 5 currmove e2e4 currmovenumber 1"),
            None
        );
        assert_eq!(parse_info("info string NNUE enabled"), None);
        assert_eq!(parse_info("bestmove e2e4"), None);
    }

    #[test]
    fn parses_bestmove() {
        assert_eq!(
            parse_bestmove("bestmove e7e8q ponder a2a3"),
            Some(Some("e7e8q"))
        );
        assert_eq!(parse_bestmove("bestmove (none)"), Some(None));
        assert_eq!(parse_bestmove("info depth 1 score cp 3"), None);
        let mv = ChessMove::from_str("e7e8q").unwrap();
        assert_eq!(mv.get_promotion(), Some(chess::Piece::Queen));
    }

    #[test]
    fn eval_flips_for_black() {
        assert_eq!(Eval::Cp(50).for_white(Color::Black), Eval::Cp(-50));
        assert_eq!(Eval::Mate(2).for_white(Color::Black), Eval::Mate(-2));
        assert_eq!(Eval::Cp(50).for_white(Color::White), Eval::Cp(50));
    }

    #[test]
    fn replay_tracks_positions() {
        let req = SearchRequest {
            root: Board::default(),
            moves: vec![
                ChessMove::from_str("e2e4").unwrap(),
                ChessMove::from_str("e7e5").unwrap(),
            ],
        };
        let (board, hashes) = req.replay();
        assert_eq!(hashes.len(), 2);
        assert_eq!(hashes[0], Board::default().get_hash());
        assert_eq!(board.side_to_move(), Color::White);
    }

    #[test]
    fn missing_engine_falls_back() {
        assert!(UciEngine::start("definitely-not-a-chess-engine-xyz").is_none());
    }
}
