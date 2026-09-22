//! Engine front-end: a UCI engine (normally the bundled Stockfish, see `bundled.rs`) or the
//! built-in search. Set `CHESS_ENGINE` to an engine path, or to `builtin` to skip detection.

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chess::{Board, ChessMove, Color, MoveGen};

use crate::ai;
use crate::clock::ClockTimes;

/// Generous: the first start of a ~100 MB engine binary may include an antivirus scan.
const UCI_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const UCI_HASH_MB: u64 = 256;
/// Our pipe and UI tick add latency that Stockfish's 10 ms default does not cover.
const UCI_MOVE_OVERHEAD_MS: u64 = 100;

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

/// Playing strength, 1 (weakest) to 8 (full strength).
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub struct Level(u8);

impl Level {
    pub const MIN: Level = Level(1);
    pub const MAX: Level = Level(8);
    pub const DEFAULT: Level = Level(4);

    const ELO: [u32; 7] = [1320, 1500, 1700, 1900, 2100, 2400, 2800];
    const BUILTIN_MS: [u64; 8] = [50, 100, 150, 250, 400, 600, 800, 1000];

    pub fn new(n: u8) -> Level {
        Level(n.clamp(Self::MIN.0, Self::MAX.0))
    }

    pub fn get(self) -> u8 {
        self.0
    }

    pub fn stronger(self) -> Level {
        Level::new(self.0 + 1)
    }

    pub fn weaker(self) -> Level {
        Level::new(self.0.saturating_sub(1))
    }

    /// Stockfish's `UCI_Elo` for this level; `None` at full strength.
    pub fn elo(self) -> Option<u32> {
        Self::ELO.get(usize::from(self.0 - 1)).copied()
    }

