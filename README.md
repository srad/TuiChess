<div align="center">

# TuiChess

Play chess against an engine in your terminal.

By Saman Sedighi Rad

[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Edition](https://img.shields.io/badge/edition-2024-blue)](https://doc.rust-lang.org/edition-guide/)
[![ratatui](https://img.shields.io/badge/built%20with-ratatui-8A2BE2)](https://ratatui.rs)
[![Stockfish](https://img.shields.io/badge/engine-Stockfish%2019%20bundled-2ea44f)](#engine)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)](#installation)

<!-- Screenshot: save one as docs/screenshot.png (or change the path) -->
<img src="docs/screenshot.png" alt="TuiChess screenshot" width="800">

</div>

## Features

- MS-DOS look: menu bar, blue desktop, single-line boxes and dialogs with drop shadows
- Menus for the game, level, clock, engine, theme and help, by keyboard (`F10`, `Alt`+letter) or mouse
- Block-art pieces that scale with the terminal, with Unicode glyphs as a fallback for small windows
- Mouse and keyboard input: click or drag pieces, or use the arrow keys or `hjkl`
- Play as White or Black against the engine, or two players on one machine
- Stockfish 19 is built into the executable: nothing else to install
- Eight difficulty levels, from about 1320 Elo to full-strength Stockfish
- Hints: the engine suggests your best move
- Chess clocks (1+0 up to 15+10) with loss or draw on time
- Resign and draw offers; the engine accepts a draw when it is not better
- Live evaluation bar and the engine's expected line while it thinks
- Step back through the game's positions without undoing moves
- Start from any position given as FEN
- Undo, which takes back your last move and the engine's reply
- Move list in algebraic notation with piece symbols (`♘f3`, `Nbd2`, `O-O`, `e8=Q#`)
- Full rules: castling, en passant, promotion, checkmate, stalemate, threefold repetition, the fifty-move rule and insufficient material
- Three colour schemes in the 16-colour VGA palette: Blue, Black and Mono
- Settings (theme, side, level, mode, clock, engine) are remembered between runs
- Fallback engine written in Rust: iterative-deepening alpha-beta search (PVS), a transposition table, quiescence search, killer moves and a tapered [PeSTO](https://www.chessprogramming.org/PeSTO%27s_Evaluation_Function) evaluation
- Any other UCI engine can be used instead

## Installation

You need [Rust](https://rustup.rs) 1.88 or newer, a C compiler (already present with the MSVC toolchain on Windows, and usually on Linux and macOS), and internet access for the first build.

```sh
git clone https://github.com/srad/TuiChess.git
cd TuiChess
cargo run --release
```

Or install the binary:

```sh
cargo install --path .
tuichess
```

The first build downloads the official Stockfish 19 release for your platform (about 80 MB) and checks it against a pinned SHA-256 checksum. Then it embeds the engine in the executable, so binaries are about 105 MB. Each build profile downloads once.

To build offline, download the archive for your platform from the [Stockfish 19 release](https://github.com/official-stockfish/Stockfish/releases/tag/sf_19) and point the build at it:

```sh
STOCKFISH_ARCHIVE=/path/to/stockfish-linux-x86-64-universal.tar.gz cargo build --release
```

Stockfish is bundled for Windows (x86-64, ARM64), Linux with glibc (x86-64, ARM64) and macOS. On other targets the game builds with the Rust engine only.

Use a terminal with true-colour support and a font that has chess glyphs, such as Windows Terminal, iTerm2, WezTerm, Kitty or Alacritty. The window needs at least 79x29 characters; a larger window shows larger pieces.

## Usage

```sh
tuichess                                   # play from the start position
tuichess --fen "8/8/8/4k3/8/8/4P3/4K3 w - -"   # start from a position (move counters optional)
tuichess --help
```

## Controls

| Key | Action |
| --- | --- |
| `←` `↑` `↓` `→` / `h` `j` `k` `l` | Move the cursor |
| `Enter` / left click | Select a piece, then its target |
| Drag with the mouse | Move a piece |
| `Esc` | Cancel the selection |
| `F10` / `Alt`+letter / click | Open a menu (Game, Level, Clock, Engine, Theme, Help) |
| `Q` `R` `B` `N` or click | Choose the promotion piece |
| `u` | Undo |
| `r` | Restart |
| `n` | New game with sides swapped |
| `m` | Switch between playing the engine and two players |
| `f` | Flip the board |
| `+` / `-` | Stronger / weaker engine |
| `s` | Suggest a move |
| `c` | Clock for the next game (off, 1+0, 3+2, 5+3, 10+5, 15+10) |
| `x` | Resign |
| `d` | Offer a draw |
| `,` / `.` | Step back / forward through the game |
| `Home` / `End` | First / current position |
| `t` | Next colour theme |
| `?` / `F1` | Show all keys |
| `q` / `Ctrl+C` | Quit |

Resigning and offering a draw ask for confirmation in a dialog, and so do `r`, `n` and `m` while a game is in progress. Answer with `Y` or `N`, or move the focus with the arrow keys and press `Enter`. The focus starts on No.

In an open menu, the arrow keys move between menus and items, `Enter` or an item's highlighted letter chooses it, and `Esc` closes the menu.

## Difficulty and clocks

Levels 1 to 7 limit Stockfish to about 1320, 1500, 1700, 1900, 2100, 2400 and 2800 Elo. Level 8 is full strength. Without a clock the engine thinks for 1 second per move. With a clock it manages its own time.

The clock starts after the first move. Running out of time loses, unless the opponent has only a king, or a king and one minor piece, which is a draw.

Theme, side, level, mode, clock and engine are saved in `settings.txt` in your config directory (`%APPDATA%\TuiChess` on Windows, `~/Library/Application Support/TuiChess` on macOS, `~/.config/TuiChess` on Linux). Help > About shows the exact path.

## Engine

Choose Stockfish or the built-in engine in the Engine menu; the choice is saved. When Stockfish is chosen, TuiChess picks the first engine that works:

1. `CHESS_ENGINE=<path>`: any UCI engine binary
2. The bundled Stockfish 19. On first start it is written to your user cache directory (`%LOCALAPPDATA%\TuiChess` on Windows, `~/Library/Caches/TuiChess` on macOS, `~/.cache/TuiChess` on Linux) and reused after that.
3. `stockfish` on your `PATH`
4. The Rust engine, which thinks for up to 1 second per move depending on the level

Stockfish uses all but one CPU core and 256 MB of hash. If it stops answering, TuiChess restarts it; if that happens twice within a minute, it switches to the Rust engine.

Set `CHESS_ENGINE=builtin` to always use the Rust engine:

```sh
# bash / zsh
CHESS_ENGINE=builtin cargo run --release
```

```powershell
# PowerShell
$env:CHESS_ENGINE = "builtin"; cargo run --release
```

The menu bar shows which engine is playing.

## Project layout

```
src/
├── main.rs     # command line, terminal setup, event loop
├── app.rs      # app state: keys, mouse, searches, confirmations, browsing
├── game.rs     # game state, rules, draw detection, notation, undo, results
├── clock.rs    # time controls and the chess clock
├── settings.rs # remembered preferences
├── menu.rs     # menu bar contents and commands
├── theme.rs    # colour schemes in the VGA palette
├── ai.rs       # Rust engine: search and PeSTO evaluation
├── engine.rs   # engine selection, levels and UCI protocol
├── bundled.rs  # embedded Stockfish binary and its extraction
└── ui.rs       # menu and status bars, board, panel, dialogs (ratatui)
build.rs        # downloads, verifies and extracts Stockfish at build time
```

Move generation comes from the [`chess`](https://crates.io/crates/chess) crate, rendering from [`ratatui`](https://ratatui.rs) and terminal I/O from [`crossterm`](https://crates.io/crates/crossterm).

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Acknowledgements

- [Stockfish](https://stockfishchess.org), bundled unmodified
- PeSTO piece-square tables by Ronald Friederich (Rofchade), from the [Chess Programming Wiki](https://www.chessprogramming.org)
- Stockfish's neural networks are trained on data from the [Leela Chess Zero project](https://storage.lczero.org/files/training_data), made available under the [Open Database License](https://opendatacommons.org/licenses/odbl/odbl-10.txt)

## License

Copyright (C) 2026 Saman Sedighi Rad

TuiChess is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. It is distributed WITHOUT ANY WARRANTY; see [LICENSE](LICENSE) for details.

TuiChess executables contain an unmodified Stockfish 19 binary. Stockfish is Copyright (C) 2004-2026 The Stockfish developers (see [AUTHORS](https://github.com/official-stockfish/Stockfish/blob/sf_19/AUTHORS)) and is licensed under the GNU GPL version 3 or later. Its complete source code is at [official-stockfish/Stockfish, tag sf_19](https://github.com/official-stockfish/Stockfish/tree/sf_19), and each platform's release archive, which includes `src/`, is on the [Stockfish 19 release page](https://github.com/official-stockfish/Stockfish/releases/tag/sf_19).
