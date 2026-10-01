use crate::board::{
    BLACK_KINGSIDE, BLACK_QUEENSIDE, Board, Color, Piece, PieceKind, WHITE_KINGSIDE,
    WHITE_QUEENSIDE, file_of, rank_of, square_index, square_name,
};

const KNIGHT_JUMPS: [(i32, i32); 8] = [
    (-2, -1),
    (-2, 1),
    (-1, -2),
    (-1, 2),
    (1, -2),
    (1, 2),
    (2, -1),
    (2, 1),
];
const KING_STEPS: [(i32, i32); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];
const ROOK_DIRECTIONS: [(i32, i32); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];
const BISHOP_DIRECTIONS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const PROMOTION_KINDS: [PieceKind; 4] = [
    PieceKind::Queen,
    PieceKind::Rook,
    PieceKind::Bishop,
    PieceKind::Knight,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayError {
    Malformed,
    Illegal,
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            PlayError::Malformed => "not a coordinate move",
            PlayError::Illegal => "not a legal move",
        })
    }
}

impl std::error::Error for PlayError {}

const SQUARE_MASK: u16 = 0b11_1111;
const TARGET_SHIFT: u32 = 6;
const PROMOTION_SHIFT: u32 = 12;
const PROMOTION_MASK: u16 = 0b111;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Move(u16);

impl Move {
    pub const NONE: Move = Move(0);

    pub fn new(origin: usize, target: usize, promotion: Option<PieceKind>) -> Self {
        debug_assert!(origin < 64 && target < 64);
        let promoted = match promotion {
            Some(kind) => kind as u16,
            None => 0,
        };
        Move(origin as u16 | (target as u16) << TARGET_SHIFT | promoted << PROMOTION_SHIFT)
    }

    pub fn origin(self) -> usize {
        (self.0 & SQUARE_MASK) as usize
    }

    pub fn target(self) -> usize {
        (self.0 >> TARGET_SHIFT & SQUARE_MASK) as usize
    }

    pub fn promotion(self) -> Option<PieceKind> {
        match self.0 >> PROMOTION_SHIFT & PROMOTION_MASK {
            1 => Some(PieceKind::Knight),
            2 => Some(PieceKind::Bishop),
            3 => Some(PieceKind::Rook),
            4 => Some(PieceKind::Queen),
            _ => None,
        }
    }

    pub fn uci(&self) -> String {
        let mut text = format!(
            "{}{}",
            square_name(self.origin()),
            square_name(self.target())
        );
        if let Some(kind) = self.promotion() {
            text.push(kind.san_letter().to_ascii_lowercase());
        }
        text
    }

    pub fn from_uci(uci: &str) -> Option<Self> {
        let origin = square_index(uci.get(0..2)?)?;
        let target = square_index(uci.get(2..4)?)?;
        let promotion = uci
            .chars()
            .nth(4)
            .and_then(PieceKind::from_promotion_letter);
        let well_formed = uci.len() == 4 || (uci.len() == 5 && promotion.is_some());
        well_formed.then(|| Move::new(origin, target, promotion))
    }
}

impl std::fmt::Debug for Move {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.uci())
    }
}

fn shifted(square: usize, file_step: i32, rank_step: i32) -> Option<usize> {
    let file = file_of(square) as i32 + file_step;
    let rank = rank_of(square) as i32 + rank_step;
    ((0..8).contains(&file) && (0..8).contains(&rank)).then(|| (rank * 8 + file) as usize)
}

impl Board {
    pub fn legal_moves(&self) -> Vec<Move> {
        self.legal_among(self.pseudo_legal_moves())
    }

    pub fn legal_captures(&self) -> Vec<Move> {
        let tactical = self
            .pseudo_legal_piece_moves()
            .into_iter()
            .filter(|candidate| candidate.promotion().is_some() || self.is_capture(*candidate))
            .collect();
        self.legal_among(tactical)
    }

    pub fn is_capture(&self, mv: Move) -> bool {
        let target = self.piece_at(mv.target());
        if target != Piece::Empty {
            return true;
        }
        self.is_en_passant(mv)
    }

    fn is_en_passant(&self, mv: Move) -> bool {
        let mover = self.piece_at(mv.origin());
        self.en_passant == Some(mv.target() as u8)
            && mover != Piece::Empty
            && mover.kind() == PieceKind::Pawn
            && file_of(mv.origin()) != file_of(mv.target())
    }

