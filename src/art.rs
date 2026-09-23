//! Piece art drawn as pixel bitmaps and rendered with quadrant block characters.
//!
//! Each character cell holds 2x2 pixels, so a bitmap is twice as wide and twice as tall as
//! the art it becomes. In a bitmap `#` is ink and `.` is blank. Bitmaps are cropped to their
//! ink; the space around a piece comes from `PADDING`.

use chess::Piece;
use ratatui::layout::Size;

/// Blank characters kept between the art and every edge of its board cell.
const PADDING: u16 = 1;

/// The art sizes, in characters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtSize {
    Small,
    Large,
}

impl ArtSize {
    pub fn size(self) -> Size {
        match self {
            ArtSize::Small => Size {
                width: 5,
                height: 4,
            },
            ArtSize::Large => Size {
                width: 9,
                height: 6,
            },
        }
    }

    /// The smallest board cell that holds this art with `PADDING` on every side.
    pub fn min_cell(self) -> Size {
        let Size { width, height } = self.size();
        Size {
            width: width + 2 * PADDING,
            height: height + 2 * PADDING,
        }
    }

    /// The largest art that fits a board cell with padding, if any.
    pub fn for_cell(cell: Size) -> Option<ArtSize> {
        [ArtSize::Large, ArtSize::Small].into_iter().find(|art| {
            let min = art.min_cell();
            cell.width >= min.width && cell.height >= min.height
        })
    }
}

pub fn piece_art(piece: Piece, size: ArtSize) -> Vec<String> {
    quadrants(bitmap(piece, size))
}

fn bitmap(piece: Piece, size: ArtSize) -> &'static [&'static str] {
    match (size, piece) {
        (ArtSize::Small, Piece::Pawn) => &SMALL_PAWN,
        (ArtSize::Small, Piece::Knight) => &SMALL_KNIGHT,
        (ArtSize::Small, Piece::Bishop) => &SMALL_BISHOP,
        (ArtSize::Small, Piece::Rook) => &SMALL_ROOK,
        (ArtSize::Small, Piece::Queen) => &SMALL_QUEEN,
        (ArtSize::Small, Piece::King) => &SMALL_KING,
        (ArtSize::Large, Piece::Pawn) => &LARGE_PAWN,
        (ArtSize::Large, Piece::Knight) => &LARGE_KNIGHT,
        (ArtSize::Large, Piece::Bishop) => &LARGE_BISHOP,
        (ArtSize::Large, Piece::Rook) => &LARGE_ROOK,
        (ArtSize::Large, Piece::Queen) => &LARGE_QUEEN,
        (ArtSize::Large, Piece::King) => &LARGE_KING,
    }
}

/// One character per 2x2 block of pixels.
fn quadrants(bitmap: &[&str]) -> Vec<String> {
    bitmap
        .chunks(2)
        .map(|pair| {
            let (top, bottom) = (pair[0].as_bytes(), pair[1].as_bytes());
            (0..top.len())
                .step_by(2)
                .map(|x| {
                    let ink = |row: &[u8], dx: usize| usize::from(row[x + dx] == b'#');
                    let index =
                        ink(top, 0) | ink(top, 1) << 1 | ink(bottom, 0) << 2 | ink(bottom, 1) << 3;
                    QUADRANTS[index]
                })
                .collect()
        })
        .collect()
}

/// Indexed by bits: 1 = top left, 2 = top right, 4 = bottom left, 8 = bottom right.
const QUADRANTS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];

// ----- small: 10x8 pixels, 5x4 characters -----------------------------------------------------

const SMALL_PAWN: [&str; 8] = [
    "..........",
    "....##....",
    "...####...",
    "...####...",
    "....##....",
    "...####...",
    "..######..",
    "##########",
];

const SMALL_KNIGHT: [&str; 8] = [
    "....##....",
    "...####...",
    "..######..",
    ".###.####.",
    "###.#####.",
    "....#####.",
    "...######.",
    "##########",
];

const SMALL_BISHOP: [&str; 8] = [
    "....##....",
    "...#..#...",
    "...##.#...",
    "...####...",
    "....##....",
    "...####...",
    "..######..",
    "##########",
];

const SMALL_ROOK: [&str; 8] = [
    "##..##..##",
    "##########",
    ".########.",
    "..######..",
    "..######..",
    "..######..",
    ".########.",
    "##########",
];

const SMALL_QUEEN: [&str; 8] = [
    "#...##...#",
    "##.####.##",
    "##########",
    ".########.",
    "..######..",
    "..######..",
    ".########.",
    "##########",
];

const SMALL_KING: [&str; 8] = [
    "....##....",
    "..######..",
    "....##....",
    ".########.",
    ".########.",
    "..######..",
    ".########.",
    "##########",
];

// ----- large: 18x12 pixels, 9x6 characters ----------------------------------------------------

const LARGE_PAWN: [&str; 12] = [
    "..................",
    "..................",
    "........##........",
    ".......####.......",
    ".......####.......",
    "........##........",
    "......######......",
    ".......####.......",
    "......######......",
    "....##########....",
    "...############...",
    "..##############..",
];

const LARGE_KNIGHT: [&str; 12] = [
    ".......##.##......",
    "......########....",
    "....###########...",
    "...####.########..",
    "..##############..",
    ".#######...#####..",
    ".####.....######..",
    ".........######...",
    "........#######...",
    ".......########...",
    "....##########....",
    "..##############..",
];

