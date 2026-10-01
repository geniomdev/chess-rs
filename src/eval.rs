use crate::board::{Board, Color, Piece, PieceKind};

const PAWN_PLACEMENT: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, //
    50, 50, 50, 50, 50, 50, 50, 50, //
    10, 10, 20, 30, 30, 20, 10, 10, //
    5, 5, 10, 25, 25, 10, 5, 5, //
    0, 0, 0, 20, 20, 0, 0, 0, //
    5, -5, -10, 0, 0, -10, -5, 5, //
    5, 10, 10, -20, -20, 10, 10, 5, //
    0, 0, 0, 0, 0, 0, 0, 0,
];

const KNIGHT_PLACEMENT: [i32; 64] = [
    -50, -40, -30, -30, -30, -30, -40, -50, //
    -40, -20, 0, 0, 0, 0, -20, -40, //
    -30, 0, 10, 15, 15, 10, 0, -30, //
    -30, 5, 15, 20, 20, 15, 5, -30, //
    -30, 0, 15, 20, 20, 15, 0, -30, //
    -30, 5, 10, 15, 15, 10, 5, -30, //
    -40, -20, 0, 5, 5, 0, -20, -40, //
    -50, -40, -30, -30, -30, -30, -40, -50,
];

const BISHOP_PLACEMENT: [i32; 64] = [
    -20, -10, -10, -10, -10, -10, -10, -20, //
    -10, 0, 0, 0, 0, 0, 0, -10, //
    -10, 0, 5, 10, 10, 5, 0, -10, //
    -10, 5, 5, 10, 10, 5, 5, -10, //
    -10, 0, 10, 10, 10, 10, 0, -10, //
    -10, 10, 10, 10, 10, 10, 10, -10, //
    -10, 5, 0, 0, 0, 0, 5, -10, //
    -20, -10, -10, -10, -10, -10, -10, -20,
];

const ROOK_PLACEMENT: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, //
    5, 10, 10, 10, 10, 10, 10, 5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    -5, 0, 0, 0, 0, 0, 0, -5, //
    0, 0, 0, 5, 5, 0, 0, 0,
];

const QUEEN_PLACEMENT: [i32; 64] = [
    -20, -10, -10, -5, -5, -10, -10, -20, //
    -10, 0, 0, 0, 0, 0, 0, -10, //
    -10, 0, 5, 5, 5, 5, 0, -10, //
    -5, 0, 5, 5, 5, 5, 0, -5, //
    0, 0, 5, 5, 5, 5, 0, -5, //
    -10, 5, 5, 5, 5, 5, 0, -10, //
    -10, 0, 5, 0, 0, 0, 0, -10, //
    -20, -10, -10, -5, -5, -10, -10, -20,
];

const KING_PLACEMENT: [i32; 64] = [
    -30, -40, -40, -50, -50, -40, -40, -30, //
    -30, -40, -40, -50, -50, -40, -40, -30, //
    -30, -40, -40, -50, -50, -40, -40, -30, //
    -30, -40, -40, -50, -50, -40, -40, -30, //
    -20, -30, -30, -40, -40, -30, -30, -20, //
    -10, -20, -20, -20, -20, -20, -20, -10, //
    20, 20, 0, 0, 0, 0, 20, 20, //
    20, 30, 10, 0, 0, 10, 30, 20,
];

pub fn evaluate(board: &Board) -> i32 {
    let mut score = 0;
    for square in 0..64 {
        let piece = board.piece_at(square);
        if piece == Piece::Empty {
            continue;
        }
        let placement_index = match piece.color() {
            Color::White => (7 - square / 8) * 8 + square % 8,
            Color::Black => square,
        };
        let value = piece_value(piece.kind()) + placement_table(piece.kind())[placement_index];
        score += match piece.color() {
            Color::White => value,
            Color::Black => -value,
        };
    }
    match board.side_to_move() {
        Color::White => score,
        Color::Black => -score,
    }
}