    fn legal_among(&self, mut candidates: Vec<Move>) -> Vec<Move> {
        if candidates.is_empty() {
            return candidates;
        }
        let mover = self.side_to_move();
        let king = self.king_square(mover);
        let exposed = self.pinned_squares(mover, king) | 1 << king;
        let evading = self.is_attacked(king, mover.opposite());

        let mut scratch = self.clone();
        candidates.retain(|candidate| {
            let settled = !evading
                && exposed & 1 << candidate.origin() == 0
                && !self.is_en_passant(*candidate);
            if settled {
                return true;
            }
            let undo = scratch.make_move_without_key(*candidate);
            let legal = !scratch.in_check(mover);
            scratch.unmake_move(*candidate, undo);
            legal
        });
        candidates
    }

    fn pinned_squares(&self, mover: Color, king: usize) -> u64 {
        let mut pinned = 0;
        for (directions, diagonal) in [(&ROOK_DIRECTIONS, false), (&BISHOP_DIRECTIONS, true)] {
            for (file_step, rank_step) in directions {
                let slider = if diagonal {
                    PieceKind::Bishop
                } else {
                    PieceKind::Rook
                };
                if let Some(square) = self.pinned_along(mover, king, *file_step, *rank_step, slider)
                {
                    pinned |= 1 << square;
                }
            }
        }
        pinned
    }

    fn pinned_along(
        &self,
        mover: Color,
        king: usize,
        file_step: i32,
        rank_step: i32,
        slider: PieceKind,
    ) -> Option<usize> {
        let mut current = king;
        let mut blocker = None;
        while let Some(next) = shifted(current, file_step, rank_step) {
            current = next;
            let piece = self.piece_at(next);
            if piece == Piece::Empty {
                continue;
            }
            match blocker {
                None if piece.color() == mover => blocker = Some(next),
                None => return None,
                Some(square) => {
                    let pins = piece.color() != mover
                        && (piece.kind() == slider || piece.kind() == PieceKind::Queen);
                    return pins.then_some(square);
                }
            }
        }
        None
    }

    pub fn apply_uci_move(&mut self, uci: &str) -> Result<(), PlayError> {
        let mv = Move::from_uci(uci).ok_or(PlayError::Malformed)?;
        self.play_move(mv)
    }

    pub fn play_move(&mut self, mv: Move) -> Result<(), PlayError> {
        if !self.legal_moves().contains(&mv) {
            return Err(PlayError::Illegal);
        }
        self.play_move_unchecked(mv);
        Ok(())
    }

    pub fn play_move_unchecked(&mut self, mv: Move) {
        self.make_move(mv);
    }

    pub fn in_check(&self, color: Color) -> bool {
        self.is_attacked(self.king_square(color), color.opposite())
    }

    fn king_square(&self, color: Color) -> usize {
        let square = self.kings[color as usize] as usize;
        debug_assert_eq!(
            self.piece_at(square),
            Piece::new(color, PieceKind::King),
            "the tracked king square went stale"
        );
        square
    }

    fn is_attacked(&self, square: usize, by: Color) -> bool {
        let pawn_origin_step = match by {
            Color::White => -1,
            Color::Black => 1,
        };
        for file_step in [-1, 1] {
            if let Some(origin) = shifted(square, file_step, pawn_origin_step)
                && self.piece_at(origin) == Piece::new(by, PieceKind::Pawn)
            {
                return true;
            }
        }

        for (steps, kind) in [
            (&KNIGHT_JUMPS, PieceKind::Knight),
            (&KING_STEPS, PieceKind::King),
        ] {
            for (file_step, rank_step) in steps {
                if let Some(origin) = shifted(square, *file_step, *rank_step)
                    && self.piece_at(origin) == Piece::new(by, kind)
                {
                    return true;
                }
            }
        }

        for (directions, diagonal) in [(&ROOK_DIRECTIONS, false), (&BISHOP_DIRECTIONS, true)] {
            for (file_step, rank_step) in directions {
                let blocker = self.first_piece_along(square, *file_step, *rank_step);
                let slider = if diagonal {
                    PieceKind::Bishop
                } else {
                    PieceKind::Rook
                };
                if blocker == Piece::new(by, slider) || blocker == Piece::new(by, PieceKind::Queen)
                {
                    return true;
                }
            }
        }
        false
    }

    fn first_piece_along(&self, from: usize, file_step: i32, rank_step: i32) -> Piece {
        let mut current = from;
        while let Some(next) = shifted(current, file_step, rank_step) {
            let piece = self.piece_at(next);
            if piece != Piece::Empty {
                return piece;
            }
            current = next;
        }
        Piece::Empty
    }

