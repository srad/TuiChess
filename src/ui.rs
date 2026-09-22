use std::time::{Duration, Instant};

use chess::{ALL_SQUARES, Board, ChessMove, Color, EMPTY, File, Piece, Rank, Square};
use ratatui::{
    Frame,
    layout::{Alignment, Position, Rect},
    style::{Color as TColor, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::clock::{self, TimeControl};
use crate::engine::{Eval, Level};
use crate::game::{Game, Phase, captured_pieces, material, piece_letter, san};

const PANEL_W: u16 = 26;
const ART_W: u16 = 7;
const ART_H: u16 = 4;

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub struct Theme {
    pub name: &'static str,
    light: TColor,
    dark: TColor,
    last_light: TColor,
    last_dark: TColor,
    select: TColor,
    check: TColor,
    capture: TColor,
    hint: TColor,
    target: TColor,
    cursor: TColor,
    white_piece: TColor,
    black_piece: TColor,
}

const fn rgb(r: u8, g: u8, b: u8) -> TColor {
    TColor::Rgb(r, g, b)
}

const CURSOR: TColor = rgb(0xFF, 0xE0, 0x3A);
const CHECK: TColor = rgb(0xD0, 0x3A, 0x2F);
const WHITE_PIECE: TColor = rgb(0xFF, 0xFF, 0xFF);
const BLACK_PIECE: TColor = rgb(0x10, 0x10, 0x10);
const DOT_DARK: TColor = rgb(0x26, 0x26, 0x26);

/// Square colours are mid-tones so that both white and black pieces stay readable.
pub const THEMES: [Theme; 4] = [
    Theme {
        name: "Wood",
        light: rgb(0xC4, 0x9A, 0x6C),
        dark: rgb(0x8B, 0x5A, 0x3C),
        last_light: rgb(0xC9, 0xB2, 0x4E),
        last_dark: rgb(0x9A, 0x86, 0x28),
        select: rgb(0x2E, 0x8B, 0x80),
        check: CHECK,
        capture: rgb(0xB0, 0x48, 0x6A),
        hint: rgb(0x5A, 0x8C, 0xD0),
        target: DOT_DARK,
        cursor: CURSOR,
        white_piece: WHITE_PIECE,
        black_piece: BLACK_PIECE,
    },
    Theme {
        name: "Forest",
        light: rgb(0xA8, 0xB8, 0x88),
        dark: rgb(0x6B, 0x8A, 0x4E),
        last_light: rgb(0xCC, 0xCC, 0x55),
        last_dark: rgb(0xA0, 0xA0, 0x30),
        select: rgb(0x3A, 0x86, 0xB0),
        check: CHECK,
        capture: rgb(0xC0, 0x60, 0x40),
        hint: rgb(0x8A, 0x6C, 0xC8),
        target: DOT_DARK,
        cursor: CURSOR,
        white_piece: WHITE_PIECE,
        black_piece: BLACK_PIECE,
    },
    Theme {
        name: "Ocean",
        light: rgb(0x92, 0xA8, 0xB8),
        dark: rgb(0x5A, 0x74, 0x90),
        last_light: rgb(0xB8, 0xC0, 0x70),
        last_dark: rgb(0x88, 0x92, 0x50),
        select: rgb(0x3E, 0xA0, 0x6A),
        check: CHECK,
        capture: rgb(0xC0, 0x60, 0x80),
        hint: rgb(0xD0, 0x96, 0x4A),
        target: DOT_DARK,
        cursor: CURSOR,
        white_piece: WHITE_PIECE,
        black_piece: BLACK_PIECE,
    },
    Theme {
        name: "Slate",
        light: rgb(0x88, 0x88, 0x88),
        dark: rgb(0x5C, 0x5C, 0x5C),
        last_light: rgb(0xA8, 0xA0, 0x60),
        last_dark: rgb(0x80, 0x78, 0x40),
        select: rgb(0x40, 0x90, 0xC0),
        check: CHECK,
        capture: rgb(0xC0, 0x50, 0x50),
        hint: rgb(0x5A, 0xA8, 0x6A),
        target: rgb(0xE0, 0xE0, 0xE0),
        cursor: CURSOR,
        white_piece: WHITE_PIECE,
        black_piece: BLACK_PIECE,
    },
];

/// Index of the theme called `name`, or the first theme.
pub fn theme_index(name: &str) -> usize {
    THEMES.iter().position(|t| t.name == name).unwrap_or(0)
}

/// Per-frame state that is not part of the game itself.
pub struct View<'a> {
    pub theme: &'a Theme,
    pub tick: u64,
    pub now: Instant,
    /// Label while a search runs, e.g. "AI thinking…".
    pub thinking: Option<&'a str>,
    pub engine_name: &'a str,
    pub level: Level,
    /// Ply shown while browsing; `None` = the live position.
    pub browse: Option<usize>,
    /// Clock preset for the next game, when it differs from this game's.
    pub next_clock: Option<Option<TimeControl>>,
    pub notice: Option<&'a str>,
    pub help_open: bool,
    /// The engine's expected line and the position it starts from.
    pub engine_line: Option<(&'a Board, &'a [ChessMove])>,
    pub settings_path: Option<&'a str>,
}

fn piece_glyph(color: Color, piece: Piece) -> char {
    match (color, piece) {
        (Color::White, Piece::King) => '♔',
        (Color::White, Piece::Queen) => '♕',
        (Color::White, Piece::Rook) => '♖',
        (Color::White, Piece::Bishop) => '♗',
        (Color::White, Piece::Knight) => '♘',
        (Color::White, Piece::Pawn) => '♙',
        (Color::Black, Piece::King) => '♚',
        (Color::Black, Piece::Queen) => '♛',
        (Color::Black, Piece::Rook) => '♜',
        (Color::Black, Piece::Bishop) => '♝',
        (Color::Black, Piece::Knight) => '♞',
        (Color::Black, Piece::Pawn) => '♟',
    }
}

/// Hand-drawn piece art, 7 wide x 4 tall, using single-width block chars.
fn piece_art(piece: Piece) -> [&'static str; 4] {
    match piece {
        Piece::Pawn => ["       ", "  ▄█▄  ", "  ▀█▀  ", " ▄███▄ "],
        Piece::Knight => ["   ▄██ ", "  █▟█▔ ", "  ▀█▄  ", " ▄████▄"],
        Piece::Bishop => ["       ", "  ▄◆▄  ", "  ▐█▌  ", " ▄████▄"],
        Piece::Rook => ["       ", " █▘█▝█ ", " ▐███▌ ", "▐█████▌"],
        Piece::Queen => [" ▘ █ ▝ ", " █████ ", " █████ ", "▐█████▌"],
        Piece::King => ["   ▄   ", "  █+█  ", " █████ ", "▐█████▌"],
    }
}

/// Per-frame layout geometry.
struct Geometry {
    frame: Rect, // box-drawing frame incl. borders
    board: Rect, // 8*cell_w x 8*cell_h, inside frame
    cell_w: u16,
    cell_h: u16,
    labels_left: Rect, // rank labels column
    labels_bottom: Rect,
    panel: Rect,
    big_art: bool,
}

fn compute_geometry(area: Rect) -> Result<Geometry, ()> {
    // height budget: frame top+bottom + file-label row
    let avail_h = area.height.saturating_sub(3);
    let cell_h = (avail_h / 8).clamp(0, 6);
    if cell_h < 3 {
        return Err(());
    }
    // width budget: left label + frame sides + gap + panel + margin
    let avail_w = area.width.saturating_sub(1 + 2 + 1 + PANEL_W + 1);
    let cell_w = (avail_w / 8).min(2 * cell_h + 1);
    if cell_w < 6 {
        return Err(());
    }

    let board_w = 8 * cell_w;
    let board_h = 8 * cell_h;
    let total_w = 1 + 1 + board_w + 1 + 1 + PANEL_W; // label + frame + board + frame + gap + panel
    let total_h = 1 + board_h + 1 + 1; // frame + board + frame + labels

    let x = area.x + (area.width.saturating_sub(total_w)) / 2;
    let y = area.y + (area.height.saturating_sub(total_h)) / 2;

    let labels_left = Rect {
        x,
        y: y + 1,
        width: 1,
        height: board_h,
    };
    let frame = Rect {
        x: x + 1,
        y,
        width: board_w + 2,
        height: board_h + 2,
    };
    let board = Rect {
        x: x + 2,
        y: y + 1,
        width: board_w,
        height: board_h,
    };
    let labels_bottom = Rect {
        x: x + 2,
        y: y + 1 + board_h + 1,
        width: board_w,
        height: 1,
    };
    let panel = Rect {
        x: frame.x + frame.width + 1,
        y: frame.y,
        width: PANEL_W,
        height: frame.height + 1,
    };
    Ok(Geometry {
        frame,
        board,
        cell_w,
        cell_h,
        labels_left,
        labels_bottom,
        panel,
        big_art: cell_h >= ART_H && cell_w >= ART_W,
    })
}

/// Board column and row (0 = top) where `sq` is drawn, with `orientation`'s side at the bottom.
fn screen_cell(orientation: Color, sq: Square) -> Position {
    let file = sq.get_file().to_index() as u16;
    let rank = sq.get_rank().to_index() as u16;
    match orientation {
        Color::White => Position {
            x: file,
            y: 7 - rank,
        },
        Color::Black => Position {
            x: 7 - file,
            y: rank,
        },
    }
}

fn square_from_cell(orientation: Color, cell: Position) -> Square {
    let (file, rank) = match orientation {
        Color::White => (cell.x, 7 - cell.y),
        Color::Black => (7 - cell.x, cell.y),
    };
    Square::make_square(
        Rank::from_index(rank as usize),
        File::from_index(file as usize),
    )
}

fn cell_area(geo: &Geometry, orientation: Color, sq: Square) -> Rect {
    let cell = screen_cell(orientation, sq);
    Rect {
        x: geo.board.x + cell.x * geo.cell_w,
        y: geo.board.y + cell.y * geo.cell_h,
        width: geo.cell_w,
        height: geo.cell_h,
    }
}

/// The square under a mouse click, if the click hit the board.
pub fn square_at(area: Rect, orientation: Color, click: Position) -> Option<Square> {
    let geo = compute_geometry(area).ok()?;
    if !geo.board.contains(click) {
        return None;
    }
    let cell = Position {
        x: (click.x - geo.board.x) / geo.cell_w,
        y: (click.y - geo.board.y) / geo.cell_h,
    };
    Some(square_from_cell(orientation, cell))
}

const PROMOTION_PIECES: [Piece; 4] = [Piece::Queen, Piece::Rook, Piece::Bishop, Piece::Knight];

/// The promotion popup and its four clickable piece cells, shared by drawing and hit-testing.
fn promotion_layout(geo: &Geometry) -> (Rect, [(Piece, Rect); 4]) {
    let w = 24u16.min(geo.frame.width);
    let h = 7u16;
    let popup = Rect {
        x: geo.frame.x + (geo.frame.width.saturating_sub(w)) / 2,
        y: geo.frame.y + (geo.frame.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let cell_w = 5;
    let left = popup.x + (w.saturating_sub(4 * cell_w)) / 2;
    let cells = PROMOTION_PIECES.map(|piece| {
        let i = PROMOTION_PIECES
            .iter()
            .position(|&p| p == piece)
            .unwrap_or(0) as u16;
        let rect = Rect {
            x: left + i * cell_w,
            y: popup.y + 3,
            width: cell_w,
            height: 2,
        };
        (piece, rect)
    });
    (popup, cells)
}

/// Screen position in the middle of `sq`, for driving the mouse in tests.
#[cfg(test)]
pub(crate) fn square_center(area: Rect, orientation: Color, sq: Square) -> Position {
    let cell = cell_area(&compute_geometry(area).unwrap(), orientation, sq);
    Position {
        x: cell.x + cell.width / 2,
        y: cell.y + cell.height / 2,
    }
}

/// Screen position of `piece`'s cell in the promotion popup, for tests.
#[cfg(test)]
pub(crate) fn promotion_cell(area: Rect, piece: Piece) -> Position {
    let (_, cells) = promotion_layout(&compute_geometry(area).unwrap());
    let rect = cells.iter().find(|(p, _)| *p == piece).unwrap().1;
    Position {
        x: rect.x + 1,
        y: rect.y,
    }
}

/// The promotion piece under a mouse click, if the click hit one of the popup's cells.
pub fn promotion_choice_at(area: Rect, click: Position) -> Option<Piece> {
    let geo = compute_geometry(area).ok()?;
    let (_, cells) = promotion_layout(&geo);
    cells
        .into_iter()
        .find(|(_, rect)| rect.contains(click))
        .map(|(piece, _)| piece)
}

pub fn draw(f: &mut Frame, game: &Game, view: &View) {
    let area = f.area();
    let geo = match compute_geometry(area) {
        Ok(g) => g,
        Err(()) => {
            let msg = Paragraph::new(format!(
                "Terminal too small ({}x{}). Maximize the window or reduce font size.",
                area.width, area.height
            ))
            .alignment(Alignment::Center);
            f.render_widget(msg, area);
            return;
        }
    };

    let title_style = Style::default()
        .fg(TColor::LightYellow)
        .add_modifier(Modifier::BOLD);
    let title = match view.browse {
        Some(ply) => Span::styled(
            format!(" Move {ply}/{} · End returns ", game.history.len()),
            title_style,
        ),
        None if game.game_over() => Span::styled(format!(" {} ", game.status_text()), title_style),
        None => Span::raw(" Chess "),
    };
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title),
        geo.frame,
    );
    let live = view.browse.is_none();
    let (board, last) = match view.browse {
        Some(ply) => game.position_at(ply),
        None => (game.board, game.last_move()),
    };
    draw_cells(
        f,
        &Shown {
            game,
            board: &board,
            last,
            live,
        },
        view.theme,
        &geo,
    );
    if live && !game.game_over() {
        draw_cursor(f, game, view.theme, &geo);
    }
    draw_labels(f, game.orientation, &geo);
    draw_panel(f, game, view, geo.panel);
    if live && let Phase::Promoting { .. } = game.phase {
        draw_promotion_popup(f, &geo, game.board.side_to_move());
    }
    if view.help_open {
        draw_help(f, area, view);
    }
}

/// The position on screen: live, or a past one while browsing.
struct Shown<'a> {
    game: &'a Game,
    board: &'a Board,
    last: Option<(Square, Square)>,
    /// Selection, targets and hint only show on the live position.
    live: bool,
}

