use std::time::{Duration, Instant};

use chess::{ALL_SQUARES, Board, ChessMove, Color, EMPTY, File, Piece, Rank, Square};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Alignment, Position, Rect, Size},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::art::{self, ArtSize, Shift};
use crate::clock::{self, TimeControl};
use crate::engine::{Eval, Level};
use crate::game::{Game, Phase, captured_pieces, material, piece_letter, san};
use crate::menu::Menu;
use crate::theme::{self, BoardColors, Chrome, Theme};

const PANEL_W: u16 = 26;

/// How long a moved piece slides: a base time plus a little per square travelled.
const SLIDE_BASE: Duration = Duration::from_millis(120);
const SLIDE_PER_SQUARE: Duration = Duration::from_millis(40);

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// What is drawn on top of the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Overlay {
    Help,
    About,
    Confirm { question: &'static str, yes: bool },
    Menu { menu: usize, item: usize },
}

/// Per-frame state that is not part of the game itself.
pub struct View<'a> {
    pub theme: &'a Theme,
    /// Frame of the thinking spinner.
    pub spin: usize,
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
    pub overlay: Option<Overlay>,
    pub menus: Vec<Menu>,
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

/// The desktop between the menu bar (top row) and the status bar (bottom row).
fn body(area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.y + 1.min(area.height),
        width: area.width,
        height: area.height.saturating_sub(2),
    }
}

/// A `width` x `height` rectangle in the middle of `area`, shrunk to fit.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
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
    /// `None` = cells too small for art; pieces are drawn as glyphs.
    art: Option<ArtSize>,
}

/// Board and panel layout for the whole screen `area`; the bars are taken off first.
fn compute_geometry(area: Rect) -> Result<Geometry, ()> {
    let area = body(area);
    // height budget: frame top+bottom + file-label row
    let avail_h = area.height.saturating_sub(3);
    // tall enough for the largest piece art with its padding
    let cell_h = (avail_h / 8).clamp(0, ArtSize::Large.min_cell().height);
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
        height: frame.height, // ends level with the board frame
    };
    Ok(Geometry {
        frame,
        board,
        cell_w,
        cell_h,
        labels_left,
        labels_bottom,
        panel,
        art: ArtSize::for_cell(Size {
            width: cell_w,
            height: cell_h,
        }),
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

// ----- menus and dialogs: layout shared by drawing and hit-testing ------------------------

/// Each menu title's clickable area on the menu bar.
fn menu_title_rects(area: Rect, menus: &[Menu]) -> Vec<Rect> {
    let mut x = area.x + 1;
    menus
        .iter()
        .map(|menu| {
            let width = menu.title.chars().count() as u16 + 2;
            let rect = Rect {
                x,
                y: area.y,
                width,
                height: 1,
            };
            x += width;
            rect.intersection(area)
        })
        .collect()
}

/// The menu whose title is under a mouse click.
pub fn menu_title_at(area: Rect, menus: &[Menu], click: Position) -> Option<usize> {
    menu_title_rects(area, menus)
        .iter()
        .position(|rect| rect.contains(click))
}

/// An open menu's box, one row per item and the rows of separator lines.
struct Dropdown {
    rect: Rect,
    items: Vec<Rect>,
    separators: Vec<u16>,
    label_w: usize,
    shortcut_w: usize,
}

fn dropdown_layout(area: Rect, menus: &[Menu], menu: usize) -> Option<Dropdown> {
    let items = &menus.get(menu)?.items;
    let title = *menu_title_rects(area, menus).get(menu)?;
    let label_w = items.iter().map(|i| i.label.chars().count()).max()?;
    let shortcut_w = items
        .iter()
        .filter_map(|i| i.shortcut)
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0);
    // " • label  shortcut "
    let inner_w = 4 + label_w + if shortcut_w > 0 { 2 + shortcut_w } else { 0 };
    let width = inner_w as u16 + 2;
    let separators = items.iter().filter(|i| i.separator_before).count() as u16;
    let rect = Rect {
        x: title.x.min((area.x + area.width).saturating_sub(width)),
        y: area.y + 1,
        width,
        height: items.len() as u16 + separators + 2,
    };
    let mut y = rect.y + 1;
    let mut item_rects = Vec::new();
    let mut separator_rows = Vec::new();
    for item in items {
        if item.separator_before {
            separator_rows.push(y);
            y += 1;
        }
        item_rects.push(Rect {
            x: rect.x + 1,
            y,
            width: inner_w as u16,
            height: 1,
        });
        y += 1;
    }
    Some(Dropdown {
        rect,
        items: item_rects,
        separators: separator_rows,
        label_w,
        shortcut_w,
    })
}

/// Where a click lands relative to an open menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuHit {
    Item(usize),
    /// On the menu's border or a separator.
    Inside,
    Outside,
}

pub fn menu_item_at(area: Rect, menus: &[Menu], menu: usize, click: Position) -> MenuHit {
    let Some(dropdown) = dropdown_layout(area, menus, menu) else {
        return MenuHit::Outside;
    };
    if let Some(i) = dropdown.items.iter().position(|r| r.contains(click)) {
        MenuHit::Item(i)
    } else if dropdown.rect.contains(click) {
        MenuHit::Inside
    } else {
        MenuHit::Outside
    }
}

const CONFIRM_W: u16 = 40;
const CONFIRM_H: u16 = 7;
const BUTTON_W: u16 = 8;

/// The confirmation dialog and its Yes and No buttons.
struct ConfirmLayout {
    dialog: Rect,
    yes: Rect,
    no: Rect,
}

fn confirm_layout(area: Rect) -> ConfirmLayout {
    let dialog = centered(area, CONFIRM_W, CONFIRM_H);
    let x = dialog.x + dialog.width.saturating_sub(2 * BUTTON_W + 2) / 2;
    let button = |x| Rect {
        x,
        y: dialog.y + 4,
        width: BUTTON_W,
        height: 1,
    };
    ConfirmLayout {
        dialog,
        yes: button(x),
        no: button(x + BUTTON_W + 2),
    }
}