    fn pseudo_legal_moves(&self) -> Vec<Move> {
        let mut moves = self.pseudo_legal_piece_moves();
        self.castling_moves(self.side_to_move(), &mut moves);
        moves
    }

    fn pseudo_legal_piece_moves(&self) -> Vec<Move> {
        let mover = self.side_to_move();
        let mut moves = Vec::with_capacity(64);
        for from in 0..64 {
            let piece = self.piece_at(from);
            if piece == Piece::Empty || piece.color() != mover {
                continue;
            }
            match piece.kind() {
                PieceKind::Pawn => self.pawn_moves(from, mover, &mut moves),
                PieceKind::Knight => self.leaper_moves(from, mover, &KNIGHT_JUMPS, &mut moves),
                PieceKind::King => self.leaper_moves(from, mover, &KING_STEPS, &mut moves),
                PieceKind::Bishop => self.slider_moves(from, mover, &BISHOP_DIRECTIONS, &mut moves),
                PieceKind::Rook => self.slider_moves(from, mover, &ROOK_DIRECTIONS, &mut moves),
                PieceKind::Queen => {
                    self.slider_moves(from, mover, &ROOK_DIRECTIONS, &mut moves);
                    self.slider_moves(from, mover, &BISHOP_DIRECTIONS, &mut moves);
                }
            }
        }
        moves
    }

    fn pawn_moves(&self, from: usize, mover: Color, moves: &mut Vec<Move>) {
        let (rank_step, start_rank) = match mover {
            Color::White => (1, 1),
            Color::Black => (-1, 6),
        };

        if let Some(one_ahead) = shifted(from, 0, rank_step)
            && self.piece_at(one_ahead) == Piece::Empty
        {
            push_pawn_move(from, one_ahead, moves);
            if rank_of(from) == start_rank
                && let Some(two_ahead) = shifted(from, 0, 2 * rank_step)
                && self.piece_at(two_ahead) == Piece::Empty
            {
                moves.push(Move::new(from, two_ahead, None));
            }
        }

        for file_step in [-1, 1] {
            if let Some(target) = shifted(from, file_step, rank_step) {
                let occupant = self.piece_at(target);
                let holds_enemy = occupant != Piece::Empty && occupant.color() != mover;
                if holds_enemy || self.en_passant == Some(target as u8) {
                    push_pawn_move(from, target, moves);
                }
            }
        }
    }

    fn leaper_moves(&self, from: usize, mover: Color, steps: &[(i32, i32)], moves: &mut Vec<Move>) {
        for (file_step, rank_step) in steps {
            if let Some(to) = shifted(from, *file_step, *rank_step) {
                let occupant = self.piece_at(to);
                if occupant == Piece::Empty || occupant.color() != mover {
                    moves.push(Move::new(from, to, None));
                }
            }
        }
    }

    fn slider_moves(
        &self,
        from: usize,
        mover: Color,
        directions: &[(i32, i32)],
        moves: &mut Vec<Move>,
    ) {
        for (file_step, rank_step) in directions {
            let mut current = from;
            while let Some(to) = shifted(current, *file_step, *rank_step) {
                let occupant = self.piece_at(to);
                if occupant == Piece::Empty {
                    moves.push(Move::new(from, to, None));
                    current = to;
                    continue;
                }
                if occupant.color() != mover {
                    moves.push(Move::new(from, to, None));
                }
                break;
            }
        }
    }

    fn castling_moves(&self, mover: Color, moves: &mut Vec<Move>) {
        let (kingside, queenside, king, opponent) = match mover {
            Color::White => (WHITE_KINGSIDE, WHITE_QUEENSIDE, 4, Color::Black),
            Color::Black => (BLACK_KINGSIDE, BLACK_QUEENSIDE, 60, Color::White),
        };
        let own = |kind| Piece::new(mover, kind);
        if self.piece_at(king) != own(PieceKind::King) || self.is_attacked(king, opponent) {
            return;
        }

        if self.castling & kingside != 0
            && self.piece_at(king + 1) == Piece::Empty
            && self.piece_at(king + 2) == Piece::Empty
            && self.piece_at(king + 3) == own(PieceKind::Rook)
            && !self.is_attacked(king + 1, opponent)
        {
            moves.push(Move::new(king, king + 2, None));
        }

        if self.castling & queenside != 0
            && self.piece_at(king - 1) == Piece::Empty
            && self.piece_at(king - 2) == Piece::Empty
            && self.piece_at(king - 3) == Piece::Empty
            && self.piece_at(king - 4) == own(PieceKind::Rook)
            && !self.is_attacked(king - 1, opponent)
        {
            moves.push(Move::new(king, king - 2, None));
        }
    }
}