fn draw_cells(f: &mut Frame, shown: &Shown, theme: &Theme, geo: &Geometry) {
    let game = shown.game;
    let board = shown.board;
    let check_sq = (*board.checkers() != EMPTY).then(|| board.king_square(board.side_to_move()));
    let hint = game
        .hint
        .filter(|_| shown.live)
        .map(|mv| (mv.get_source(), mv.get_dest()));

    for sq in ALL_SQUARES {
        let cell = cell_area(geo, game.orientation, sq);
        let light = (sq.get_file().to_index() + sq.get_rank().to_index()) % 2 == 1;
        let occupant = board.piece_on(sq).zip(board.color_on(sq));
        let is_target = shown.live && game.legal_targets.contains(&sq);

        let mut bg = if light { theme.light } else { theme.dark };
        if shown.last.is_some_and(|(a, b)| sq == a || sq == b) {
            bg = if light {
                theme.last_light
            } else {
                theme.last_dark
            };
        }
        if hint.is_some_and(|(a, b)| sq == a || sq == b) {
            bg = theme.hint;
        }
        if is_target && occupant.is_some() {
            bg = theme.capture;
        }
        if shown.live && game.phase == Phase::Selected(sq) {
            bg = theme.select;
        }
        if Some(sq) == check_sq {
            bg = theme.check;
        }

        let base = Style::default().bg(bg);
        let mid = geo.cell_h / 2;
        let centered = |text: String, style: Style| -> Vec<Line<'static>> {
            (0..geo.cell_h)
                .map(|i| {
                    if i == mid {
                        Line::styled(text.clone(), style)
                    } else {
                        Line::from("")
                    }
                })
                .collect()
        };

        let lines: Vec<Line> = if let Some((piece, color)) = occupant {
            let fg = match color {
                Color::White => theme.white_piece,
                Color::Black => theme.black_piece,
            };
            let st = base.fg(fg).add_modifier(Modifier::BOLD);
            if geo.big_art {
                let pad_top = (geo.cell_h - ART_H) / 2;
                (0..pad_top)
                    .map(|_| Line::from(""))
                    .chain(piece_art(piece).iter().map(|row| Line::styled(*row, st)))
                    .collect()
            } else {
                centered(piece_glyph(color, piece).to_string(), st)
            }
        } else if is_target {
            centered("●".to_string(), base.fg(theme.target))
        } else {
            Vec::new()
        };

        f.render_widget(
            Paragraph::new(lines)
                .style(base)
                .alignment(Alignment::Center),
            cell,
        );
    }
}