/// `Some(true)` for a click on Yes, `Some(false)` on No.
pub fn confirm_button_at(area: Rect, click: Position) -> Option<bool> {
    let layout = confirm_layout(area);
    if layout.yes.contains(click) {
        Some(true)
    } else if layout.no.contains(click) {
        Some(false)
    } else {
        None
    }
}

// ----- drawing ------------------------------------------------------------------------------

pub fn draw(f: &mut Frame, game: &Game, view: &View) {
    let area = f.area();
    let chrome = &view.theme.chrome;
    f.render_widget(
        Block::default().style(Style::default().bg(chrome.desktop).fg(chrome.text)),
        body(area),
    );
    draw_menu_bar(f, game, view);
    draw_status_bar(f, view);
    match compute_geometry(area) {
        Ok(geo) => draw_game(f, game, view, &geo),
        Err(()) => {
            let msg = Paragraph::new(format!(
                "Terminal too small ({}x{}). Maximize the window or reduce font size.",
                area.width, area.height
            ))
            .alignment(Alignment::Center);
            f.render_widget(msg, body(area));
        }
    }
    match view.overlay {
        Some(Overlay::Help) => draw_help(f, chrome),
        Some(Overlay::About) => draw_about(f, view),
        Some(Overlay::Confirm { question, yes }) => draw_confirm(f, chrome, question, yes),
        Some(Overlay::Menu { menu, item }) => draw_dropdown(f, view, menu, item),
        None => {}
    }
}

fn bar_style(chrome: &Chrome) -> Style {
    Style::default().bg(chrome.bar).fg(chrome.bar_text)
}

fn select_style() -> Style {
    Style::default().bg(theme::SELECT_BG).fg(theme::SELECT_FG)
}

/// `label` in `base`, with the first occurrence of `key` (any case) in `hot`.
fn keyed(label: &str, key: Option<char>, base: Style, hot: Style) -> Vec<Span<'static>> {
    let found = key.and_then(|key| {
        label
            .char_indices()
            .find(|(_, c)| c.to_ascii_lowercase() == key)
    });
    match found {
        Some((i, c)) => {
            let end = i + c.len_utf8();
            vec![
                Span::styled(label[..i].to_string(), base),
                Span::styled(label[i..end].to_string(), hot),
                Span::styled(label[end..].to_string(), base),
            ]
        }
        None => vec![Span::styled(label.to_string(), base)],
    }
}

/// A single-line box with a centered title, as used for the board and the panel.
fn titled_box(chrome: &Chrome, title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(chrome.border))
        .title(title)
        .title_alignment(Alignment::Center)
        .title_style(
            Style::default()
                .fg(chrome.title)
                .add_modifier(Modifier::BOLD),
        )
}

/// Draws the frame of a dialog with its drop shadow; returns the area inside the border.
fn dialog(f: &mut Frame, rect: Rect, title: &str, chrome: &Chrome) -> Rect {
    let rect = rect.intersection(f.area());
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(chrome.dialog).fg(chrome.dialog_text))
        .title(format!(" {title} "))
        .title_alignment(Alignment::Center)
        .title_style(
            Style::default()
                .fg(chrome.dialog_title)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(rect);
    f.render_widget(Clear, rect);
    f.render_widget(block, rect);
    shadow(f.buffer_mut(), rect);
    inner
}

/// DOS-style shadow: two columns right of `rect` and one row below, characters kept but dimmed.
fn shadow(buf: &mut Buffer, rect: Rect) {
    let right = rect.x + rect.width;
    let bottom = rect.y + rect.height;
    let cells = (rect.y + 1..=bottom)
        .flat_map(|y| (right..right + 2).map(move |x| (x, y)))
        .chain((rect.x + 2..right).map(|x| (x, bottom)));
    for (x, y) in cells {
        if let Some(cell) = buf.cell_mut(Position { x, y }) {
            cell.set_bg(theme::SHADOW_BG).set_fg(theme::SHADOW_FG);
        }
    }
}

fn draw_menu_bar(f: &mut Frame, game: &Game, view: &View) {
    let area = f.area();
    if area.height == 0 {
        return;
    }
    let chrome = &view.theme.chrome;
    let base = bar_style(chrome);
    let bar = Rect { height: 1, ..area };
    f.render_widget(Block::default().style(base), bar);

    let open = match view.overlay {
        Some(Overlay::Menu { menu, .. }) => Some(menu),
        _ => None,
    };
    let rects = menu_title_rects(area, &view.menus);
    for (i, (menu, &rect)) in view.menus.iter().zip(&rects).enumerate() {
        let style = if open == Some(i) {
            select_style()
        } else {
            base
        };
        let mut spans = vec![Span::styled(" ", style)];
        spans.extend(keyed(
            menu.title,
            Some(menu.key),
            style,
            style.fg(chrome.hotkey),
        ));
        spans.push(Span::styled(" ", style));
        f.render_widget(Paragraph::new(Line::from(spans)), rect);
    }

    let titles_end = rects.last().map_or(area.x, |r| r.x + r.width);
    let right = Rect {
        x: titles_end + 1,
        width: (area.x + area.width).saturating_sub(titles_end + 1),
        ..bar
    };
    let mut spans = vec![Span::styled(view.engine_name.to_string(), base)];
    if game.engine_side.is_some() {
        spans.push(Span::styled(" · ", base));
        spans.push(Span::styled(
            format!("Level {}", view.level.get()),
            base.fg(chrome.hotkey),
        ));
    }
    spans.push(Span::styled(" ", base));
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        right,
    );
}

const LEGAL_NOTICE: &str = "GPL-3.0 · no warranty ";

fn draw_status_bar(f: &mut Frame, view: &View) {
    let area = f.area();
    if area.height < 2 {
        return;
    }
    let chrome = &view.theme.chrome;
    let base = bar_style(chrome);
    let bar = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };
    f.render_widget(
        Paragraph::new(LEGAL_NOTICE)
            .style(base)
            .alignment(Alignment::Right),
        bar,
    );
    let left = Rect {
        width: bar
            .width
            .saturating_sub(LEGAL_NOTICE.chars().count() as u16 + 1),
        ..bar
    };
    let hot = base.fg(chrome.hotkey);
    let line = match view.notice {
        Some(notice) => Line::styled(format!(" {notice}"), hot.add_modifier(Modifier::BOLD)),
        None => Line::from(vec![
            Span::styled(" F1", hot),
            Span::styled(" Help  ", base),
            Span::styled("F10", hot),
            Span::styled(" Menu", base),
        ]),
    };
    f.render_widget(Paragraph::new(line).style(base), left);
}

