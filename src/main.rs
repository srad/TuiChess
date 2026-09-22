mod ai;
#[cfg(bundled_stockfish)]
mod bundled;
mod engine;
mod game;
mod ui;

use std::io;
use std::panic;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use chess::{Board, Color, Piece};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Position, Rect},
};

use engine::{Engine, SearchResult};
use game::{Game, Phase};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

fn main() -> io::Result<()> {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // A panicking search thread must not tear down the live UI; main reports it instead.
        if std::thread::current().name() == Some("main") {
            restore_terminal();
        }
        default_hook(info);
    }));

    println!("Starting engine…"); // the first start unpacks ~100 MB; the alt screen hides this after
    let engine = Engine::detect();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run(&mut terminal, &engine);

    restore_terminal();
    terminal.show_cursor()?;
    result
}

/// A search in flight and the position it was started from.
struct Pending {
    searched: Board,
    rx: Receiver<Option<SearchResult>>,
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

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, engine: &Engine) -> io::Result<()> {
    let mut game = Game::new(Color::White);
    let mut pending: Option<Pending> = None;
    // Set when a search yields no usable move; blocks re-searching until the user acts.
    let mut engine_failed = false;
    let mut theme = 0usize;
    let mut tick: u64 = 0;

    loop {
        tick = tick.wrapping_add(1);

        if let Some(p) = &pending {
            match p.rx.try_recv() {
                Ok(Some(res)) => {
                    if game.apply_engine_move(&p.searched, res.mv) {
                        game.eval = Some(res.eval);
                    } else {
                        engine_failed = true;
                    }
                    pending = None;
                }
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    engine_failed = true;
                    pending = None;
                    terminal.clear()?; // a panicking search thread may have printed over the UI
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if pending.is_none() && !engine_failed && !game.game_over() && !game.is_human_turn() {
            pending = Some(Pending {
                searched: game.board,
                rx: engine.think(game.search_request()),
            });
        }

        let view = ui::View {
            theme: &ui::THEMES[theme],
            tick,
            thinking: pending.is_some(),
            engine_name: engine.name(),
            notice: engine_failed.then_some("Engine failed: u/r/n"),
        };
        terminal.draw(|f| ui::draw(f, &game, &view))?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let promoting = matches!(game.phase, Phase::Promoting { .. });
        match event::read()? {
            // Windows emits both Press and Release key events — handle Press only
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Esc => game.cancel(),
                KeyCode::Left | KeyCode::Char('h') => game.move_cursor(-1, 0),
                KeyCode::Right | KeyCode::Char('l') => game.move_cursor(1, 0),
                KeyCode::Up | KeyCode::Char('k') => game.move_cursor(0, 1),
                KeyCode::Down | KeyCode::Char('j') => game.move_cursor(0, -1),
                KeyCode::Enter => {
                    game.confirm();
                }
                KeyCode::Char(c) if promoting => {
                    if let Some(piece) = promotion_piece(c) {
                        game.promote(piece);
                    }
                }
                KeyCode::Char(c) => match c.to_ascii_lowercase() {
                    'q' => break,
                    'u' => {
                        game.undo();
                        pending = None; // orphaned search's result is discarded
                        engine_failed = false;
                    }
                    'r' => {
                        game.restart();
                        pending = None;
                        engine_failed = false;
                    }
                    'n' => {
                        game = Game::new(!game.human);
                        pending = None;
                        engine_failed = false;
                    }
                    't' => theme = (theme + 1) % ui::THEMES.len(),
                    _ => {}
                },
                _ => {}
            },
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) && !promoting => {
                let size = terminal.size()?;
                let area = Rect::new(0, 0, size.width, size.height);
                let click = Position {
                    x: m.column,
                    y: m.row,
                };
                if let Some(sq) = ui::square_at(area, game.human, click) {
                    game.set_cursor(sq);
                    game.confirm();
                }
            }
            _ => {}
        }
    }
    Ok(())
}