/// Corner brackets only, so the piece inside stays visible.
fn draw_cursor(f: &mut Frame, game: &Game, theme: &Theme, geo: &Geometry) {
    let cell = cell_area(geo, game.orientation, game.cursor_square());
    let right = cell.x + cell.width - 1;
    let bottom = cell.y + cell.height - 1;
    let style = Style::default()
        .fg(theme.cursor)
        .add_modifier(Modifier::BOLD);
    let buf = f.buffer_mut();
    for (x, y, ch) in [
        (cell.x, cell.y, '┏'),
        (right, cell.y, '┓'),
        (cell.x, bottom, '┗'),
        (right, bottom, '┛'),
    ] {
        if let Some(c) = buf.cell_mut(Position { x, y }) {
            c.set_char(ch).set_style(style);
        }
    }
}

fn draw_labels(f: &mut Frame, orientation: Color, geo: &Geometry) {
    let label_style = Style::default().fg(TColor::DarkGray);
    for i in 0..8u8 {
        let rank_sq = Square::make_square(Rank::from_index(i as usize), File::A);
        let row = screen_cell(orientation, rank_sq).y;
        f.render_widget(
            Paragraph::new(((b'1' + i) as char).to_string()).style(label_style),
            Rect {
                x: geo.labels_left.x,
                y: geo.labels_left.y + row * geo.cell_h + geo.cell_h / 2,
                width: 1,
                height: 1,
            },
        );
        let file_sq = Square::make_square(Rank::First, File::from_index(i as usize));
        let col = screen_cell(orientation, file_sq).x;
        f.render_widget(
            Paragraph::new(((b'a' + i) as char).to_string()).style(label_style),
            Rect {
                x: geo.labels_bottom.x + col * geo.cell_w + geo.cell_w / 2,
                y: geo.labels_bottom.y,
                width: 1,
                height: 1,
            },
        );
    }
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::White => "White",
        Color::Black => "Black",
    }
}