fn draw_game(f: &mut Frame, game: &Game, view: &View, geo: &Geometry) {
    let chrome = &view.theme.chrome;
    let title = match view.browse {
        Some(ply) => format!(" Move {ply}/{} · End returns ", game.history.len()),
        None if game.game_over() => format!(" {} ", game.status_text()),
        None => " Chess ".to_string(),
    };
    f.render_widget(titled_box(chrome, title), geo.frame);
    let live = view.browse.is_none();
    let (board, last) = match view.browse {
        Some(ply) => game.position_at(ply),
        None => (game.board, game.last_move()),
    };
    let moving = live.then(|| animation(game, view.now)).flatten();
    let hidden: Vec<Square> = moving
        .iter()
        .flat_map(|m| m.slides.iter().map(|s| s.from))
        .collect();
    draw_cells(
        f,
        &Shown {
            game,
            board: &board,
            pieces: moving.as_ref().map_or(&board, |m| &m.before),
            hidden: &hidden,
            last,
            live,
        },
        &view.theme.board,
        geo,
    );
    if let Some(moving) = &moving {
        draw_slides(f, moving, game.orientation, &view.theme.board, geo);
    }
    if live && !game.game_over() {
        draw_cursor(f, game, &view.theme.board, geo);
    }
    draw_labels(f, game.orientation, chrome, geo);
    draw_panel(f, game, view, geo.panel);
    if live && let Phase::Promoting { .. } = game.phase {
        draw_promotion_popup(f, geo, chrome, game.board.side_to_move());
    }
}

/// The position on screen: live, or a past one while browsing.
struct Shown<'a> {
    game: &'a Game,
    /// The position the highlights belong to.
    board: &'a Board,
    /// The position whose pieces are drawn: `board`, or the one before a sliding move.
    pieces: &'a Board,
    /// Squares whose piece in `pieces` is not drawn, because it is sliding away.
    hidden: &'a [Square],
    last: Option<(Square, Square)>,
    /// Selection, targets and hint only show on the live position.
    live: bool,
}

fn draw_cells(f: &mut Frame, shown: &Shown, colors: &BoardColors, geo: &Geometry) {
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
        let occupant = shown
            .pieces
            .piece_on(sq)
            .zip(shown.pieces.color_on(sq))
            .filter(|_| !shown.hidden.contains(&sq));
        let is_target = shown.live && game.legal_targets.contains(&sq);

        let mut bg = if light { colors.light } else { colors.dark };
        if shown.last.is_some_and(|(a, b)| sq == a || sq == b) {
            bg = if light {
                colors.last_light
            } else {
                colors.last_dark
            };
        }
        if hint.is_some_and(|(a, b)| sq == a || sq == b) {
            bg = colors.hint;
        }
        if is_target && board.piece_on(sq).is_some() {
            bg = colors.capture;
        }
        if shown.live && game.phase == Phase::Selected(sq) {
            bg = colors.select;
        }
        if Some(sq) == check_sq {
            bg = colors.check;
        }

        let base = Style::default().bg(bg);
        let lines: Vec<Line> = if let Some((piece, color)) = occupant {
            piece_lines(piece, color, colors, geo)
        } else if is_target {
            mid_row(geo, "●".to_string(), base.fg(colors.target))
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

/// `text` on the middle row of a board cell.
fn mid_row(geo: &Geometry, text: String, style: Style) -> Vec<Line<'static>> {
    let mid = geo.cell_h / 2;
    (0..geo.cell_h)
        .map(|i| {
            if i == mid {
                Line::styled(text.clone(), style)
            } else {
                Line::from("")
            }
        })
        .collect()
}

fn piece_style(color: Color, colors: &BoardColors) -> Style {
    let fg = match color {
        Color::White => colors.white_piece,
        Color::Black => colors.black_piece,
    };
    Style::default().fg(fg).add_modifier(Modifier::BOLD)
}

/// A piece as drawn in its board cell (centre-aligned): art padded from the top, or a glyph.
fn piece_lines(
    piece: Piece,
    color: Color,
    colors: &BoardColors,
    geo: &Geometry,
) -> Vec<Line<'static>> {
    let style = piece_style(color, colors);
    match geo.art {
        Some(size) => {
            let pad_top = (geo.cell_h - size.size().height) / 2;
            (0..pad_top)
                .map(|_| Line::from(""))
                .chain(
                    art::piece_art(piece, size)
                        .into_iter()
                        .map(|row| Line::styled(row, style)),
                )
                .collect()
        }
        None => mid_row(geo, piece_glyph(color, piece).to_string(), style),
    }
}

/// A piece travelling from one square to another while a move animates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Slide {
    piece: Piece,
    color: Color,
    from: Square,
    to: Square,
}

/// The pieces `mv` moves when played on `before`: one, or king and rook when castling.
fn slides(before: &Board, mv: ChessMove) -> Vec<Slide> {
    let (from, to) = (mv.get_source(), mv.get_dest());
    let (Some(piece), Some(color)) = (before.piece_on(from), before.color_on(from)) else {
        return Vec::new();
    };
    let mut slides = vec![Slide {
        piece,
        color,
        from,
        to,
    }];
    let (from_file, to_file) = (from.get_file().to_index(), to.get_file().to_index());
    if piece == Piece::King && from_file.abs_diff(to_file) == 2 {
        let (rook_from, rook_to) = if to_file > from_file {
            (File::H, File::F)
        } else {
            (File::A, File::D)
        };
        let rank = from.get_rank();
        slides.push(Slide {
            piece: Piece::Rook,
            color,
            from: Square::make_square(rank, rook_from),
            to: Square::make_square(rank, rook_to),
        });
    }
    slides
}

