<div align="center">

# TuiChess

Play chess against an engine in your terminal.

[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Edition](https://img.shields.io/badge/edition-2024-blue)](https://doc.rust-lang.org/edition-guide/)
[![ratatui](https://img.shields.io/badge/built%20with-ratatui-8A2BE2)](https://ratatui.rs)
[![UCI](https://img.shields.io/badge/engine-built--in%20%7C%20UCI-2ea44f)](#engine)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)](#installation)
[![Last commit](https://img.shields.io/github/last-commit/srad/TuiChess)](https://github.com/srad/TuiChess/commits)
[![Stars](https://img.shields.io/github/stars/srad/TuiChess?style=social)](https://github.com/srad/TuiChess/stargazers)

<!-- Screenshot: save one as docs/screenshot.png (or change the path) -->
<img src="docs/screenshot.png" alt="TuiChess screenshot" width="800">

</div>

## Features

- Block-art pieces that scale with the terminal, with Unicode glyphs as a fallback for small windows
- Mouse and keyboard input: click a piece and then its target, or use the arrow keys or `hjkl`
- Play as White or Black; the board flips so your side is always at the bottom
- Built-in engine: iterative-deepening alpha-beta search (PVS), a transposition table, quiescence search, killer moves and a tapered [PeSTO](https://www.chessprogramming.org/PeSTO%27s_Evaluation_Function) evaluation
- Stockfish or any other UCI engine on your `PATH` is detected and used automatically
- Evaluation bar with the score in centipawns or moves to mate
- Four colour themes: Wood, Forest, Ocean and Slate
- Undo, which takes back your last move and the engine's reply
- Move list in algebraic notation with piece symbols (`♘f3`, `Nbd2`, `O-O`, `e8=Q#`)
- Full rules: castling, en passant, promotion, checkmate, stalemate, threefold repetition, the fifty-move rule and insufficient material
- Highlights for the last move, legal targets, captures and check

## Installation

You need [Rust](https://rustup.rs) 1.88 or newer.

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

Use a terminal with true-colour support and a font that has chess glyphs, such as Windows Terminal, iTerm2, WezTerm, Kitty or Alacritty. A larger window shows larger pieces.

## Controls

| Key | Action |
| --- | --- |
| `←` `↑` `↓` `→` / `h` `j` `k` `l` | Move the cursor |
| `Enter` / left click | Select a piece, then its target |
| `Esc` | Cancel the selection |
| `Q` `R` `B` `N` | Choose the promotion piece |
| `u` | Undo |
| `r` | Restart with the same side |
| `n` | New game with sides swapped |
| `t` | Next colour theme |
| `q` / `Ctrl+C` | Quit |

## Engine

At startup, TuiChess picks an engine in this order:

1. `CHESS_ENGINE=<path>`: any UCI engine binary
2. `stockfish` on your `PATH`
3. The built-in engine, which thinks for about 1.5 s per move

Set `CHESS_ENGINE=builtin` to always use the built-in engine:

```sh
# bash / zsh
CHESS_ENGINE=builtin cargo run --release
```

```powershell
# PowerShell
$env:CHESS_ENGINE = "builtin"; cargo run --release
```

The panel shows which engine is playing.

## Project layout

```
src/
├── main.rs     # terminal setup, event loop, keyboard and mouse input
├── game.rs     # game state, rules, draw detection, notation, undo
├── ai.rs       # built-in search and PeSTO evaluation
├── engine.rs   # engine selection and UCI protocol
└── ui.rs       # board, panel, themes, evaluation bar (ratatui)
```

Move generation comes from the [`chess`](https://crates.io/crates/chess) crate, rendering from [`ratatui`](https://ratatui.rs) and terminal I/O from [`crossterm`](https://crates.io/crates/crossterm).

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Acknowledgements

- PeSTO piece-square tables by Ronald Friederich (Rofchade), from the [Chess Programming Wiki](https://www.chessprogramming.org)
- [Stockfish](https://stockfishchess.org) and the UCI protocol
