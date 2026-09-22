<div align="center">

# TuiChess

Play chess against an engine in your terminal.

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

- Block-art pieces that scale with the terminal, with Unicode glyphs as a fallback for small windows
- Mouse and keyboard input: click a piece and then its target, or use the arrow keys or `hjkl`
- Play as White or Black; the board flips so your side is always at the bottom
- Stockfish 19 is built into the executable: nothing else to install
- Fallback engine written in Rust: iterative-deepening alpha-beta search (PVS), a transposition table, quiescence search, killer moves and a tapered [PeSTO](https://www.chessprogramming.org/PeSTO%27s_Evaluation_Function) evaluation
- Any other UCI engine can be used instead
- Evaluation bar with the score in centipawns or moves to mate
- Four colour themes: Wood, Forest, Ocean and Slate
- Undo, which takes back your last move and the engine's reply
- Move list in algebraic notation with piece symbols (`♘f3`, `Nbd2`, `O-O`, `e8=Q#`)
- Full rules: castling, en passant, promotion, checkmate, stalemate, threefold repetition, the fifty-move rule and insufficient material
- Highlights for the last move, legal targets, captures and check

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

At startup, TuiChess picks the first engine that works:

1. `CHESS_ENGINE=<path>`: any UCI engine binary
2. The bundled Stockfish 19. On first start it is written to your user cache directory (`%LOCALAPPDATA%\TuiChess` on Windows, `~/Library/Caches/TuiChess` on macOS, `~/.cache/TuiChess` on Linux) and reused after that.
3. `stockfish` on your `PATH`
4. The Rust engine, which thinks for about 1.5 s per move

Stockfish gets 1 second per move and plays at full strength.

Set `CHESS_ENGINE=builtin` to always use the Rust engine:

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
├── ai.rs       # Rust engine: search and PeSTO evaluation
├── engine.rs   # engine selection and UCI protocol
├── bundled.rs  # embedded Stockfish binary and its extraction
└── ui.rs       # board, panel, themes, evaluation bar (ratatui)
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

TuiChess is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. It is distributed WITHOUT ANY WARRANTY; see [LICENSE](LICENSE) for details.

TuiChess executables contain an unmodified Stockfish 19 binary. Stockfish is Copyright (C) 2004-2026 The Stockfish developers (see [AUTHORS](https://github.com/official-stockfish/Stockfish/blob/sf_19/AUTHORS)) and is licensed under the GNU GPL version 3 or later. Its complete source code is at [official-stockfish/Stockfish, tag sf_19](https://github.com/official-stockfish/Stockfish/tree/sf_19), and each platform's release archive, which includes `src/`, is on the [Stockfish 19 release page](https://github.com/official-stockfish/Stockfish/releases/tag/sf_19).