/// The last move while it animates: the position it was played from, and what slides.
struct Animation {
    before: Board,
    slides: Vec<Slide>,
    /// 0 = on the source squares, 1 = arrived.
    progress: f32,
}

/// How long `mv` slides: longer moves take longer, but travel faster.
fn slide_duration(mv: ChessMove) -> Duration {
    let (from, to) = (mv.get_source(), mv.get_dest());
    let file = |sq: Square| sq.get_file().to_index();
    let rank = |sq: Square| sq.get_rank().to_index();
    let files = file(from).abs_diff(file(to));
    let ranks = rank(from).abs_diff(rank(to));
    SLIDE_BASE + SLIDE_PER_SQUARE * files.max(ranks) as u32
}

/// How far through its animation the last move is (0..1), while it animates.
fn animation_time(game: &Game, now: Instant) -> Option<f32> {
    let elapsed = now.saturating_duration_since(game.moved_at?);
    let duration = slide_duration(game.history.last()?.mv);
    (elapsed < duration).then(|| elapsed.as_secs_f32() / duration.as_secs_f32())
}

/// Whether the last move is still sliding, so frames should come faster.
pub fn animating(game: &Game, now: Instant) -> bool {
    animation_time(game, now).is_some()
}

fn animation(game: &Game, now: Instant) -> Option<Animation> {
    let t = animation_time(game, now)?;
    let ply = game.history.last()?;
    Some(Animation {
        before: ply.before,
        slides: slides(&ply.before, ply.mv),
        progress: t * t * (3.0 - 2.0 * t), // ease in and out
    })
}

/// Where a piece drawn in `sq`'s cell starts: its top-left character, as `piece_lines` places
/// it under centre alignment.
fn piece_origin(geo: &Geometry, orientation: Color, sq: Square) -> Position {
    let cell = cell_area(geo, orientation, sq);
    let (dx, dy) = match geo.art {
        Some(size) => {
            let Size { width, height } = size.size();
            (geo.cell_w / 2 - width / 2, (geo.cell_h - height) / 2)
        }
        None => (geo.cell_w / 2, geo.cell_h / 2),
    };
    Position {
        x: cell.x + dx,
        y: cell.y + dy,
    }
}

/// `slide`'s piece at `progress`: its rows and the character they start at. Art moves in
/// quadrant pixels (half characters), glyphs in whole characters.
fn sprite(
    geo: &Geometry,
    orientation: Color,
    slide: &Slide,
    progress: f32,
) -> (Vec<String>, Position) {
    let steps: u16 = if geo.art.is_some() { 2 } else { 1 }; // positions per character
    let from = piece_origin(geo, orientation, slide.from);
    let to = piece_origin(geo, orientation, slide.to);
    let lerp = |a: u16, b: u16| {
        let (a, b) = (f32::from(a * steps), f32::from(b * steps));
        (a + (b - a) * progress).round() as u16
    };
    let (x, y) = (lerp(from.x, to.x), lerp(from.y, to.y));
    let rows = match geo.art {
        Some(size) => art::shifted_piece_art(
            slide.piece,
            size,
            Shift {
                right: x % 2 == 1,
                down: y % 2 == 1,
            },
        ),
        None => vec![piece_glyph(slide.color, slide.piece).to_string()],
    };
    let at = Position {
        x: x / steps,
        y: y / steps,
    };
    (rows, at)
}

/// The sliding pieces over the board; blank parts of a piece let the squares show through.
fn draw_slides(
    f: &mut Frame,
    moving: &Animation,
    orientation: Color,
    colors: &BoardColors,
    geo: &Geometry,
) {
    let buf = f.buffer_mut();
    for slide in &moving.slides {
        let style = piece_style(slide.color, colors);
        let (rows, at) = sprite(geo, orientation, slide, moving.progress);
        for (dy, row) in (0..).zip(&rows) {
            for (dx, c) in (0..).zip(row.chars()) {
                if c != ' ' {
                    buf[(at.x + dx, at.y + dy)].set_char(c).set_style(style);
                }
            }
        }
    }
}

/// Corner brackets only, so the piece inside stays visible.
fn draw_cursor(f: &mut Frame, game: &Game, colors: &BoardColors, geo: &Geometry) {
    let cell = cell_area(geo, game.orientation, game.cursor_square());
    let right = cell.x + cell.width - 1;
    let bottom = cell.y + cell.height - 1;
    let style = Style::default()
        .fg(colors.cursor)
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

fn draw_labels(f: &mut Frame, orientation: Color, chrome: &Chrome, geo: &Geometry) {
    let label_style = Style::default().fg(chrome.text);
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
        Span::styled("█".repeat(white), Style::default().fg(theme::WHITE)),
        Span::styled(
            "█".repeat(width - white),
            Style::default().fg(theme::DARK_GRAY),
        ),
    ])
}