pub(crate) fn piece_value(kind: PieceKind) -> i32 {
    match kind {
        PieceKind::Pawn => 100,
        PieceKind::Knight => 320,
        PieceKind::Bishop => 330,
        PieceKind::Rook => 500,
        PieceKind::Queen => 900,
        PieceKind::King => 0,
    }
}

fn placement_table(kind: PieceKind) -> &'static [i32; 64] {
    match kind {
        PieceKind::Pawn => &PAWN_PLACEMENT,
        PieceKind::Knight => &KNIGHT_PLACEMENT,
        PieceKind::Bishop => &BISHOP_PLACEMENT,
        PieceKind::Rook => &ROOK_PLACEMENT,
        PieceKind::Queen => &QUEEN_PLACEMENT,
        PieceKind::King => &KING_PLACEMENT,
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate, piece_value, placement_table};
    use crate::board::{Board, PieceKind, STARTPOS_FEN};
    use std::cmp::Ordering;

    const KINDS: [PieceKind; 6] = [
        PieceKind::Pawn,
        PieceKind::Knight,
        PieceKind::Bishop,
        PieceKind::Rook,
        PieceKind::Queen,
        PieceKind::King,
    ];

    #[test]
    fn the_pieces_are_worth_what_the_books_say() {
        let cases = [
            (PieceKind::Pawn, 100, "a pawn is the unit"),
            (
                PieceKind::King,
                0,
                "the king is priceless and so counts for nothing",
            ),
        ];
        for (kind, value, reason) in cases {
            assert_eq!(piece_value(kind), value, "{kind:?}: {reason}");
        }

        let ascending = [
            PieceKind::Knight,
            PieceKind::Bishop,
            PieceKind::Rook,
            PieceKind::Queen,
        ];
        for pair in ascending.windows(2) {
            assert!(
                piece_value(pair[0]) < piece_value(pair[1]),
                "{:?} is not worth less than {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn every_kind_reads_a_table_of_its_own() {
        for (first, kind) in KINDS.iter().enumerate() {
            for other in KINDS.iter().skip(first + 1) {
                assert!(
                    !std::ptr::eq(placement_table(*kind), placement_table(*other)),
                    "{kind:?} and {other:?} share a placement table"
                );
            }
        }
    }

    #[test]
    fn the_score_leans_toward_the_side_with_more() {
        let cases = [
            (
                STARTPOS_FEN,
                Ordering::Equal,
                "the opening position is level",
            ),
            (
                "4k3/8/8/8/8/8/8/3QK3 w - - 0 1",
                Ordering::Greater,
                "white is a queen up and to move",
            ),
            (
                "4k3/8/8/8/8/8/8/3QK3 b - - 0 1",
                Ordering::Less,
                "black is to move a queen down",
            ),
            (
                "rnb1kbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                Ordering::Greater,
                "white has an extra queen",
            ),
        ];
        for (fen, expected, reason) in cases {
            let score = evaluate(&Board::from_fen(fen).unwrap());
            assert_eq!(
                score.cmp(&0),
                expected,
                "fen {fen:?} scored {score}: {reason}"
            );
        }
    }

    #[test]
    fn the_score_is_read_from_the_side_to_move() {
        let cases = [
            (
                "4k3/8/8/8/8/8/8/3QK3 w - - 0 1",
                "4k3/8/8/8/8/8/8/3QK3 b - - 0 1",
                -1,
                "the same position seen from the other side",
            ),
            (
                "4k3/8/8/8/8/5N2/8/4K3 w - - 0 1",
                "4k3/8/5n2/8/8/8/8/4K3 b - - 0 1",
                1,
                "a mirrored position is worth the same to the side that owns it",
            ),
        ];
        for (fen, other, sign, reason) in cases {
            let first = evaluate(&Board::from_fen(fen).unwrap());
            let second = evaluate(&Board::from_fen(other).unwrap());
            assert_eq!(second, sign * first, "{fen:?} against {other:?}: {reason}");
        }
    }
}
