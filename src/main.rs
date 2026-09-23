mod ai;
mod app;
mod art;
#[cfg(bundled_stockfish)]
mod bundled;
mod clock;
mod engine;
mod game;
mod menu;
mod settings;
mod theme;
mod ui;

use std::io;
use std::panic;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};

use app::{App, Flow};
use engine::Engine;
use game::StartPosition;
use settings::Settings;

const USAGE: &str = "\
Usage: tuichess [--fen \"<FEN>\"]

Play chess against Stockfish (or a second player) in the terminal.

Options:
  --fen <FEN>   start from this position (move counters optional)
  --help        show this help
  --version     show the version

Environment:
  CHESS_ENGINE  path to a UCI engine to use instead, or `builtin`

In the game, press F10 for the menu or ? for the keys.";

/// What the command line asks for.
enum Command {
    Play(StartPosition),
    Help,
    Version,
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut start = StartPosition::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            "--fen" => {
                let fen = rest.next().ok_or("--fen needs a position")?;
                start = StartPosition::from_fen(fen)?;
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Command::Play(start))
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = match parse_args(&args) {
        Ok(Command::Play(start)) => start,
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            print!("tuichess {}", env!("CARGO_PKG_VERSION"));
            #[cfg(bundled_stockfish)]
            print!(" (Stockfish {})", env!("TUICHESS_STOCKFISH_TAG"));
            println!();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("tuichess: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match play(start) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tuichess: {e}");
            ExitCode::FAILURE
        }
    }
}

fn play(start: StartPosition) -> io::Result<()> {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // A panicking search thread must not tear down the live UI; main reports it instead.
        if std::thread::current().name() == Some("main") {
            restore_terminal();
        }
        default_hook(info);
    }));

    let settings_path = settings::default_path();
    let settings = settings_path
        .as_deref()
        .map_or_else(Settings::default, Settings::load);
    println!("Starting engine…"); // the first start unpacks ~100 MB; the alt screen hides this after
    let engine = Engine::start(settings.engine);
    let mut app = App::new(engine, settings, settings_path, start);

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run(&mut terminal, &mut app);

    restore_terminal();
    terminal.show_cursor()?;
    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        let now = Instant::now();
        app.tick(now);
        terminal.draw(|f| ui::draw(f, &app.game, &app.view(now)))?;

        // Faster frames while a move slides.
        let wait = if ui::animating(&app.game, now) {
            16
        } else {
            50
        };
        if !event::poll(Duration::from_millis(wait))? {
            continue;
        }
        let now = Instant::now();
        let flow = match event::read()? {
            Event::Key(key) => app.on_key(key, now),
            Event::Mouse(mouse) => {
                let size = terminal.size()?;
                app.on_mouse(mouse, Rect::new(0, 0, size.width, size.height), now)
            }
            _ => Flow::Continue,
        };
        if flow == Flow::Quit {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_command_line() {
        assert!(
            matches!(parse_args(&args(&[])), Ok(Command::Play(s)) if s == StartPosition::default())
        );
        assert!(matches!(parse_args(&args(&["--help"])), Ok(Command::Help)));
        assert!(matches!(
            parse_args(&args(&["--version"])),
            Ok(Command::Version)
        ));
        let Ok(Command::Play(start)) =
            parse_args(&args(&["--fen", "4k3/8/8/8/8/8/8/4K3 b - - 3 20"]))
        else {
            panic!("FEN not accepted");
        };
        assert_eq!(start.fullmove, 20);
        assert!(parse_args(&args(&["--fen"])).is_err());
        assert!(parse_args(&args(&["--fen", "nonsense"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
    }
}