/// SAN with piece letters replaced by figurines of the mover's colour, each followed by a
/// space so it does not run into the next character: "♘ f3", "e8=♕ +".
fn figurine(san: &str, mover: Color) -> String {
    let mut out = String::new();
    let mut chars = san.chars().peekable();
    while let Some(c) = chars.next() {
        let piece = [
            Piece::King,
            Piece::Queen,
            Piece::Rook,
            Piece::Bishop,
            Piece::Knight,
        ]
        .into_iter()
        .find(|&p| piece_letter(p) == c);
        match piece {
            Some(p) => {
                out.push(piece_glyph(mover, p));
                if chars.peek().is_some() {
                    out.push(' ');
                }
            }
            None => out.push(c),
        }
    }
    out
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
    let chrome = &view.theme.chrome;
    let mut spans = Vec::new();
    match &game.clock {
        None => spans.push(Span::raw("No clock")),
        Some(clock) => {
            for (color, label) in [(Color::White, "W "), (Color::Black, "B ")] {
                let left = clock.remaining(color, view.now);
                let mut style = Style::default();
                if clock.running() == Some(color) {
                    style = style.fg(chrome.value).add_modifier(Modifier::BOLD);
                }
                if left.is_zero() {
                    style = style.fg(chrome.alert);
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
            Style::default().fg(chrome.dim),
        ));
    }
    Line::from(spans)
}

/// Inner lines of the GAME box, which keeps this height so the boxes below never move.
const GAME_LINES: usize = 4;

fn draw_panel(f: &mut Frame, game: &Game, view: &View, rect: Rect) {
    let chrome = &view.theme.chrome;
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(chrome.dim);
    let inner_w = rect.width.saturating_sub(2) as usize;
    let board = &game.board;

    let mut status: Vec<Line> = Vec::new();
    if game.game_over() {
        status.push(Line::styled(
            game.status_text(),
            Style::default()
                .fg(theme::BLACK)
                .bg(chrome.value)
                .add_modifier(Modifier::BOLD),
        ));
    } else if let Some(label) = view.thinking {
        let spin = SPINNER[view.spin % SPINNER.len()];
        status.push(Line::from(vec![
            Span::styled(format!("{spin} "), dim),
            Span::styled(label.to_string(), Style::default().fg(chrome.value)),
        ]));
    } else {
        let text = game.status_text();
        let color = if text.contains("Check") {
            chrome.alert
        } else {
            chrome.value
        };
        status.push(Line::styled(text, Style::default().fg(color)));
    }
    status.push(match game.engine_side {
        Some(engine) => Line::from(format!(
            "You {} · AI {}",
            color_name(!engine),
            color_name(engine)
        )),
        None => Line::from("White vs Black"),
    });
    status.push(clock_row(game, view));
    if game.game_over() {
        let keys = if game.is_final() {
            "r new · n swap"
        } else {
            "r new · n swap · u undo"
        };
        status.push(Line::styled(keys, dim));
    }

    let depth = game.eval_depth.map_or(String::new(), |d| format!("  d{d}"));
    let line = view
        .engine_line
        .map_or(String::new(), |(b, moves)| line_text(b, moves, inner_w));
    let evaluation = vec![
        Line::from(vec![
            Span::styled("Eval ", bold),
            Span::raw(format!("{}{depth}", eval_label(game.eval))),
        ]),
        eval_bar(inner_w, game.eval),
        Line::styled(line, dim),
    ];

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
    let material_lines = vec![
        Line::from(format!("Material {material_text}")),
        Line::from(vec![
            Span::raw(me_label),
            captured_span(them, &captured_pieces(board, them)),
        ]),
        Line::from(vec![
            Span::raw(them_label),
            captured_span(me, &captured_pieces(board, me)),
        ]),
    ];

    // Boxes stacked top to bottom; MOVES takes the rest and shows the latest moves.
    let fixed = GAME_LINES as u16 + 2 + 5 + 5;
    let rows = rect.height.saturating_sub(fixed + 2) as usize;
    let move_lines = move_rows(game, view.browse, chrome);
    let start = move_lines.len().saturating_sub(rows);
    let moves = move_lines.into_iter().skip(start).collect();
    let sections = [
        ("Game", status, GAME_LINES as u16 + 2),
        ("Evaluation", evaluation, 5),
        ("Material", material_lines, 5),
        ("Moves", moves, rect.height.saturating_sub(fixed)),
    ];
    let bottom = rect.y + rect.height;
    let mut y = rect.y;
    for (title, lines, height) in sections {
        let height = height.min(bottom - y);
        f.render_widget(
            Paragraph::new(lines).block(titled_box(chrome, format!(" {title} "))),
            Rect { y, height, ..rect },
        );
        y += height;
    }
}

/// One row per full move, numbered from the start position; the move leading to the shown
/// position is highlighted.
fn move_rows(game: &Game, browse: Option<usize>, chrome: &Chrome) -> Vec<Line<'static>> {
    let dim = Style::default().fg(chrome.dim);
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
                    bar_style(chrome).add_modifier(Modifier::BOLD)
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
        Color::White => theme::WHITE,
        Color::Black => theme::LIGHT_GRAY,
    };
    Span::styled(s, Style::default().fg(fg))
}

fn draw_promotion_popup(f: &mut Frame, geo: &Geometry, chrome: &Chrome, color: Color) {
    let (popup, cells) = promotion_layout(geo);
    let inner = dialog(f, popup, "Promotion", chrome);
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Promote pawn to:",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::from(""),
            Line::from(""),
            Line::from(""),
            Line::from("click or key · Esc"),
        ])
        .alignment(Alignment::Center),
        inner,
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
                    Style::default()
                        .fg(chrome.dialog_title)
                        .add_modifier(Modifier::BOLD),
                ),
            ])
            .alignment(Alignment::Center),
            rect,
        );
    }
}

const HELP: [(&str, &str); 20] = [
    ("←↑↓→ / hjkl", "move the cursor"),
    ("Enter / click", "select, then move"),
    ("drag", "move a piece with the mouse"),
    ("Esc", "cancel"),
    ("F10 / Alt+key", "menus"),
    ("u", "undo"),
    ("r", "restart"),
    ("n", "new game, other side"),
    ("m", "engine / two players"),
    ("f", "flip the board"),
    ("+ / -", "stronger / weaker engine"),
    ("s", "suggest a move"),
    ("c", "clock for the next game"),
    ("t", "next theme"),
    ("x", "resign"),
    ("d", "offer a draw"),
    (", / .", "step through the game"),
    ("Home / End", "first / current position"),
    ("? / F1", "this help"),
    ("q", "quit"),
];

/// A dialog sized to `lines`, centered on the screen.
fn text_dialog(f: &mut Frame, chrome: &Chrome, title: &str, width: u16, lines: Vec<Line>) {
    let area = f.area();
    let rect = centered(area, width, lines.len() as u16 + 2);
    let inner = dialog(f, rect, title, chrome);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_help(f: &mut Frame, chrome: &Chrome) {
    let key = Style::default()
        .fg(chrome.dialog_title)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = HELP
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(format!(" {k:<14}"), key),
                Span::raw(*what),
            ])
        })
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(" Press any key to close"));
    text_dialog(f, chrome, "Keys", 46, lines);
}