fn eval_label(eval: Option<Eval>) -> String {
    match eval {
        None => "–".to_string(),
        Some(Eval::Cp(cp)) => format!("{:+.2}", cp as f64 / 100.0),
        Some(Eval::Mate(n)) if n >= 0 => format!("M{n}"),
        Some(Eval::Mate(n)) => format!("-M{}", -n),
    }
}

/// Share of the bar that is White's: 0.0 (Black winning) to 1.0 (White winning).
fn eval_fraction(eval: Option<Eval>) -> f64 {
    match eval {
        None => 0.5,
        Some(Eval::Cp(cp)) => 1.0 / (1.0 + (-(cp as f64) / 400.0).exp()),
        Some(Eval::Mate(n)) if n > 0 => 1.0,
        Some(Eval::Mate(n)) if n < 0 => 0.0,
        Some(Eval::Mate(_)) => 0.5,
    }
}

fn eval_bar(width: usize, eval: Option<Eval>) -> Line<'static> {
    let white = ((eval_fraction(eval) * width as f64).round() as usize).min(width);
    Line::from(vec![
        Span::styled("█".repeat(white), Style::default().fg(TColor::White)),
        Span::styled(
            "█".repeat(width - white),
            Style::default().fg(TColor::DarkGray),
        ),
    ])
}

