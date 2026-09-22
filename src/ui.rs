use chess::{ALL_SQUARES, Color, EMPTY, File, Piece, Rank, Square};
use ratatui::{
    Frame,
    layout::{Alignment, Position, Rect},
    style::{Color as TColor, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::engine::Eval;
use crate::game::{Game, Phase, captured_pieces, material, piece_letter};

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
        target: rgb(0xE0, 0xE0, 0xE0),
        cursor: CURSOR,
        white_piece: WHITE_PIECE,
        black_piece: BLACK_PIECE,
    },
];

/// Per-frame state that is not part of the game itself.
pub struct View<'a> {
    pub theme: &'a Theme,
    pub tick: u64,
    pub thinking: bool,
    pub engine_name: &'a str,
    pub notice: Option<&'a str>,
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

/// Board column and row (0 = top) where `sq` is drawn, with `human`'s side at the bottom.
fn screen_cell(human: Color, sq: Square) -> Position {
    let file = sq.get_file().to_index() as u16;
    let rank = sq.get_rank().to_index() as u16;
    match human {
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

fn square_from_cell(human: Color, cell: Position) -> Square {
    let (file, rank) = match human {
        Color::White => (cell.x, 7 - cell.y),
        Color::Black => (7 - cell.x, cell.y),
    };
    Square::make_square(
        Rank::from_index(rank as usize),
        File::from_index(file as usize),
    )
}

fn cell_area(geo: &Geometry, human: Color, sq: Square) -> Rect {
    let cell = screen_cell(human, sq);
    Rect {
        x: geo.board.x + cell.x * geo.cell_w,
        y: geo.board.y + cell.y * geo.cell_h,
        width: geo.cell_w,
        height: geo.cell_h,
    }
}

/// The square under a mouse click, if the click hit the board.
pub fn square_at(area: Rect, human: Color, click: Position) -> Option<Square> {
    let geo = compute_geometry(area).ok()?;
    if !geo.board.contains(click) {
        return None;
    }
    let cell = Position {
        x: (click.x - geo.board.x) / geo.cell_w,
        y: (click.y - geo.board.y) / geo.cell_h,
    };
    Some(square_from_cell(human, cell))
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

    let title = if game.game_over() {
        Span::styled(
            format!(" {} ", game.status_text()),
            Style::default()
                .fg(TColor::LightYellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(" Chess ")
    };
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title),
        geo.frame,
    );
    draw_cells(f, game, view.theme, &geo);
    if !game.game_over() {
        draw_cursor(f, game, view.theme, &geo);
    }
    draw_labels(f, game.human, &geo);
    draw_panel(f, game, view, geo.panel);
    if let Phase::Promoting { .. } = game.phase {
        draw_promotion_popup(f, &geo);
    }
}

fn draw_cells(f: &mut Frame, game: &Game, theme: &Theme, geo: &Geometry) {
    let board = &game.board;
    let check_sq = (*board.checkers() != EMPTY).then(|| board.king_square(board.side_to_move()));
    let last = game.last_move();

    for sq in ALL_SQUARES {
        let cell = cell_area(geo, game.human, sq);
        let light = (sq.get_file().to_index() + sq.get_rank().to_index()) % 2 == 1;
        let occupant = board.piece_on(sq).zip(board.color_on(sq));
        let is_target = game.legal_targets.contains(&sq);

        let mut bg = if light { theme.light } else { theme.dark };
        if last.is_some_and(|(a, b)| sq == a || sq == b) {
            bg = if light {
                theme.last_light
            } else {
                theme.last_dark
            };
        }
        if is_target && occupant.is_some() {
            bg = theme.capture;
        }
        if game.phase == Phase::Selected(sq) {
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
    let cell = cell_area(geo, game.human, game.cursor_square());
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

fn draw_labels(f: &mut Frame, human: Color, geo: &Geometry) {
    let label_style = Style::default().fg(TColor::DarkGray);
    for i in 0..8u8 {
        let rank_sq = Square::make_square(Rank::from_index(i as usize), File::A);
        let row = screen_cell(human, rank_sq).y;
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
        let col = screen_cell(human, file_sq).x;
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

fn draw_panel(f: &mut Frame, game: &Game, view: &View, rect: Rect) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(TColor::DarkGray);
    let inner_w = rect.width.saturating_sub(2) as usize;
    let inner_h = rect.height.saturating_sub(2) as usize;
    let board = &game.board;
    let human = game.human;

    let mut top: Vec<Line> = Vec::new();
    if game.game_over() {
        top.push(Line::styled(
            game.status_text(),
            Style::default()
                .fg(TColor::Black)
                .bg(TColor::LightYellow)
                .add_modifier(Modifier::BOLD),
        ));
        top.push(Line::styled("r restart·n swap·u undo", dim));
    } else if view.thinking {
        let spin = SPINNER[(view.tick as usize / 2) % SPINNER.len()];
        top.push(Line::from(vec![
            Span::styled(format!("{spin} "), Style::default().fg(TColor::Cyan)),
            Span::styled("AI thinking...", Style::default().fg(TColor::Yellow)),
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
    top.push(Line::from(""));
    top.push(Line::from(format!(
        "You: {}  AI: {}",
        color_name(human),
        color_name(!human)
    )));
    top.push(Line::styled(format!("Engine: {}", view.engine_name), dim));
    top.push(Line::from(vec![
        Span::styled("Eval ", bold),
        Span::raw(eval_label(game.eval)),
    ]));
    top.push(eval_bar(inner_w, game.eval));
    top.push(Line::from(""));

    let diff = material(board, human) - material(board, !human);
    let material_text = match diff {
        0 => "even".to_string(),
        d if d > 0 => format!("+{d} you"),
        d => format!("+{} AI", -d),
    };
    top.push(Line::from(format!("Material: {material_text}")));
    top.push(Line::from(vec![
        Span::raw("You took: "),
        captured_span(!human, &captured_pieces(board, !human)),
    ]));
    top.push(Line::from(vec![
        Span::raw("AI took:  "),
        captured_span(human, &captured_pieces(board, human)),
    ]));
    top.push(Line::from(""));
    top.push(Line::styled("Moves", bold));

    let bottom: Vec<Line> = vec![
        Line::from(""),
        Line::styled("←↑↓→/hjkl  move", dim),
        Line::styled("Enter/click select", dim),
        Line::styled("u undo  r restart", dim),
        Line::styled("n switch side", dim),
        Line::styled(format!("t theme ({})", view.theme.name), dim),
        Line::styled("Esc cancel  q quit", dim),
        Line::styled("GPL-3.0 · no warranty", dim),
    ];

    // Move list: one line per full move, newest at the bottom, last ply highlighted.
    let last = game.history.len().checked_sub(1);
    let move_lines: Vec<Line> = game
        .history
        .chunks(2)
        .enumerate()
        .map(|(i, pair)| {
            let mut spans = vec![Span::styled(format!("{:>3}. ", i + 1), dim)];
            for (j, ply) in pair.iter().enumerate() {
                let text = format!("{:<9}", figurine(&ply.san, ply.before.side_to_move()));
                let style = if Some(i * 2 + j) == last {
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(text, style));
            }
            Line::from(spans)
        })
        .collect();
    let rows = inner_h.saturating_sub(top.len() + bottom.len());
    let start = move_lines.len().saturating_sub(rows);

    let lines: Vec<Line> = top
        .into_iter()
        .chain(move_lines.into_iter().skip(start))
        .chain(bottom)
        .collect();
    let panel = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );
    f.render_widget(panel, rect);
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

fn draw_promotion_popup(f: &mut Frame, geo: &Geometry) {
    let w = 24u16.min(geo.frame.width);
    let h = 7u16;
    let area = Rect {
        x: geo.frame.x + (geo.frame.width.saturating_sub(w)) / 2,
        y: geo.frame.y + (geo.frame.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let keys = Style::default().fg(TColor::LightMagenta);
    f.render_widget(Clear, area);
    let popup = Paragraph::new(vec![
        Line::styled(
            "Promote pawn to:",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        Line::styled("[Q]ueen   [R]ook", keys),
        Line::styled("[B]ishop  k[N]ight", keys),
        Line::styled("Esc cancel", Style::default().fg(TColor::DarkGray)),
    ])
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .title("Promotion"),
    );
    f.render_widget(popup, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Game;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
    use std::str::FromStr;

    fn view() -> View<'static> {
        View {
            theme: &THEMES[0],
            tick: 0,
            thinking: false,
            engine_name: "built-in",
            notice: None,
        }
    }

    fn render_buf(game: &Game, w: u16, h: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, game, &view())).unwrap();
        term.backend().buffer().clone()
    }

    fn render(game: &Game, w: u16, h: u16) -> String {
        render_buf(game, w, h)
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn geo(w: u16, h: u16) -> Geometry {
        compute_geometry(Rect::new(0, 0, w, h)).unwrap()
    }

    #[test]
    fn renders_initial_position() {
        let mut g = Game::new(Color::White);
        g.cursor = (4, 1);
        g.confirm(); // select e2
        let content = render(&g, 100, 40);
        assert!(content.contains('█'), "piece art should be rendered");
        assert!(content.contains('●'), "legal target markers should appear");
        assert!(content.contains("Chess"));
        assert!(content.contains("Enter"));
        assert!(content.contains('┏'), "cursor corners should appear");
    }

    #[test]
    fn renders_small_board_with_glyph_fallback() {
        let g = Game::new(Color::White);
        let content = render(&g, 80, 27); // gives cell_h == 3 -> glyph fallback
        assert!(content.contains('♙'), "glyph fallback should render");
        assert!(content.contains('♛'));
    }

    #[test]
    fn renders_too_small_message() {
        let g = Game::new(Color::White);
        let content = render(&g, 40, 15);
        assert!(content.contains("too small"));
    }

    #[test]
    fn panel_shows_history_and_material() {
        let mut g = Game::new(Color::White);
        g.cursor = (6, 0);
        g.confirm();
        g.cursor = (5, 2);
        assert!(g.confirm()); // Nf3
        let content = render(&g, 100, 40);
        assert!(content.contains("♘f3"), "figurine notation");
        assert!(content.contains("Material"));
        assert!(content.contains("Moves"));
    }

    #[test]
    fn capture_target_is_tinted() {
        // e4 pawn can take d5
        let mut g = Game::new(Color::White);
        g.board =
            chess::Board::from_str("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 1")
                .unwrap();
        g.cursor = (4, 3);
        g.confirm();
        let buf = render_buf(&g, 100, 40);
        let cell = cell_area(&geo(100, 40), Color::White, Square::D5);
        assert_eq!(buf[(cell.x + 1, cell.y + 1)].bg, THEMES[0].capture);
    }

    #[test]
    fn black_orientation_puts_rank_8_at_bottom() {
        let g = Game::new(Color::Black);
        let buf = render_buf(&g, 100, 40);
        let geo = geo(100, 40);
        let bottom_label_y = geo.labels_left.y + 7 * geo.cell_h + geo.cell_h / 2;
        assert_eq!(buf[(geo.labels_left.x, bottom_label_y)].symbol(), "8");
        let first_file_x = geo.labels_bottom.x + geo.cell_w / 2;
        assert_eq!(buf[(first_file_x, geo.labels_bottom.y)].symbol(), "h");
    }

    #[test]
    fn eval_shown_in_panel() {
        let mut g = Game::new(Color::White);
        g.eval = Some(Eval::Cp(35));
        assert!(render(&g, 100, 40).contains("+0.35"));
        g.eval = Some(Eval::Mate(-3));
        assert!(render(&g, 100, 40).contains("-M3"));
        assert_eq!(eval_fraction(Some(Eval::Cp(0))), 0.5);
        assert!(eval_fraction(Some(Eval::Cp(300))) > 0.6);
    }

    #[test]
    fn game_over_result_in_title() {
        let mut g = Game::new(Color::White);
        g.board =
            chess::Board::from_str("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
                .unwrap();
        let content = render(&g, 100, 40);
        assert!(content.contains("Checkmate — AI wins."));
        assert!(content.contains("r restart"));
    }

    #[test]
    fn click_maps_to_square_in_both_orientations() {
        let area = Rect::new(0, 0, 100, 40);
        let geo = geo(100, 40);
        for human in [Color::White, Color::Black] {
            for sq in [Square::E2, Square::A1, Square::H8] {
                let cell = cell_area(&geo, human, sq);
                let click = Position {
                    x: cell.x + cell.width / 2,
                    y: cell.y + cell.height / 2,
                };
                assert_eq!(square_at(area, human, click), Some(sq));
            }
        }
        assert_eq!(square_at(area, Color::White, Position { x: 0, y: 0 }), None);
    }
}