fn draw_about(f: &mut Frame, view: &View) {
    let mut lines = vec![
        Line::styled(
            format!(" TuiChess {}", env!("CARGO_PKG_VERSION")),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::from(format!(" by {}", env!("CARGO_PKG_AUTHORS"))),
        Line::from(""),
        Line::from(format!(" Engine: {}", view.engine_name)),
    ];
    if let Some(path) = view.settings_path {
        lines.push(Line::from(format!(" Settings: {path}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(" GPL-3.0-or-later · no warranty"));
    lines.push(Line::from(" Press any key to close"));
    text_dialog(f, &view.theme.chrome, "About", 60, lines);
}

fn draw_confirm(f: &mut Frame, chrome: &Chrome, question: &str, yes: bool) {
    let layout = confirm_layout(f.area());
    let inner = dialog(f, layout.dialog, "Confirm", chrome);
    f.render_widget(
        Paragraph::new(vec![Line::from(""), Line::from(question.to_string())])
            .alignment(Alignment::Center),
        inner,
    );
    for (label, rect, focused) in [("Yes", layout.yes, yes), ("No", layout.no, !yes)] {
        let style = if focused {
            select_style()
        } else {
            bar_style(chrome)
        };
        let key = label.chars().next().map(|c| c.to_ascii_lowercase());
        let mut spans = vec![Span::styled("< ", style)];
        spans.extend(keyed(label, key, style, style.fg(chrome.hotkey)));
        spans.push(Span::styled(" >", style));
        f.render_widget(
            Paragraph::new(Line::from(spans))
                .style(style)
                .alignment(Alignment::Center),
            rect.intersection(f.area()),
        );
    }
}

fn draw_dropdown(f: &mut Frame, view: &View, menu: usize, selected: usize) {
    let area = f.area();
    let Some(dropdown) = dropdown_layout(area, &view.menus, menu) else {
        return;
    };
    let chrome = &view.theme.chrome;
    let base = bar_style(chrome);
    let rect = dropdown.rect.intersection(area);
    f.render_widget(Clear, rect);
    f.render_widget(Block::default().borders(Borders::ALL).style(base), rect);
    for &y in &dropdown.separators {
        let line = format!("├{}┤", "─".repeat(rect.width.saturating_sub(2) as usize));
        f.render_widget(
            Paragraph::new(line).style(base),
            Rect {
                y,
                height: 1,
                ..rect
            }
            .intersection(area),
        );
    }
    for (i, (item, &row)) in view.menus[menu]
        .items
        .iter()
        .zip(&dropdown.items)
        .enumerate()
    {
        let style = if i == selected { select_style() } else { base };
        let mark = if item.checked == Some(true) {
            '•'
        } else {
            ' '
        };
        let mut spans = vec![Span::styled(format!(" {mark} "), style)];
        spans.extend(keyed(&item.label, item.key, style, style.fg(chrome.hotkey)));
        let pad = dropdown.label_w - item.label.chars().count();
        spans.push(Span::styled(" ".repeat(pad), style));
        if dropdown.shortcut_w > 0 {
            let shortcut = item.shortcut.unwrap_or("");
            let w = dropdown.shortcut_w;
            spans.push(Span::styled(format!("  {shortcut:>w$}"), style));
        }
        spans.push(Span::styled(" ", style));
        f.render_widget(
            Paragraph::new(Line::from(spans)).style(style),
            row.intersection(area),
        );
    }
    shadow(f.buffer_mut(), rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::PRESETS;
    use crate::game::tests::{game, play, white_game};
    use crate::menu::{self, Context};
    use crate::settings::Settings;
    use crate::theme::THEMES;
    use ratatui::{Terminal, backend::TestBackend};

    fn view() -> View<'static> {
        View {
            theme: &THEMES[0],
            spin: 0,
            // Past any move animation, so a render shows the position after the moves.
            now: Instant::now() + Duration::from_secs(1),
            thinking: None,
            engine_name: "built-in",
            level: Level::DEFAULT,
            browse: None,
            next_clock: None,
            notice: None,
            overlay: None,
            menus: menu::build(Context {
                settings: &Settings::default(),
                uci: false,
                engine_name: "built-in",
            }),
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

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 40,
    };

    fn middle(rect: Rect) -> Position {
        Position {
            x: rect.x + rect.width / 2,
            y: rect.y + rect.height / 2,
        }
    }

    #[test]
    fn renders_initial_position() {
        let mut g = white_game();
        g.cursor = (4, 1);
        g.confirm(now()); // select e2
        let content = render(&g, 100, 40);
        assert!(content.contains('♙'), "cells too small for art show glyphs");
        assert!(content.contains('●'), "legal target markers should appear");
        assert!(
            content.contains("─ Chess ─"),
            "title with a space on each side"
        );
        assert!(content.contains(" Game  Level  Clock  Engine  Theme  Help "));
        assert!(content.contains("F1 Help  F10 Menu"));
        assert!(content.contains("no warranty"));
        assert!(content.contains('┏'), "cursor corners should appear");
    }

    #[test]
    fn desktop_and_bars_use_theme_colours() {
        let buf = render_buf(&white_game(), 100, 40);
        let chrome = &THEMES[0].chrome;
        assert_eq!(buf[(0, 1)].bg, chrome.desktop);
        assert_eq!(buf[(0, 0)].bg, chrome.bar);
        assert_eq!(buf[(0, 39)].bg, chrome.bar);
        assert_eq!(buf[(2, 0)].fg, chrome.hotkey, "G of Game");
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
        let content = render(&g, 80, 29); // gives cell_h == 3 -> glyph fallback
        assert!(content.contains('♙'), "glyph fallback should render");
        assert!(content.contains('♛'));
        assert!(content.contains("no warranty"), "status bar fits");
        assert!(content.contains("5. c3"), "at least 5 move rows fit");
    }

    #[test]
    fn art_size_follows_the_terminal() {
        // d2, not e2: the cursor starts on e2 and draws its corners in the padding
        let d2_rows = |w, h| -> Vec<String> {
            let buf = render_buf(&white_game(), w, h);
            let cell = cell_area(&geo(w, h), Color::White, Square::D2);
            (cell.top()..cell.bottom())
                .map(|y| {
                    (cell.left()..cell.right())
                        .map(|x| buf[(x, y)].symbol())
                        .collect()
                })
                .collect()
        };
        assert!(d2_rows(100, 40).concat().contains('♙'), "8x4 cells: glyphs");
        for (w, h, size) in [(130, 56, ArtSize::Small), (150, 72, ArtSize::Large)] {
            let rows = d2_rows(w, h);
            let text = rows.concat();
            for row in art::piece_art(Piece::Pawn, size) {
                assert!(text.contains(row.trim()), "{size:?} at {w}x{h}: {row:?}");
            }
            let blank = |s: &str| s.chars().all(|c| c == ' ');
            assert!(blank(&rows[0]) && blank(&rows[rows.len() - 1]), "{size:?}");
            for row in &rows {
                assert!(
                    row.starts_with(' ') && row.ends_with(' '),
                    "{size:?}: {row:?}"
                );
            }
        }
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
        assert!(content.contains("♘ f3"), "figurine notation");
        assert!(content.contains("Material"));
        assert!(
            content.contains("─ Moves ─"),
            "title with a space on each side"
        );
        assert!(content.contains("─ Evaluation ─"));
        assert!(content.contains("You White · AI Black"));
        assert!(content.contains("built-in · Level 4"), "menu bar");
    }

    #[test]
    fn black_to_move_start_numbers_moves() {
        let mut g = game(None, Some("4k3/8/8/8/8/8/4P3/4K3 b - - 0 12"));
        play(&mut g, &[((4, 7), (3, 7))]); // Kd8
        let content = render(&g, 100, 40);
        assert!(content.contains(" 12. …"), "{content}");
        assert!(content.contains("White vs Black"));
        assert!(!content.contains("Level 4"), "no level in two-player mode");
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
        assert_eq!(buf[(cell.x + 1, cell.y + 1)].bg, THEMES[0].board.capture);
    }

    #[test]
    fn hint_is_tinted() {
        let mut g = white_game();
        assert!(g.set_hint(ChessMove::new(Square::E2, Square::E4, None)));
        let buf = render_buf(&g, 100, 40);
        let cell = cell_area(&geo(100, 40), Color::White, Square::E4);
        assert_eq!(buf[(cell.x + 1, cell.y + 1)].bg, THEMES[0].board.hint);
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
        assert!(content.contains("e4 e5 ♘ f3"));
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
    fn notice_replaces_the_status_bar_hints() {
        let mut v = view();
        v.notice = Some("Level 5/8");
        let content = text(&render_buf_with(&white_game(), &v, 100, 40));
        assert!(content.contains("Level 5/8"));
        assert!(!content.contains("F10 Menu"));
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
        assert_eq!(buf[(e2.x + 1, e2.y + 1)].bg, THEMES[0].board.light);
        assert!(!text(&buf).contains('┏'), "no cursor while browsing");
    }

    #[test]
    fn figurines_are_spaced_from_what_follows() {
        assert_eq!(figurine("Nbd2", Color::White), "♘ bd2");
        assert_eq!(figurine("Qxh7#", Color::Black), "♛ xh7#");
        assert_eq!(figurine("e8=Q+", Color::White), "e8=♕ +");
        assert_eq!(figurine("e8=Q", Color::White), "e8=♕", "no trailing space");
        assert_eq!(figurine("O-O", Color::White), "O-O");
        // The longest moves still leave a gap in the 9-wide move list column.
        assert_eq!(figurine("Qa1xh8#", Color::White).chars().count(), 8);
        assert_eq!(figurine("exd8=Q+", Color::White).chars().count(), 8);
    }

    fn count_in(buf: &Buffer, rect: Rect, symbol: &str) -> usize {
        rect.positions()
            .filter(|&pos| buf[pos].symbol() == symbol)
            .count()
    }

    #[test]
    fn moved_piece_slides_to_its_square() {
        let mut g = white_game();
        play(&mut g, &[((4, 1), (4, 3))]); // e4
        let moved_at = g.moved_at.expect("a move was played");
        let duration = slide_duration(g.history[0].mv);
        let geo = geo(100, 40);
        assert!(geo.art.is_none(), "glyph mode at this size");
        let (e2, e4) = (
            cell_area(&geo, Color::White, Square::E2),
            cell_area(&geo, Color::White, Square::E4),
        );

        let mut v = view();
        v.now = moved_at + duration / 2;
        assert!(animating(&g, v.now));
        let buf = render_buf_with(&g, &v, 100, 40);
        assert_eq!(count_in(&buf, e2, "♙"), 0, "left the source");
        assert_eq!(count_in(&buf, e4, "♙"), 0, "not arrived yet");
        assert_eq!(
            count_in(&buf, geo.board, "♙"),
            8,
            "seven at home, one on the way"
        );

        v.now = moved_at + duration;
        assert!(!animating(&g, v.now));
        let end = render_buf_with(&g, &v, 100, 40);
        assert_eq!(count_in(&end, e4, "♙"), 1);
        g.moved_at = None;
        assert_eq!(
            end,
            render_buf_with(&g, &v, 100, 40),
            "ends on the plain position"
        );
    }

    #[test]
    fn castling_slides_king_and_rook() {
        let board: Board = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1".parse().unwrap();
        let slide = |piece, from, to| Slide {
            piece,
            color: Color::White,
            from,
            to,
        };
        assert_eq!(
            slides(&board, ChessMove::new(Square::E1, Square::G1, None)),
            [
                slide(Piece::King, Square::E1, Square::G1),
                slide(Piece::Rook, Square::H1, Square::F1)
            ]
        );
        assert_eq!(
            slides(&board, ChessMove::new(Square::E1, Square::C1, None)),
            [
                slide(Piece::King, Square::E1, Square::C1),
                slide(Piece::Rook, Square::A1, Square::D1)
            ]
        );
        assert_eq!(
            slides(&board, ChessMove::new(Square::E1, Square::F1, None)),
            [slide(Piece::King, Square::E1, Square::F1)]
        );
    }

    #[test]
    fn slide_runs_from_source_to_destination_in_half_characters() {
        let slide = Slide {
            piece: Piece::Knight,
            color: Color::Black,
            from: Square::G8,
            to: Square::F6,
        };
        for (w, h) in [(100, 40), (130, 56), (150, 72)] {
            let geo = geo(w, h);
            let still = match geo.art {
                Some(size) => art::piece_art(Piece::Knight, size),
                None => vec!["♞".to_string()],
            };
            for orientation in [Color::White, Color::Black] {
                let at = |sq| piece_origin(&geo, orientation, sq);
                let sprite = |progress| sprite(&geo, orientation, &slide, progress);
                assert_eq!(sprite(0.0), (still.clone(), at(Square::G8)), "{w}x{h}");
                assert_eq!(sprite(1.0), (still.clone(), at(Square::F6)), "{w}x{h}");

                // The origin is where a standing piece is drawn.
                let mut g = white_game();
                g.orientation = orientation;
                let buf = render_buf(&g, w, h);
                let origin = at(Square::G8);
                for (dy, row) in (0..).zip(&still) {
                    for (dx, c) in (0..).zip(row.chars()) {
                        let cell = &buf[(origin.x + dx, origin.y + dy)];
                        assert_eq!(cell.symbol(), c.to_string(), "{w}x{h} {orientation:?}");
                    }
                }
            }
            if let Some(size) = geo.art {
                // One pixel down a file: the art is shifted by half a character.
                let down = Slide {
                    to: Square::G7,
                    ..slide
                };
                let step = 1.0 / f32::from(2 * geo.cell_h);
                let (rows, at) = sprite(&geo, Color::White, &down, step);
                assert_eq!(rows.len(), size.size().height as usize + 1, "{w}x{h}");
                assert_eq!(at, piece_origin(&geo, Color::White, Square::G8));
            }
        }
    }

    #[test]
    fn promotion_popup_hit_test() {
        let (_, cells) = promotion_layout(&geo(100, 40));
        for (piece, rect) in cells {
            let click = Position {
                x: rect.x + 2,
                y: rect.y,
            };
            assert_eq!(promotion_choice_at(AREA, click), Some(piece));
        }
        assert_eq!(promotion_choice_at(AREA, Position { x: 0, y: 0 }), None);

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
        v.overlay = Some(Overlay::Help);
        let content = text(&render_buf_with(&g, &v, 100, 40));
        assert!(content.contains("suggest a move"));
        assert!(content.contains("F10 / Alt+key"));
    }

    #[test]
    fn about_shows_engine_and_settings_path() {
        let mut v = view();
        v.overlay = Some(Overlay::About);
        v.settings_path = Some("C:/cfg/settings.txt");
        let content = text(&render_buf_with(&white_game(), &v, 100, 40));
        assert!(content.contains("by Saman Sedighi Rad"));
        assert!(content.contains("Engine: built-in"));
        assert!(content.contains("Settings: C:/cfg/settings.txt"));
    }

    #[test]
    fn open_menu_shows_items_and_marks_the_choice() {
        let mut v = view();
        v.overlay = Some(Overlay::Menu { menu: 4, item: 1 });
        let buf = render_buf_with(&white_game(), &v, 100, 40);
        let content = text(&buf);
        assert!(content.contains(" • Blue "));
        assert!(content.contains("   Black "));
        let dropdown = dropdown_layout(AREA, &v.menus, 4).unwrap();
        let black = dropdown.items[1];
        assert_eq!(buf[(black.x + 3, black.y)].bg, theme::SELECT_BG);

        v.overlay = Some(Overlay::Menu { menu: 0, item: 0 });
        let content = text(&render_buf_with(&white_game(), &v, 100, 40));
        assert!(content.contains("Resign"));
        assert!(content.contains('├'), "separator");
    }

    #[test]
    fn menu_hit_tests_match_the_layout() {
        let menus = view().menus;
        for (i, rect) in menu_title_rects(AREA, &menus).into_iter().enumerate() {
            assert_eq!(menu_title_at(AREA, &menus, middle(rect)), Some(i));
            let dropdown = dropdown_layout(AREA, &menus, i).unwrap();
            for (j, &row) in dropdown.items.iter().enumerate() {
                assert_eq!(menu_item_at(AREA, &menus, i, middle(row)), MenuHit::Item(j));
            }
            let corner = Position {
                x: dropdown.rect.x,
                y: dropdown.rect.y,
            };
            assert_eq!(menu_item_at(AREA, &menus, i, corner), MenuHit::Inside);
        }
        let game = dropdown_layout(AREA, &menus, 0).unwrap();
        let separator = Position {
            x: game.rect.x + 2,
            y: game.separators[0],
        };
        assert_eq!(menu_item_at(AREA, &menus, 0, separator), MenuHit::Inside);
        let far = Position { x: 90, y: 30 };
        assert_eq!(menu_item_at(AREA, &menus, 0, far), MenuHit::Outside);
        assert_eq!(menu_title_at(AREA, &menus, far), None);
    }

    #[test]
    fn confirm_dialog_and_buttons() {
        let mut v = view();
        v.overlay = Some(Overlay::Confirm {
            question: "Resign this game?",
            yes: false,
        });
        let buf = render_buf_with(&white_game(), &v, 100, 40);
        let content = text(&buf);
        assert!(content.contains("Resign this game?"));
        assert!(content.contains("< Yes >"));
        let layout = confirm_layout(AREA);
        assert_eq!(buf[(layout.no.x, layout.no.y)].bg, theme::SELECT_BG);
        assert_eq!(confirm_button_at(AREA, middle(layout.yes)), Some(true));
        assert_eq!(confirm_button_at(AREA, middle(layout.no)), Some(false));
        assert_eq!(confirm_button_at(AREA, middle(layout.dialog)), None);
    }

    #[test]
    fn click_maps_to_square_in_both_orientations() {
        let geo = geo(100, 40);
        for orientation in [Color::White, Color::Black] {
            for sq in [Square::E2, Square::A1, Square::H8] {
                let cell = cell_area(&geo, orientation, sq);
                assert_eq!(square_at(AREA, orientation, middle(cell)), Some(sq));
            }
        }
        assert_eq!(square_at(AREA, Color::White, Position { x: 0, y: 0 }), None);
    }
}