/// SAN with piece letters replaced by figurines of the mover's colour.
fn figurine(san: &str, mover: Color) -> String {
    san.chars()
        .map(|c| {
            [
                Piece::King,
                Piece::Queen,
                Piece::Rook,
                Piece::Bishop,
                Piece::Knight,
            ]
            .into_iter()
            .find(|&p| piece_letter(p) == c)
            .map_or(c, |p| piece_glyph(mover, p))
        })
        .collect()
}

/// A line of moves from `board` in figurine SAN, cut to `width` characters.
fn line_text(board: &Board, moves: &[ChessMove], width: usize) -> String {
    let mut board = *board;
    let mut text = String::new();
    for &mv in moves {
        let word = figurine(&san(&board, mv), board.side_to_move());
        let needed = word.chars().count() + usize::from(!text.is_empty());
        if text.chars().count() + needed > width {
            break;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&word);
        board = board.make_move_new(mv);
    }
    text
}

/// `m:ss`, with tenths under ten seconds.
fn clock_text(d: Duration) -> String {
    let secs = d.as_secs();
    if d < Duration::from_secs(10) {
        format!("{}:{:02}.{}", secs / 60, secs % 60, d.subsec_millis() / 100)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn clock_row(game: &Game, view: &View) -> Line<'static> {
    let mut spans = Vec::new();
    match &game.clock {
        None => spans.push(Span::raw("No clock")),
        Some(clock) => {
            for (color, label) in [(Color::White, "W "), (Color::Black, "B ")] {
                let left = clock.remaining(color, view.now);
                let mut style = Style::default();
                if clock.running() == Some(color) {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if left.is_zero() {
                    style = style.fg(TColor::LightRed);
                }
                if color == Color::Black {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(format!("{label}{}", clock_text(left)), style));
            }
        }
    }
    if let Some(next) = view.next_clock {
        spans.push(Span::styled(
            format!(" →{}", clock::preset_name(next)),
            Style::default().fg(TColor::DarkGray),
        ));
    }
    Line::from(spans)
}

fn draw_panel(f: &mut Frame, game: &Game, view: &View, rect: Rect) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(TColor::DarkGray);
    let inner_w = rect.width.saturating_sub(2) as usize;
    let inner_h = rect.height.saturating_sub(2) as usize;
    let board = &game.board;
    let two_player = game.engine_side.is_none();

    let mut top: Vec<Line> = Vec::new();
    if game.game_over() {
        top.push(Line::styled(
            game.status_text(),
            Style::default()
                .fg(TColor::Black)
                .bg(TColor::LightYellow)
                .add_modifier(Modifier::BOLD),
        ));
        let keys = if game.is_final() {
            "r new · n swap"
        } else {
            "r new · n swap · u undo"
        };
        top.push(Line::styled(keys, dim));
    } else if let Some(label) = view.thinking {
        let spin = SPINNER[(view.tick as usize / 2) % SPINNER.len()];
        top.push(Line::from(vec![
            Span::styled(format!("{spin} "), Style::default().fg(TColor::Cyan)),
            Span::styled(label.to_string(), Style::default().fg(TColor::Yellow)),
        ]));
    } else {
        let status = game.status_text();
        let color = if status.contains("Check") {
            TColor::LightRed
        } else {
            TColor::Yellow
        };
        top.push(Line::styled(status, Style::default().fg(color)));
    }
    if let Some(notice) = view.notice {
        top.push(Line::styled(
            notice.to_string(),
            Style::default().fg(TColor::LightRed),
        ));
    }

    // Players, engine, clocks, evaluation.
    top.push(match game.engine_side {
        Some(engine) => Line::from(format!(
            "You {} · AI {}",
            color_name(!engine),
            color_name(engine)
        )),
        None => Line::from("White vs Black"),
    });
    let engine_row = if two_player {
        view.engine_name.to_string()
    } else {
        format!("{} · L{}", view.engine_name, view.level.get())
    };
    top.push(Line::styled(engine_row, dim));
    top.push(clock_row(game, view));
    let depth = game.eval_depth.map_or(String::new(), |d| format!("  d{d}"));
    top.push(Line::from(vec![
        Span::styled("Eval ", bold),
        Span::raw(format!("{}{depth}", eval_label(game.eval))),
    ]));
    top.push(eval_bar(inner_w, game.eval));
    let line = view
        .engine_line
        .map_or(String::new(), |(b, moves)| line_text(b, moves, inner_w));
    top.push(Line::styled(line, dim));

    // Material and captures.
    let (me, them, me_label, them_label) = match game.engine_side {
        Some(engine) => (!engine, engine, "You ", "AI  "),
        None => (Color::White, Color::Black, "W ", "B "),
    };
    let diff = material(board, me) - material(board, them);
    let material_text = match (diff, game.engine_side) {
        (0, _) => "even".to_string(),
        (d, Some(_)) if d > 0 => format!("+{d} you"),
        (d, Some(_)) => format!("+{} AI", -d),
        (d, None) if d > 0 => format!("+{d} White"),
        (d, None) => format!("+{} Black", -d),
    };
    top.push(Line::from(format!("Material {material_text}")));
    top.push(Line::from(vec![
        Span::raw(me_label),
        captured_span(them, &captured_pieces(board, them)),
    ]));
    top.push(Line::from(vec![
        Span::raw(them_label),
        captured_span(me, &captured_pieces(board, me)),
    ]));
    top.push(Line::styled("Moves", bold));

    let bottom: Vec<Line> = vec![
        Line::styled("? help", dim),
        Line::styled("GPL-3.0 · no warranty", dim),
    ];

    let move_lines = move_rows(game, view.browse);
    let rows = inner_h.saturating_sub(top.len() + bottom.len());
    let start = move_lines.len().saturating_sub(rows);
    let filler = rows.saturating_sub(move_lines.len() - start);

    let lines: Vec<Line> = top
        .into_iter()
        .chain(move_lines.into_iter().skip(start))
        .chain(std::iter::repeat_n(Line::from(""), filler))
        .chain(bottom)
        .collect();
    let panel = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );
    f.render_widget(panel, rect);
}