const LARGE_BISHOP: [&str; 12] = [
    "........##........",
    ".......####.......",
    "......####.#......",
    "......###.##......",
    "......##.###......",
    "......######......",
    ".......####.......",
    "......######......",
    ".......####.......",
    "......######......",
    "....##########....",
    "..##############..",
];

const LARGE_ROOK: [&str; 12] = [
    "..###..####..###..",
    "..###..####..###..",
    "..##############..",
    "...############...",
    "....##########....",
    "....##########....",
    "....##########....",
    "....##########....",
    "...############...",
    "..##############..",
    ".################.",
    ".################.",
];

const LARGE_QUEEN: [&str; 12] = [
    "#.......##.......#",
    ".#..#..####..#..#.",
    ".##.##.####.##.##.",
    ".################.",
    "..##############..",
    "...############...",
    "....##########....",
    "....##########....",
    "...############...",
    "..##############..",
    ".################.",
    ".################.",
];

const LARGE_KING: [&str; 12] = [
    "........##........",
    "......######......",
    "........##........",
    "..#####.##.#####..",
    ".######.##.######.",
    ".################.",
    "..##############..",
    "...############...",
    "....##########....",
    "...############...",
    ".################.",
    ".################.",
];

#[cfg(test)]
mod tests {
    use super::*;

    const PIECES: [Piece; 6] = [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
        Piece::King,
    ];
    const SIZES: [ArtSize; 2] = [ArtSize::Small, ArtSize::Large];

    fn is_palindrome(row: &str) -> bool {
        row.bytes().eq(row.bytes().rev())
    }

    #[test]
    fn bitmaps_have_exact_dimensions_and_only_ink_or_blank() {
        for size in SIZES {
            let Size { width, height } = size.size();
            for piece in PIECES {
                let bitmap = bitmap(piece, size);
                assert_eq!(bitmap.len(), 2 * height as usize, "{piece:?} {size:?}");
                for row in bitmap {
                    assert_eq!(row.len(), 2 * width as usize, "{piece:?} {size:?}: {row}");
                    assert!(row.bytes().all(|b| b == b'#' || b == b'.'), "{row}");
                }
                let art = piece_art(piece, size);
                assert_eq!(art.len(), height as usize);
                assert!(art.iter().all(|r| r.chars().count() == width as usize));
            }
        }
    }

    #[test]
    fn bitmaps_are_cropped_to_their_ink() {
        for size in SIZES {
            let bitmaps = PIECES.map(|piece| bitmap(piece, size));
            for (piece, bitmap) in PIECES.iter().zip(bitmaps) {
                let base = bitmap[bitmap.len() - 1];
                assert!(
                    base.contains('#'),
                    "{piece:?} {size:?} stands on the bottom row"
                );
            }
            let rows = || bitmaps.iter().flat_map(|b| b.iter());
            assert!(
                rows().any(|r| r.starts_with('#')),
                "{size:?}: blank first column"
            );
            assert!(
                rows().any(|r| r.ends_with('#')),
                "{size:?}: blank last column"
            );
        }
    }

    #[test]
    fn pieces_are_symmetric_where_they_should_be() {
        for size in SIZES {
            for piece in [Piece::Pawn, Piece::Rook, Piece::Queen, Piece::King] {
                assert!(
                    bitmap(piece, size).iter().all(|r| is_palindrome(r)),
                    "{piece:?}"
                );
            }
            let bishop = bitmap(Piece::Bishop, size);
            let bottom_half = &bishop[bishop.len() / 2..];
            assert!(bottom_half.iter().all(|r| is_palindrome(r)), "bishop");
            let knight = bitmap(Piece::Knight, size);
            assert!(is_palindrome(knight[knight.len() - 1]), "knight base");
        }
    }

    #[test]
    fn pieces_differ() {
        for size in SIZES {
            for (i, a) in PIECES.iter().enumerate() {
                for b in &PIECES[i + 1..] {
                    assert_ne!(piece_art(*a, size), piece_art(*b, size), "{a:?} {b:?}");
                }
            }
        }
    }

    #[test]
    fn quadrants_cover_every_pattern() {
        let cases = [
            ("..", "..", " "),
            ("#.", "..", "▘"),
            (".#", "..", "▝"),
            ("..", "#.", "▖"),
            ("..", ".#", "▗"),
            ("##", "..", "▀"),
            ("..", "##", "▄"),
            ("#.", "#.", "▌"),
            (".#", ".#", "▐"),
            ("#.", ".#", "▚"),
            (".#", "#.", "▞"),
            ("##", "#.", "▛"),
            ("##", ".#", "▜"),
            ("#.", "##", "▙"),
            (".#", "##", "▟"),
            ("##", "##", "█"),
        ];
        for (top, bottom, want) in cases {
            assert_eq!(quadrants(&[top, bottom]), [want], "{top}/{bottom}");
        }
    }

    #[test]
    fn largest_art_that_fits_the_cell_with_padding() {
        let fit = |width, height| ArtSize::for_cell(Size { width, height });
        assert_eq!(fit(6, 6), None);
        assert_eq!(fit(7, 5), None);
        assert_eq!(fit(7, 6), Some(ArtSize::Small));
        assert_eq!(fit(10, 8), Some(ArtSize::Small));
        assert_eq!(fit(11, 7), Some(ArtSize::Small));
        assert_eq!(fit(11, 8), Some(ArtSize::Large));
    }
}