    /// Thinking time of the built-in engine.
    pub fn builtin_time(self) -> Duration {
        Duration::from_millis(Self::BUILTIN_MS[usize::from(self.0 - 1)])
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchLimit {
    MoveTime(Duration),
    Clock(ClockTimes),
}

/// A game to search: the starting position plus every move played since.
#[derive(Clone, Debug)]
pub struct SearchRequest {
    /// Increasing per request; used to cancel.
    pub id: u64,
    pub root: Board,
    pub moves: Vec<ChessMove>,
    pub level: Level,
    pub limit: SearchLimit,
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

/// Progress of a running search.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SearchInfo {
    pub eval: Eval,
    pub depth: u32,
    /// Expected line, starting with the move to play; legal from the searched position.
    pub pv: Vec<ChessMove>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchResult {
    pub mv: ChessMove,
    pub eval: Eval,
    pub depth: u32,
}

#[derive(Clone, Debug)]
pub enum SearchEvent {
    Progress(SearchInfo),
    /// `result` is `None` when the position has no legal move. `at` is when the engine
    /// answered, so the clock can judge the move by that time rather than when it is read.
    Done {
        result: Option<SearchResult>,
        at: Instant,
    },
}

/// The engine the player asked for; what actually runs may differ (see `Engine::start`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineChoice {
    /// A UCI engine as found by `Engine::detect`, normally the bundled Stockfish.
    Stockfish,
    Builtin,
}

pub enum Engine {
    Builtin,
    Uci(UciEngine),
}

impl Engine {
    /// The engine for `choice`; Stockfish falls back to the built-in engine when none starts.
    pub fn start(choice: EngineChoice) -> Engine {
        match choice {
            EngineChoice::Stockfish => Engine::detect(),
            EngineChoice::Builtin => Engine::Builtin,
        }
    }

    pub fn is_uci(&self) -> bool {
        matches!(self, Engine::Uci(_))
    }

    /// First that works: `CHESS_ENGINE`, the bundled Stockfish, `stockfish` on PATH, built-in.
    pub fn detect() -> Engine {
        match std::env::var("CHESS_ENGINE") {
            Ok(v) if v.eq_ignore_ascii_case("builtin") => return Engine::Builtin,
            Ok(v) => {
                if let Some(engine) = UciEngine::start(Path::new(&v)) {
                    return Engine::Uci(engine);
                }
            }
            Err(_) => {}
        }
        #[cfg(bundled_stockfish)]
        if let Some(engine) = crate::bundled::path()
            .ok()
            .and_then(|path| UciEngine::start(&path))
        {
            return Engine::Uci(engine);
        }
        UciEngine::start(Path::new("stockfish")).map_or(Engine::Builtin, Engine::Uci)
    }

    pub fn name(&self) -> &str {
        match self {
            Engine::Builtin => "built-in",
            Engine::Uci(uci) => &uci.name,
        }
    }

    /// Starts searching `req`; progress and the result arrive on the returned channel.
    /// A disconnect without `Done` means the engine died.
    pub fn think(&self, req: SearchRequest) -> Receiver<SearchEvent> {
        let (tx, rx) = mpsc::channel();
        match self {
            Engine::Builtin => {
                thread::spawn(move || builtin_search(&req, &tx));
            }
            Engine::Uci(uci) => {
                let _ = uci.requests.send((req, tx));
            }
        }
        rx
    }

    /// Abandons request `id`: stops it if it is running and skips it if it is still queued.
    pub fn cancel(&self, id: u64) {
        if let Engine::Uci(uci) = self {
            let mut shared = uci.shared.lock().unwrap_or_else(|e| e.into_inner());
            shared.min_live_id = shared.min_live_id.max(id + 1);
            if shared.active == Some(id) {
                // Only while this very search runs, so a `stop` can never hit a newer one.
                let _ = writeln!(shared.stdin, "stop").and_then(|_| shared.stdin.flush());
            }
        }
    }
}

fn builtin_search(req: &SearchRequest, tx: &Sender<SearchEvent>) {
    let result = ai::search(req, &mut |info| {
        let _ = tx.send(SearchEvent::Progress(info));
    });
    let _ = tx.send(SearchEvent::Done {
        result,
        at: Instant::now(),
    });
}

type Request = (SearchRequest, Sender<SearchEvent>);

/// State the worker and `Engine::cancel` share.
struct Shared {
    stdin: ChildStdin,
    /// The request whose `go` is running.
    active: Option<u64>,
    /// Queued requests below this id were cancelled and are skipped.
    min_live_id: u64,
}

pub struct UciEngine {
    name: String,
    requests: Sender<Request>,
    shared: Arc<Mutex<Shared>>,
    child: Child,
}

impl UciEngine {
    /// Spawns the engine and completes the UCI handshake, or returns `None`.
    fn start(path: &Path) -> Option<UciEngine> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        let shared = Arc::new(Mutex::new(Shared {
            stdin,
            active: None,
            min_live_id: 0,
        }));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (req_tx, req_rx) = mpsc::channel::<Request>();

        let worker_shared = Arc::clone(&shared);
        thread::spawn(move || {
            let mut worker = UciWorker {
                shared: worker_shared,
                stdout,
                level: None,
                root: None,
            };
            match worker.handshake() {
                Ok(name) => {
                    let _ = ready_tx.send(name);
                }
                Err(_) => return,
            }
            // One request at a time, so a stale search never interleaves with a new one.
            for (req, reply) in req_rx {
                if req.id < worker.lock().min_live_id {
                    continue; // cancelled while queued
                }
                match worker.search(&req, &reply) {
                    Ok(Searched::Done(result)) => {
                        let _ = reply.send(SearchEvent::Done {
                            result,
                            at: Instant::now(),
                        });
                    }
                    Ok(Searched::Cancelled) => {}
                    Err(_) => {
                        // The engine is gone: answer this request ourselves, then stop so
                        // the next request sees a disconnect and the app restarts the engine.
                        builtin_search(&req, &reply);
                        return;
                    }
                }
            }
        });

        match ready_rx.recv_timeout(UCI_HANDSHAKE_TIMEOUT) {
            Ok(name) => Some(UciEngine {
                name,
                requests: req_tx,
                shared,
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

struct UciWorker {
    shared: Arc<Mutex<Shared>>,
    stdout: BufReader<ChildStdout>,
    /// Strength currently set in the engine.
    level: Option<Level>,
    /// Root of the previous request, to send `ucinewgame` when a new game starts.
    root: Option<Board>,
}

impl UciWorker {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, cmd: &str) -> io::Result<()> {
        let mut shared = self.lock();
        writeln!(shared.stdin, "{cmd}")?;
        shared.stdin.flush()
    }

    fn read_line(&mut self) -> io::Result<String> {
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(line)
    }

    fn wait_ready(&mut self) -> io::Result<()> {
        self.send("isready")?;
        while self.read_line()?.trim() != "readyok" {}
        Ok(())
    }

    /// Returns the engine's name.
    fn handshake(&mut self) -> io::Result<String> {
        self.send("uci")?;
        let mut name = "UCI engine".to_string();
        let mut spin_max: Vec<(String, u64)> = Vec::new();
        loop {
            let line = self.read_line()?;
            let line = line.trim();
            if let Some(n) = line.strip_prefix("id name ") {
                name = n.to_string();
            }
            if let Some(option) = parse_spin_option(line) {
                spin_max.push(option);
            }
            if line == "uciok" {
                break;
            }
        }
        let max = |option: &str| spin_max.iter().find(|(n, _)| n == option).map(|&(_, m)| m);
        let cores = thread::available_parallelism().map_or(1, |n| n.get() as u64);
        let wanted = [
            ("Threads", cores.saturating_sub(1).max(1)),
            ("Hash", UCI_HASH_MB),
            ("Move Overhead", UCI_MOVE_OVERHEAD_MS),
        ];
        for (option, value) in wanted {
            if let Some(max) = max(option) {
                self.send(&format!("setoption name {option} value {}", value.min(max)))?;
            }
        }
        self.wait_ready()?;
        Ok(name)
    }

    fn search(&mut self, req: &SearchRequest, reply: &Sender<SearchEvent>) -> io::Result<Searched> {
        if self.root != Some(req.root) {
            self.send("ucinewgame")?;
            self.root = Some(req.root);
        }
        if self.level != Some(req.level) {
            for (option, value) in uci_level_options(req.level) {
                self.send(&format!("setoption name {option} value {value}"))?;
            }
            self.wait_ready()?;
            self.level = Some(req.level);
        }

        let mut position = format!("position fen {}", req.root);
        if !req.moves.is_empty() {
            position.push_str(" moves");
            for mv in &req.moves {
                position.push_str(&format!(" {mv}"));
            }
        }
        let go = match req.limit {
            SearchLimit::MoveTime(d) => format!("go movetime {}", d.as_millis()),
            SearchLimit::Clock(t) => format!(
                "go wtime {} btime {} winc {} binc {}",
                t.white.as_millis(),
                t.black.as_millis(),
                t.increment.as_millis(),
                t.increment.as_millis()
            ),
        };
        {
            // Checking for cancellation, writing `go` and marking the search active happen
            // under one lock, so `Engine::cancel` either skips the request or stops it.
            let mut shared = self.lock();
            if req.id < shared.min_live_id {
                return Ok(Searched::Cancelled);
            }
            writeln!(shared.stdin, "{position}")?;
            writeln!(shared.stdin, "{go}")?;
            shared.stdin.flush()?;
            shared.active = Some(req.id);
        }

        let (board, _) = req.replay();
        let side = board.side_to_move();
        let mut last: Option<(u32, Eval)> = None;
        let best = loop {
            let line = self.read_line()?;
            if let Some(info) = parse_info(&line)
                && !info.bound
            {
                let eval = info.eval.for_white(side);
                last = Some((info.depth, eval));
                let pv = legal_line(&board, &info.pv);
                if !pv.is_empty() {
                    let _ = reply.send(SearchEvent::Progress(SearchInfo {
                        eval,
                        depth: info.depth,
                        pv,
                    }));
                }
            }
            if let Some(best) = parse_bestmove(&line) {
                self.lock().active = None;
                break best.and_then(|s| ChessMove::from_str(s).ok());
            }
        };
        let result = best
            .filter(|mv| MoveGen::new_legal(&board).any(|m| m == *mv))
            .map(|mv| {
                let (depth, eval) = last.unwrap_or((0, Eval::Cp(0)));
                SearchResult { mv, eval, depth }
            });
        Ok(Searched::Done(result))
    }
}

enum Searched {
    Done(Option<SearchResult>),
    /// Cancelled before it started; nobody waits for an answer.
    Cancelled,
}

/// `setoption` pairs that give Stockfish the strength of `level`.
fn uci_level_options(level: Level) -> Vec<(&'static str, String)> {
    match level.elo() {
        None => vec![("UCI_LimitStrength", "false".to_string())],
        Some(elo) => vec![
            ("UCI_LimitStrength", "true".to_string()),
            ("UCI_Elo", elo.to_string()),
        ],
    }
}

/// Parses `option name <name> type spin ... max <n>` into `(name, n)`.
fn parse_spin_option(line: &str) -> Option<(String, u64)> {
    let rest = line.strip_prefix("option name ")?;
    let (name, rest) = rest.split_once(" type ")?;
    let mut tokens = rest.split_whitespace();
    if tokens.next()? != "spin" {
        return None;
    }
    while let Some(tok) = tokens.next() {
        if tok == "max" {
            return Some((name.to_string(), tokens.next()?.parse().ok()?));
        }
    }
    None
}

/// One parsed `info` line with a score; the score is from the side to move's point of view.
#[derive(Debug, PartialEq, Eq)]
struct UciInfo {
    depth: u32,
    eval: Eval,
    /// `lowerbound` / `upperbound`: a provisional score.
    bound: bool,
    pv: Vec<String>,
}

fn parse_info(line: &str) -> Option<UciInfo> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "info" {
        return None;
    }
    let mut depth = 0;
    let mut eval = None;
    let mut bound = false;
    let mut pv = Vec::new();
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
            "lowerbound" | "upperbound" => bound = true,
            // Only the move list follows `pv`.
            "pv" => {
                pv = tokens.by_ref().map(str::to_string).collect();
            }
            _ => {}
        }
    }
    Some(UciInfo {
        depth,
        eval: eval?,
        bound,
        pv,
    })
}

/// The longest legal prefix of `moves` (UCI notation) played from `board`.
fn legal_line(board: &Board, moves: &[String]) -> Vec<ChessMove> {
    let mut board = *board;
    let mut line = Vec::new();
    for text in moves {
        let Ok(mv) = ChessMove::from_str(text) else {
            break;
        };
        if !MoveGen::new_legal(&board).any(|m| m == mv) {
            break;
        }
        line.push(mv);
        board = board.make_move_new(mv);
    }
    line
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

    fn mv(s: &str) -> ChessMove {
        ChessMove::from_str(s).unwrap()
    }

    #[test]
    fn parses_info_with_pv() {
        let info =
            parse_info("info depth 12 seldepth 18 multipv 1 score cp -34 nodes 1000 pv e2e4 e7e5")
                .unwrap();
        assert_eq!(info.depth, 12);
        assert_eq!(info.eval, Eval::Cp(-34));
        assert!(!info.bound);
        assert_eq!(info.pv, vec!["e2e4", "e7e5"]);
        let mate = parse_info("info depth 20 score mate 3 pv h5f7").unwrap();
        assert_eq!(mate.eval, Eval::Mate(3));
        assert!(
            parse_info("info depth 9 score cp 20 lowerbound nodes 5 pv e2e4")
                .unwrap()
                .bound
        );
        assert_eq!(
            parse_info("info depth 5 currmove e2e4 currmovenumber 1"),
            None
        );
        assert_eq!(parse_info("info string NNUE enabled"), None);
        assert_eq!(parse_info("bestmove e2e4"), None);
    }

    #[test]
    fn legal_line_stops_at_first_bad_move() {
        let moves: Vec<String> = ["e2e4", "e7e5", "e4e5", "g1f3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            legal_line(&Board::default(), &moves),
            vec![mv("e2e4"), mv("e7e5")]
        );
    }

    #[test]
    fn parses_options_and_levels() {
        assert_eq!(
            parse_spin_option("option name Threads type spin default 1 min 1 max 1024"),
            Some(("Threads".to_string(), 1024))
        );
        assert_eq!(
            parse_spin_option("option name Move Overhead type spin default 10 min 0 max 5000"),
            Some(("Move Overhead".to_string(), 5000))
        );
        assert_eq!(
            parse_spin_option("option name Ponder type check default false"),
            None
        );

        assert_eq!(
            uci_level_options(Level::MAX),
            vec![("UCI_LimitStrength", "false".to_string())]
        );
        assert_eq!(
            uci_level_options(Level::new(1)),
            vec![
                ("UCI_LimitStrength", "true".to_string()),
                ("UCI_Elo", "1320".to_string())
            ]
        );
        assert_eq!(Level::new(0), Level::MIN);
        assert_eq!(Level::new(99), Level::MAX);
        assert_eq!(Level::MAX.stronger(), Level::MAX);
        assert_eq!(Level::MIN.weaker(), Level::MIN);
    }

    #[test]
    fn parses_bestmove() {
        assert_eq!(
            parse_bestmove("bestmove e7e8q ponder a2a3"),
            Some(Some("e7e8q"))
        );
        assert_eq!(parse_bestmove("bestmove (none)"), Some(None));
        assert_eq!(parse_bestmove("info depth 1 score cp 3"), None);
        assert_eq!(mv("e7e8q").get_promotion(), Some(chess::Piece::Queen));
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
            id: 0,
            root: Board::default(),
            moves: vec![mv("e2e4"), mv("e7e5")],
            level: Level::MAX,
            limit: SearchLimit::MoveTime(Duration::from_millis(100)),
        };
        let (board, hashes) = req.replay();
        assert_eq!(hashes.len(), 2);
        assert_eq!(hashes[0], Board::default().get_hash());
        assert_eq!(board.side_to_move(), Color::White);
    }

    #[test]
    fn missing_engine_falls_back() {
        assert!(UciEngine::start(Path::new("definitely-not-a-chess-engine-xyz")).is_none());
    }

    #[cfg(bundled_stockfish)]
    mod stockfish {
        use super::*;

        fn request(id: u64, level: Level, millis: u64) -> SearchRequest {
            SearchRequest {
                id,
                root: Board::default(),
                moves: Vec::new(),
                level,
                limit: SearchLimit::MoveTime(Duration::from_millis(millis)),
            }
        }

        /// Collects events until `Done`; returns the progress seen and the result.
        fn finish(rx: &Receiver<SearchEvent>) -> (Vec<SearchInfo>, Option<SearchResult>) {
            let mut progress = Vec::new();
            loop {
                match rx
                    .recv_timeout(Duration::from_secs(10))
                    .expect("engine answers")
                {
                    SearchEvent::Progress(info) => progress.push(info),
                    SearchEvent::Done { result, .. } => return (progress, result),
                }
            }
        }

        fn legal(mv: ChessMove) -> bool {
            MoveGen::new_legal(&Board::default()).any(|m| m == mv)
        }

        #[test]
        fn plays_streams_cancels_and_switches_level() {
            let dir = crate::bundled::tests::temp_dir("uci");
            let path = crate::bundled::extract_to(&dir).unwrap();
            let engine = Engine::Uci(UciEngine::start(&path).expect("bundled Stockfish starts"));
            assert!(
                engine.name().starts_with("Stockfish"),
                "name: {}",
                engine.name()
            );

            // Level 1: a legal move and a streamed, legal principal variation.
            let (progress, result) = finish(&engine.think(request(1, Level::new(1), 300)));
            assert!(legal(result.expect("a move").mv));
            let last = progress.last().expect("progress events");
            assert!(legal(last.pv[0]));
            assert_eq!(
                legal_line(
                    &Board::default(),
                    &last.pv.iter().map(|m| m.to_string()).collect::<Vec<_>>()
                ),
                last.pv
            );

            // Cancel: a 5 s search stops almost at once.
            let started = Instant::now();
            let rx = engine.think(request(2, Level::MAX, 5_000));
            thread::sleep(Duration::from_millis(200));
            engine.cancel(2);
            let (_, result) = finish(&rx);
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "cancel took {:?}",
                started.elapsed()
            );
            assert!(legal(result.expect("a move").mv));

            // A queued request cancelled before it starts is skipped; the next one runs.
            let skipped = engine.think(request(3, Level::MAX, 5_000));
            engine.cancel(3);
            let (_, result) = finish(&engine.think(request(4, Level::MAX, 200)));
            assert!(legal(result.expect("a move").mv));
            assert!(skipped.recv_timeout(Duration::from_millis(100)).is_err());

            drop(engine); // stop the process before removing its file
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