/// One row per full move, numbered from the start position; the move leading to the shown
/// position is highlighted.
fn move_rows(game: &Game, browse: Option<usize>) -> Vec<Line<'static>> {
    let dim = Style::default().fg(TColor::DarkGray);
    let highlight = match browse {
        Some(ply) => ply.checked_sub(1),
        None => game.history.len().checked_sub(1),
    };
    // A Black-to-move start leaves White's first slot empty.
    let offset = usize::from(game.start.board.side_to_move() == Color::Black);
    let slots = game.history.len() + offset;
    (0..slots.div_ceil(2))
        .map(|row| {
            let number = game.start.fullmove as usize + row;
            let mut spans = vec![Span::styled(format!("{number:>3}. "), dim)];
            for slot in [row * 2, row * 2 + 1] {
                let Some(ply) = slot.checked_sub(offset) else {
                    spans.push(Span::styled(format!("{:<9}", "…"), dim));
                    continue;
                };
                let Some(p) = game.history.get(ply) else {
                    continue;
                };
                let text = format!("{:<9}", figurine(&p.san, p.before.side_to_move()));
                let style = if Some(ply) == highlight {
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(text, style));
            }
            Line::from(spans)
        })
        .collect()
}

fn captured_span(cap_color: Color, captured: &[(Piece, u8)]) -> Span<'static> {
    let s: String = captured
        .iter()
        .flat_map(|&(piece, n)| std::iter::repeat_n(piece_glyph(cap_color, piece), n as usize))
        .collect();
    let fg = match cap_color {
        Color::White => TColor::White,
        Color::Black => TColor::Gray,
    };
    Span::styled(s, Style::default().fg(fg))
}

fn draw_promotion_popup(f: &mut Frame, geo: &Geometry, color: Color) {
    let (popup, cells) = promotion_layout(geo);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Promote pawn to:",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::from(""),
            Line::from(""),
            Line::from(""),
            Line::styled("click or key · Esc", Style::default().fg(TColor::DarkGray)),
        ])
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .title("Promotion"),
        ),
        popup,
    );
    for (piece, rect) in cells {
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    piece_glyph(color, piece).to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    piece_letter(piece).to_string(),
                    Style::default().fg(TColor::LightMagenta),
                ),
            ])
            .alignment(Alignment::Center),
            rect,
        );
    }
}

const HELP: [(&str, &str); 19] = [
    ("←↑↓→ / hjkl", "move the cursor"),
    ("Enter / click", "select, then move"),
    ("drag", "move a piece with the mouse"),
    ("Esc", "cancel"),
    ("u", "undo"),
    ("r", "restart"),
    ("n", "new game, other side"),
    ("m", "engine / two players"),
    ("f", "flip the board"),
    ("+ / -", "stronger / weaker engine"),
    ("s", "suggest a move"),
    ("c", "clock for the next game"),
    ("t", "next theme"),
    ("x", "resign (press twice)"),
    ("d", "offer a draw (twice)"),
    (", / .", "step through the game"),
    ("Home / End", "first / current position"),
    ("? / F1", "this help"),
    ("q", "quit"),
];