fn push_pawn_move(from: usize, to: usize, moves: &mut Vec<Move>) {
    if rank_of(to) == 0 || rank_of(to) == 7 {
        for kind in PROMOTION_KINDS {
            moves.push(Move::new(from, to, Some(kind)));
        }
    } else {
        moves.push(Move::new(from, to, None));
    }
}

pub fn perft(board: &Board, depth: u32) -> u64 {
    let mut walked = board.clone();
    perft_on(&mut walked, depth)
}

fn perft_on(board: &mut Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = board.legal_moves();
    if depth == 1 {
        return moves.len() as u64;
    }
    moves
        .iter()
        .map(|mv| {
            #[cfg(debug_assertions)]
            let before = board.clone();
            let undo = board.make_move(*mv);
            let count = perft_on(board, depth - 1);
            board.unmake_move(*mv, undo);
            #[cfg(debug_assertions)]
            debug_assert_eq!(*board, before, "{} was not undone", mv.uci());
            count
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{Move, PlayError, perft};
    use crate::board::tests::REFERENCE_FENS;
    use crate::board::{Board, PieceKind, STARTPOS_FEN};

    const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    const POSITION_3: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
    const POSITION_4: &str = "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1";
    const POSITION_5: &str = "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8";

    fn assert_counts(cases: &[(&str, &[u64])]) {
        for (fen, expected) in cases {
            let board = Board::from_fen(fen).unwrap();
            for (depth, nodes) in expected.iter().enumerate() {
                assert_eq!(
                    perft(&board, depth as u32 + 1),
                    *nodes,
                    "perft({}) of {fen}",
                    depth + 1
                );
            }
        }
    }

    #[test]
    fn perft_matches_the_published_counts() {
        assert_counts(&[
            (STARTPOS_FEN, &[20, 400, 8_902, 197_281, 4_865_609]),
            (KIWIPETE, &[48, 2_039, 97_862, 4_085_603]),
            (POSITION_3, &[14, 191, 2_812, 43_238, 674_624]),
            (POSITION_4, &[6, 264, 9_467, 422_333]),
            (POSITION_5, &[44, 1_486, 62_379, 2_103_487]),
        ]);
    }

    #[test]
    fn every_square_pair_survives_the_sixteen_bits() {
        assert_eq!(std::mem::size_of::<Move>(), 2);
        for origin in 0..64 {
            for target in 0..64 {
                let mv = Move::new(origin, target, None);
                assert_eq!(mv.origin(), origin);
                assert_eq!(mv.target(), target);
                assert_eq!(mv.promotion(), None);
            }
        }
    }

    #[test]
    fn every_promotion_survives_the_sixteen_bits() {
        for kind in [
            PieceKind::Knight,
            PieceKind::Bishop,
            PieceKind::Rook,
            PieceKind::Queen,
        ] {
            let mv = Move::new(8, 0, Some(kind));
            assert_eq!(mv.origin(), 8);
            assert_eq!(mv.target(), 0);
            assert_eq!(mv.promotion(), Some(kind));
        }
    }

    #[test]
    fn the_absent_move_is_one_no_generator_can_produce() {
        assert_eq!(Move::NONE.origin(), Move::NONE.target());
        for fen in REFERENCE_FENS {
            let board = Board::from_fen(fen).unwrap();
            assert!(
                !board.legal_moves().contains(&Move::NONE),
                "{fen} generated the sentinel"
            );
        }
    }

    #[test]
    fn a_move_is_played_only_when_it_is_well_formed_and_legal() {
        let cases = [
            (STARTPOS_FEN, "e2e4", Ok(()), "an ordinary opening move"),
            (
                STARTPOS_FEN,
                "e7e5",
                Err(PlayError::Illegal),
                "black is not to move",
            ),
            (
                STARTPOS_FEN,
                "e2e5",
                Err(PlayError::Illegal),
                "a pawn cannot go three squares",
            ),
            (
                STARTPOS_FEN,
                "a1a3",
                Err(PlayError::Illegal),
                "the rook is boxed in",
            ),
            (
                STARTPOS_FEN,
                "",
                Err(PlayError::Malformed),
                "nothing to read",
            ),
            (
                STARTPOS_FEN,
                "e2",
                Err(PlayError::Malformed),
                "no target square",
            ),
            (
                STARTPOS_FEN,
                "e2e4x",
                Err(PlayError::Malformed),
                "no such promotion",
            ),
            (
                STARTPOS_FEN,
                "Nf3",
                Err(PlayError::Malformed),
                "algebraic, not uci",
            ),
            (
                "8/8/8/8/k2Pp2Q/8/8/3K4 b - d3 0 1",
                "e4d3",
                Err(PlayError::Illegal),
                "taking on d3 removes both pawns from the rank and hands the queen the king",
            ),
            (
                "8/8/8/8/k2Pp3/8/8/3K4 b - d3 0 1",
                "e4d3",
                Ok(()),
                "with no queen on the rank the same capture is ordinary",
            ),
            (
                "7k/8/8/8/8/8/r7/K7 w - - 0 1",
                "a1a2",
                Ok(()),
                "the king takes the undefended rook",
            ),
            (
                "7k/8/8/8/8/8/8/KR5r w - - 0 1",
                "b1b8",
                Err(PlayError::Illegal),
                "the rook is pinned to the king",
            ),
        ];
        for (fen, uci, expected, reason) in cases {
            let mut board = Board::from_fen(fen).unwrap();
            let before = board.clone();
            assert_eq!(
                board.legal_moves().iter().any(|mv| mv.uci() == uci),
                expected.is_ok(),
                "fen {fen:?}, move {uci:?}: {reason}"
            );
            assert_eq!(
                board.apply_uci_move(uci),
                expected,
                "fen {fen:?}, move {uci:?}: {reason}"
            );
            match expected {
                Ok(()) => assert_eq!(
                    board.side_to_move(),
                    before.side_to_move().opposite(),
                    "fen {fen:?}, move {uci:?}: the turn did not pass"
                ),
                Err(_) => assert_eq!(
                    board, before,
                    "fen {fen:?}, move {uci:?}: a refused move changed the board"
                ),
            }
        }
    }

    #[test]
    fn a_pinned_piece_may_only_move_along_the_pin() {
        let cases = [
            (
                "7k/8/8/8/4q3/8/4B3/R3K3 w - - 0 1",
                "e2",
                false,
                "the bishop shields the king from the queen's file and every square it reaches \
                 leaves that file",
            ),
            (
                "7k/8/8/8/4q3/8/4B3/R3K3 w - - 0 1",
                "a1",
                true,
                "the rook is on the other side of the king and is not pinned by anything",
            ),
        ];
        for (fen, origin, can_move, reason) in cases {
            let board = Board::from_fen(fen).unwrap();
            let moves: Vec<String> = board
                .legal_moves()
                .iter()
                .map(Move::uci)
                .filter(|uci| uci.starts_with(origin))
                .collect();
            assert_eq!(
                !moves.is_empty(),
                can_move,
                "fen {fen:?}, from {origin}: {reason}, got {moves:?}"
            );
        }
    }

    #[test]
    fn the_tactical_list_is_the_legal_list_without_the_quiet_moves() {
        for fen in REFERENCE_FENS {
            let board = Board::from_fen(fen).unwrap();
            let expected = named(
                board
                    .legal_moves()
                    .into_iter()
                    .filter(|mv| mv.promotion().is_some() || board.is_capture(*mv))
                    .collect(),
            );
            assert_eq!(named(board.legal_captures()), expected, "fen {fen:?}");
        }
    }

    #[test]
    fn the_tactical_list_holds_exactly_the_moves_that_change_material() {
        let cases: [(&str, &[&str], &str); 3] = [
            (
                "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3",
                &["e5d6"],
                "an en passant capture lands on an empty square",
            ),
            (
                "7k/P7/8/8/8/8/8/7K w - - 0 1",
                &["a7a8b", "a7a8n", "a7a8q", "a7a8r"],
                "a promotion is tactical even when it captures nothing",
            ),
            (
                "7k/8/8/1n6/8/8/8/KR5q w - - 0 1",
                &["b1h1"],
                "taking the knight would leave the king alone on the rank with the queen",
            ),
        ];
        for (fen, expected, reason) in cases {
            let board = Board::from_fen(fen).unwrap();
            assert_eq!(
                named(board.legal_captures()),
                expected,
                "fen {fen:?}: {reason}"
            );
        }
    }

    fn named(moves: Vec<Move>) -> Vec<String> {
        let mut names: Vec<String> = moves.iter().map(Move::uci).collect();
        names.sort();
        names
    }
}