fn draw_help(f: &mut Frame, area: Rect, view: &View) {
    let key = Style::default()
        .fg(TColor::LightMagenta)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(TColor::DarkGray);
    let mut lines: Vec<Line> = HELP
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(format!("{k:<14}"), key),
                Span::raw(*what),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::styled(format!("Engine: {}", view.engine_name), dim));
    if let Some(path) = view.settings_path {
        lines.push(Line::styled(format!("Settings: {path}"), dim));
    }
    lines.push(Line::styled("Press any key to close", dim));

    let width = 46u16.min(area.width.saturating_sub(2));
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Keys "),
        ),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::PRESETS;
    use crate::game::tests::{game, play, white_game};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    fn view() -> View<'static> {
        View {
            theme: &THEMES[0],
            tick: 0,
            now: Instant::now(),
            thinking: None,
            engine_name: "built-in",
            level: Level::DEFAULT,
            browse: None,
            next_clock: None,
            notice: None,
            help_open: false,
            engine_line: None,
            settings_path: None,
        }
    }

    fn render_buf_with(game: &Game, view: &View, w: u16, h: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, game, view)).unwrap();
        term.backend().buffer().clone()
    }

    fn render_buf(game: &Game, w: u16, h: u16) -> Buffer {
        render_buf_with(game, &view(), w, h)
    }

    fn text(buf: &Buffer) -> String {
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    fn render(game: &Game, w: u16, h: u16) -> String {
        text(&render_buf(game, w, h))
    }

    fn geo(w: u16, h: u16) -> Geometry {
        compute_geometry(Rect::new(0, 0, w, h)).unwrap()
    }

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn renders_initial_position() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm(now()); // select e2
        let content = render(&g, 100, 40);
        assert!(content.contains('█'), "piece art should be rendered");
        assert!(content.contains('●'), "legal target markers should appear");
        assert!(content.contains("Chess"));
        assert!(content.contains("? help"));
        assert!(content.contains("no warranty"));
        assert!(content.contains('┏'), "cursor corners should appear");
    }

    #[test]
    fn renders_small_board_with_glyph_fallback_and_moves() {
        let mut g = white_game();
        play(
            &mut g,
            &[
                ((4, 1), (4, 3)),
                ((4, 6), (4, 4)),
                ((6, 0), (5, 2)),
                ((1, 7), (2, 5)),
                ((5, 0), (2, 3)),
                ((6, 7), (5, 5)),
                ((3, 1), (3, 2)),
                ((5, 7), (2, 4)),
                ((2, 1), (2, 2)),
                ((3, 6), (3, 5)),
            ],
        );
        let content = render(&g, 80, 27); // gives cell_h == 3 -> glyph fallback
        assert!(content.contains('♙'), "glyph fallback should render");
        assert!(content.contains('♛'));
        assert!(content.contains("no warranty"), "footer fits");
        assert!(content.contains("5. c3"), "at least 5 move rows fit");
    }

    #[test]
    fn renders_too_small_message() {
        let g = white_game();
        let content = render(&g, 40, 15);
        assert!(content.contains("too small"));
    }

    #[test]
    fn panel_shows_history_and_material() {
        let mut g = white_game();
        play(&mut g, &[((6, 0), (5, 2))]); // Nf3
        let content = render(&g, 100, 40);
        assert!(content.contains("♘f3"), "figurine notation");
        assert!(content.contains("Material"));
        assert!(content.contains("Moves"));
        assert!(content.contains("You White · AI Black"));
        assert!(content.contains("built-in · L4"));
    }

    #[test]
    fn black_to_move_start_numbers_moves() {
        let mut g = game(None, Some("4k3/8/8/8/8/8/4P3/4K3 b - - 0 12"));
        play(&mut g, &[((4, 7), (3, 7))]); // Kd8
        let content = render(&g, 100, 40);
        assert!(content.contains(" 12. …"), "{content}");
        assert!(content.contains("White vs Black"));
    }

    #[test]
    fn capture_target_is_tinted() {
        // e4 pawn can take d5
        let mut g = game(
            Some(Color::Black),
            Some("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1"),
        );
        g.cursor = (4, 3);
        g.confirm(now());
        let buf = render_buf(&g, 100, 40);
        let cell = cell_area(&geo(100, 40), Color::White, Square::D5);
        assert_eq!(buf[(cell.x + 1, cell.y + 1)].bg, THEMES[0].capture);
    }

    #[test]
    fn hint_is_tinted() {
        let mut g = white_game();
        assert!(g.set_hint(ChessMove::new(Square::E2, Square::E4, None)));
        let buf = render_buf(&g, 100, 40);
        let cell = cell_area(&geo(100, 40), Color::White, Square::E4);
        assert_eq!(buf[(cell.x + 1, cell.y + 1)].bg, THEMES[0].hint);
    }

    #[test]
    fn black_orientation_puts_rank_8_at_bottom() {
        let g = game(Some(Color::White), None);
        let buf = render_buf(&g, 100, 40);
        let geo = geo(100, 40);
        let bottom_label_y = geo.labels_left.y + 7 * geo.cell_h + geo.cell_h / 2;
        assert_eq!(buf[(geo.labels_left.x, bottom_label_y)].symbol(), "8");
        let first_file_x = geo.labels_bottom.x + geo.cell_w / 2;
        assert_eq!(buf[(first_file_x, geo.labels_bottom.y)].symbol(), "h");
    }

    #[test]
    fn eval_and_engine_line_shown() {
        let mut g = white_game();
        g.eval = Some(Eval::Cp(35));
        g.eval_depth = Some(18);
        let board = Board::default();
        let moves = [
            ChessMove::new(Square::E2, Square::E4, None),
            ChessMove::new(Square::E7, Square::E5, None),
            ChessMove::new(Square::G1, Square::F3, None),
        ];
        let mut v = view();
        v.engine_line = Some((&board, &moves));
        let content = text(&render_buf_with(&g, &v, 100, 40));
        assert!(content.contains("+0.35  d18"));
        assert!(content.contains("e4 e5 ♘f3"));
        g.eval = Some(Eval::Mate(-3));
        assert!(render(&g, 100, 40).contains("-M3"));
        assert_eq!(eval_fraction(Some(Eval::Cp(0))), 0.5);
        assert!(eval_fraction(Some(Eval::Cp(300))) > 0.6);
        assert_eq!(line_text(&board, &moves, 5), "e4 e5");
    }

    #[test]
    fn clock_row_and_next_preset() {
        let t0 = Instant::now();
        let mut g = Game::new(crate::game::GameSetup {
            start: crate::game::StartPosition::default(),
            engine_side: Some(Color::Black),
            orientation: Color::White,
            time_control: PRESETS[3], // 5+3
        });
        g.cursor = (4, 1);
        g.confirm(t0);
        g.cursor = (4, 3);
        g.confirm(t0); // Black's clock runs from t0
        let mut v = view();
        v.now = t0 + Duration::from_millis(295_500); // 4.5 s left for Black
        v.next_clock = Some(PRESETS[1]);
        let content = text(&render_buf_with(&g, &v, 100, 40));
        assert!(content.contains("W 5:03 B 0:04.5 →1+0"), "{content}");
        assert_eq!(clock_text(Duration::from_secs(754)), "12:34");
        assert!("W 0:09.3 B 15:00 →15+10".chars().count() <= 24);
    }

    #[test]
    fn game_over_result_in_title_and_panel() {
        let g = game(
            Some(Color::Black),
            Some("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3"),
        );
        let content = render(&g, 100, 40);
        assert!(content.contains("Checkmate — AI wins."));
        assert!(content.contains("r new · n swap · u undo"));
    }

    #[test]
    fn browsing_shows_the_past_position() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3)), ((4, 6), (4, 4))]);
        let mut v = view();
        v.browse = Some(0);
        let buf = render_buf_with(&g, &v, 100, 40);
        assert!(text(&buf).contains("Move 0/2 · End returns"));
        // The start position has no last move, so e2 shows its plain colour.
        let geo = geo(100, 40);
        let e2 = cell_area(&geo, Color::White, Square::E2);
        assert_eq!(buf[(e2.x + 1, e2.y + 1)].bg, THEMES[0].light);
        assert!(!text(&buf).contains('┏'), "no cursor while browsing");
    }

    #[test]
    fn promotion_popup_hit_test() {
        let area = Rect::new(0, 0, 100, 40);
        let (_, cells) = promotion_layout(&geo(100, 40));
        for (piece, rect) in cells {
            let click = Position {
                x: rect.x + 2,
                y: rect.y,
            };
            assert_eq!(promotion_choice_at(area, click), Some(piece));
        }
        assert_eq!(promotion_choice_at(area, Position { x: 0, y: 0 }), None);

        let mut g = game(Some(Color::Black), Some("7k/P7/8/8/8/8/8/K7 w - - 0 1"));
        g.cursor = (0, 6);
        g.confirm(now());
        g.cursor = (0, 7);
        g.confirm(now());
        let content = render(&g, 100, 40);
        assert!(content.contains("Promote pawn to:"));
        assert!(content.contains('♕'));
    }

    #[test]
    fn help_overlay_lists_keys() {
        let g = white_game();
        let mut v = view();
        v.help_open = true;
        v.settings_path = Some("C:/cfg/settings.txt");
        let content = text(&render_buf_with(&g, &v, 100, 40));
        assert!(content.contains("suggest a move"));
        assert!(content.contains("Settings: C:/cfg/settings.txt"));
    }

    #[test]
    fn click_maps_to_square_in_both_orientations() {
        let area = Rect::new(0, 0, 100, 40);
        let geo = geo(100, 40);
        for orientation in [Color::White, Color::Black] {
            for sq in [Square::E2, Square::A1, Square::H8] {
                let cell = cell_area(&geo, orientation, sq);
                let click = Position {
                    x: cell.x + cell.width / 2,
                    y: cell.y + cell.height / 2,
                };
                assert_eq!(square_at(area, orientation, click), Some(sq));
            }
        }
        assert_eq!(square_at(area, Color::White, Position { x: 0, y: 0 }), None);
    }

    #[test]
    fn theme_lookup_by_name() {
        assert_eq!(theme_index("Ocean"), 2);
        assert_eq!(theme_index("nope"), 0);
    }
}
