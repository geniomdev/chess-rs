use crate::movegen::Move;

pub const STARTPOS_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

const FILES: usize = 8;
const RANKS: usize = 8;
const NO_KING: u8 = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub fn opposite(self) -> Self {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl PieceKind {
    pub fn from_san_letter(letter: char) -> Option<Self> {
        match letter {
            'K' => Some(PieceKind::King),
            'Q' => Some(PieceKind::Queen),
            'R' => Some(PieceKind::Rook),
            'B' => Some(PieceKind::Bishop),
            'N' => Some(PieceKind::Knight),
            _ => None,
        }
    }

    pub fn from_promotion_letter(letter: char) -> Option<Self> {
        match Self::from_san_letter(letter.to_ascii_uppercase()) {
            Some(PieceKind::King) => None,
            kind => kind,
        }
    }

    pub fn san_letter(self) -> char {
        match self {
            PieceKind::Pawn => 'P',
            PieceKind::Knight => 'N',
            PieceKind::Bishop => 'B',
            PieceKind::Rook => 'R',
            PieceKind::Queen => 'Q',
            PieceKind::King => 'K',
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Piece {
    WhitePawn,
    WhiteKnight,
    WhiteBishop,
    WhiteRook,
    WhiteQueen,
    WhiteKing,
    BlackPawn,
    BlackKnight,
    BlackBishop,
    BlackRook,
    BlackQueen,
    BlackKing,
    Empty,
}

const PIECES: [[Piece; 6]; 2] = [
    [
        Piece::WhitePawn,
        Piece::WhiteKnight,
        Piece::WhiteBishop,
        Piece::WhiteRook,
        Piece::WhiteQueen,
        Piece::WhiteKing,
    ],
    [
        Piece::BlackPawn,
        Piece::BlackKnight,
        Piece::BlackBishop,
        Piece::BlackRook,
        Piece::BlackQueen,
        Piece::BlackKing,
    ],
];

const KINDS: [PieceKind; 6] = [
    PieceKind::Pawn,
    PieceKind::Knight,
    PieceKind::Bishop,
    PieceKind::Rook,
    PieceKind::Queen,
    PieceKind::King,
];

impl Piece {
    pub fn new(color: Color, kind: PieceKind) -> Self {
        PIECES[color as usize][kind as usize]
    }

    pub fn color(self) -> Color {
        debug_assert!(self != Piece::Empty);
        if (self as u8) < 6 {
            Color::White
        } else {
            Color::Black
        }
    }

    pub fn kind(self) -> PieceKind {
        debug_assert!(self != Piece::Empty);
        KINDS[self as usize % 6]
    }

    fn fen_letter(self) -> char {
        let letter = self.kind().san_letter();
        match self.color() {
            Color::White => letter,
            Color::Black => letter.to_ascii_lowercase(),
        }
    }

    fn from_fen_letter(letter: char) -> Option<Self> {
        let color = if letter.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        let kind = match letter.to_ascii_uppercase() {
            'P' => PieceKind::Pawn,
            other => PieceKind::from_san_letter(other)?,
        };
        Some(Piece::new(color, kind))
    }
}

pub fn square_index(name: &str) -> Option<usize> {
    let mut characters = name.chars();
    let file = characters.next()?;
    let rank = characters.next()?;
    if characters.next().is_some() || !file.is_ascii_lowercase() || !rank.is_ascii_digit() {
        return None;
    }
    let file = (file as usize).checked_sub('a' as usize)?;
    let rank = (rank as usize).checked_sub('1' as usize)?;
    (file < FILES && rank < RANKS).then_some(rank * FILES + file)
}

pub fn square_name(index: usize) -> String {
    format!("{}{}", file_char(index), rank_char(index))
}

pub(crate) fn file_of(index: usize) -> usize {
    index % FILES
}

pub(crate) fn rank_of(index: usize) -> usize {
    index / FILES
}

pub(crate) fn file_char(index: usize) -> char {
    (b'a' + file_of(index) as u8) as char
}

pub(crate) fn rank_char(index: usize) -> char {
    (b'1' + rank_of(index) as u8) as char
}

pub(crate) const WHITE_KINGSIDE: u8 = 1;
pub(crate) const WHITE_QUEENSIDE: u8 = 2;
pub(crate) const BLACK_KINGSIDE: u8 = 4;
pub(crate) const BLACK_QUEENSIDE: u8 = 8;

#[derive(Clone, Copy)]
pub struct Undo {
    captured: Piece,
    capture_square: u8,
    castling: u8,
    en_passant: Option<u8>,
    halfmove_clock: u32,
    hash: u64,
}

#[derive(Clone, Copy)]
pub struct NullUndo {
    en_passant: Option<u8>,
    halfmove_clock: u32,
    hash: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Board {
    squares: [Piece; 64],
    pub(crate) kings: [u8; 2],
    side_to_move: Color,
    pub(crate) castling: u8,
    pub(crate) en_passant: Option<u8>,
    halfmove_clock: u32,
    fullmove_number: u32,
    hash: u64,
}

impl std::fmt::Debug for Board {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} [{:016x}]", self.to_fen(), self.hash)
    }
}

impl Board {
    pub fn startpos() -> Self {
        Self::from_fen(STARTPOS_FEN).expect("startpos FEN is valid")
    }

    pub fn from_fen(fen: &str) -> Option<Self> {
        let mut fields = fen.split_whitespace();
        let placement = fields.next()?;

        let mut squares = [Piece::Empty; 64];
        let mut rows = placement.split('/');
        for rank in (0..RANKS).rev() {
            let row = rows.next()?;
            let mut file = 0;
            for letter in row.chars() {
                if let Some(skipped) = letter.to_digit(10) {
                    if skipped == 0 {
                        return None;
                    }
                    file += skipped as usize;
                } else {
                    if file >= FILES {
                        return None;
                    }
                    squares[rank * FILES + file] = Piece::from_fen_letter(letter)?;
                    file += 1;
                }
            }
            if file != FILES {
                return None;
            }
        }
        if rows.next().is_some() {
            return None;
        }

        let side_to_move = match fields.next() {
            None | Some("w") => Color::White,
            Some("b") => Color::Black,
            Some(_) => return None,
        };

        let mut castling = 0;
        match fields.next() {
            None | Some("-") => {}
            Some(rights) => {
                for letter in rights.chars() {
                    castling |= match letter {
                        'K' => WHITE_KINGSIDE,
                        'Q' => WHITE_QUEENSIDE,
                        'k' => BLACK_KINGSIDE,
                        'q' => BLACK_QUEENSIDE,
                        _ => return None,
                    };
                }
            }
        }

        let en_passant = match fields.next() {
            None | Some("-") => None,
            Some(name) => Some(square_index(name)? as u8),
        };

        let halfmove_clock = match fields.next() {
            None => 0,
            Some(field) => field.parse().ok()?,
        };
        let fullmove_number = match fields.next() {
            None => 1,
            Some(field) => field.parse().ok()?,
        };

        let mut kings = [NO_KING; 2];
        for (square, piece) in squares.iter().enumerate() {
            if *piece != Piece::Empty && piece.kind() == PieceKind::King {
                kings[piece.color() as usize] = square as u8;
            }
        }

        Board {
            squares,
            kings,
            side_to_move,
            castling,
            en_passant,
            halfmove_clock,
            fullmove_number,
            hash: 0,
        }
        .validated()
    }

    pub fn to_fen(&self) -> String {
        let mut placement = String::new();
        for rank in (0..RANKS).rev() {
            let mut empty = 0;
            for file in 0..FILES {
                match self.squares[rank * FILES + file] {
                    Piece::Empty => empty += 1,
                    piece => {
                        if empty > 0 {
                            placement.push_str(&empty.to_string());
                            empty = 0;
                        }
                        placement.push(piece.fen_letter());
                    }
                }
            }
            if empty > 0 {
                placement.push_str(&empty.to_string());
            }
            if rank > 0 {
                placement.push('/');
            }
        }

        let side = match self.side_to_move {
            Color::White => 'w',
            Color::Black => 'b',
        };

        let mut rights = String::new();
        for (bit, letter) in [
            (WHITE_KINGSIDE, 'K'),
            (WHITE_QUEENSIDE, 'Q'),
            (BLACK_KINGSIDE, 'k'),
            (BLACK_QUEENSIDE, 'q'),
        ] {
            if self.castling & bit != 0 {
                rights.push(letter);
            }
        }
        if rights.is_empty() {
            rights.push('-');
        }

        let en_passant = match self.en_passant {
            Some(square) => square_name(square as usize),
            None => "-".to_string(),
        };

        format!(
            "{placement} {side} {rights} {en_passant} {} {}",
            self.halfmove_clock, self.fullmove_number
        )
    }

    fn validated(mut self) -> Option<Self> {
        for color in [Color::White, Color::Black] {
            let king = Piece::new(color, PieceKind::King);
            if (0..64)
                .filter(|&square| self.squares[square] == king)
                .count()
                != 1
            {
                return None;
            }
        }
        let backranks = (0..FILES).chain(56..64);
        if backranks.clone().any(|square| {
            let piece = self.squares[square];
            piece != Piece::Empty && piece.kind() == PieceKind::Pawn
        }) {
            return None;
        }
        if self.in_check(self.side_to_move.opposite()) {
            return None;
        }

        self.castling &= self.castling_supported_by_placement();
        self.en_passant = self
            .en_passant
            .filter(|&square| self.en_passant_is_capturable(square as usize));
        self.hash = self.recomputed_hash();
        Some(self)
    }

    fn recomputed_hash(&self) -> u64 {
        let mut hash = zobrist::castling(self.castling);
        for (square, &piece) in self.squares.iter().enumerate() {
            if piece != Piece::Empty {
                hash ^= zobrist::piece(piece, square);
            }
        }
        if let Some(square) = self.en_passant {
            hash ^= zobrist::en_passant(square);
        }
        if self.side_to_move == Color::Black {
            hash ^= zobrist::BLACK_TO_MOVE;
        }
        hash
    }

    fn castling_supported_by_placement(&self) -> u8 {
        let mut rights = 0;
        for (color, king, kingside, queenside) in [
            (Color::White, 4, WHITE_KINGSIDE, WHITE_QUEENSIDE),
            (Color::Black, 60, BLACK_KINGSIDE, BLACK_QUEENSIDE),
        ] {
            if self.squares[king] != Piece::new(color, PieceKind::King) {
                continue;
            }
            let rook = Piece::new(color, PieceKind::Rook);
            if self.squares[king + 3] == rook {
                rights |= kingside;
            }
            if self.squares[king - 4] == rook {
                rights |= queenside;
            }
        }
        rights
    }

    fn en_passant_is_capturable(&self, square: usize) -> bool {
        let mover = self.side_to_move;
        let (skipped_rank, pushed_rank) = match mover {
            Color::White => (5, 4),
            Color::Black => (2, 3),
        };
        if rank_of(square) != skipped_rank {
            return false;
        }

        let pushed = pushed_rank * FILES + file_of(square);
        if self.squares[pushed] != Piece::new(mover.opposite(), PieceKind::Pawn) {
            return false;
        }

        let waiting = Piece::new(mover, PieceKind::Pawn);
        [-1i32, 1].into_iter().any(|step| {
            let file = file_of(pushed) as i32 + step;
            (0..FILES as i32).contains(&file)
                && self.squares[pushed_rank * FILES + file as usize] == waiting
        })
    }

    pub fn piece_at(&self, index: usize) -> Piece {
        self.squares[index]
    }

    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    pub fn halfmove_clock(&self) -> u32 {
        self.halfmove_clock
    }

    pub fn key(&self) -> u64 {
        self.hash
    }

    pub fn has_pieces_beyond_pawns(&self, color: Color) -> bool {
        self.squares.iter().any(|piece| {
            *piece != Piece::Empty
                && piece.color() == color
                && !matches!(piece.kind(), PieceKind::Pawn | PieceKind::King)
        })
    }

    pub fn make_move(&mut self, mv: Move) -> Undo {
        self.make_move_keyed::<true>(mv)
    }

    pub fn make_null_move(&mut self) -> NullUndo {
        debug_assert!(
            !self.in_check(self.side_to_move),
            "passing the turn in check leaves the king to be captured"
        );
        let undo = NullUndo {
            en_passant: self.en_passant,
            halfmove_clock: self.halfmove_clock,
            hash: self.hash,
        };

        if let Some(square) = self.en_passant {
            self.hash ^= zobrist::en_passant(square);
        }
        self.en_passant = None;
        self.halfmove_clock += 1;
        if self.side_to_move == Color::Black {
            self.fullmove_number += 1;
        }
        self.side_to_move = self.side_to_move.opposite();
        self.hash ^= zobrist::BLACK_TO_MOVE;

        undo
    }

    pub fn unmake_null_move(&mut self, undo: NullUndo) {
        self.side_to_move = self.side_to_move.opposite();
        if self.side_to_move == Color::Black {
            self.fullmove_number -= 1;
        }
        self.en_passant = undo.en_passant;
        self.halfmove_clock = undo.halfmove_clock;
        self.hash = undo.hash;
    }

    pub(crate) fn make_move_without_key(&mut self, mv: Move) -> Undo {
        self.make_move_keyed::<false>(mv)
    }

    fn make_move_keyed<const KEYED: bool>(&mut self, mv: Move) -> Undo {
        let (from, to) = (mv.origin(), mv.target());
        let mut moved = self.squares[from];
        debug_assert!(moved != Piece::Empty, "no piece on {}", square_name(from));
        let was_pawn = moved.kind() == PieceKind::Pawn;
        let hash_before_move = self.hash;
        self.squares[from] = Piece::Empty;
        if KEYED {
            self.hash ^= zobrist::piece(moved, from);
        }

        let mut capture_square = to;
        if was_pawn && file_of(from) != file_of(to) && self.squares[to] == Piece::Empty {
            capture_square = rank_of(from) * FILES + file_of(to);
        }

        let undo = Undo {
            captured: self.squares[capture_square],
            capture_square: capture_square as u8,
            castling: self.castling,
            en_passant: self.en_passant,
            halfmove_clock: self.halfmove_clock,
            hash: hash_before_move,
        };
        let captured = undo.captured != Piece::Empty;
        if KEYED && captured {
            self.hash ^= zobrist::piece(undo.captured, capture_square);
        }
        self.squares[capture_square] = Piece::Empty;

        if moved.kind() == PieceKind::King {
            self.kings[moved.color() as usize] = to as u8;
            if file_of(from).abs_diff(file_of(to)) == 2 {
                let rank = rank_of(from) * FILES;
                let (rook_from, rook_to) = if file_of(to) > file_of(from) {
                    (rank + 7, rank + 5)
                } else {
                    (rank, rank + 3)
                };
                let rook = self.squares[rook_from];
                self.squares[rook_to] = rook;
                self.squares[rook_from] = Piece::Empty;
                if KEYED {
                    self.hash ^= zobrist::piece(rook, rook_from) ^ zobrist::piece(rook, rook_to);
                }
            }
        }

        if let Some(kind) = mv.promotion() {
            moved = Piece::new(moved.color(), kind);
        }
        self.squares[to] = moved;
        if KEYED {
            self.hash ^= zobrist::piece(moved, to);
        }

        self.castling &= !(rights_lost_at(from) | rights_lost_at(to));
        if KEYED && self.castling != undo.castling {
            self.hash ^= zobrist::castling(undo.castling) ^ zobrist::castling(self.castling);
        }

        if was_pawn || captured {
            self.halfmove_clock = 0;
        } else {
            self.halfmove_clock += 1;
        }
        if self.side_to_move == Color::Black {
            self.fullmove_number += 1;
        }
        self.side_to_move = self.side_to_move.opposite();

        self.en_passant = (was_pawn && rank_of(from).abs_diff(rank_of(to)) == 2)
            .then(|| ((from + to) / 2) as u8)
            .filter(|&square| self.en_passant_is_capturable(square as usize));

        if KEYED {
            self.hash ^= zobrist::BLACK_TO_MOVE;
            if let Some(square) = undo.en_passant {
                self.hash ^= zobrist::en_passant(square);
            }
            if let Some(square) = self.en_passant {
                self.hash ^= zobrist::en_passant(square);
            }
        }

        undo
    }

    pub fn unmake_move(&mut self, mv: Move, undo: Undo) {
        let (from, to) = (mv.origin(), mv.target());

        self.side_to_move = self.side_to_move.opposite();
        if self.side_to_move == Color::Black {
            self.fullmove_number -= 1;
        }

        let moved = match mv.promotion() {
            Some(_) => Piece::new(self.side_to_move, PieceKind::Pawn),
            None => self.squares[to],
        };
        self.squares[to] = Piece::Empty;
        self.squares[from] = moved;

        if moved.kind() == PieceKind::King {
            self.kings[moved.color() as usize] = from as u8;
            if file_of(from).abs_diff(file_of(to)) == 2 {
                let rank = rank_of(from) * FILES;
                let (rook_from, rook_to) = if file_of(to) > file_of(from) {
                    (rank + 7, rank + 5)
                } else {
                    (rank, rank + 3)
                };
                self.squares[rook_from] = self.squares[rook_to];
                self.squares[rook_to] = Piece::Empty;
            }
        }

        self.squares[undo.capture_square as usize] = undo.captured;

        self.castling = undo.castling;
        self.en_passant = undo.en_passant;
        self.halfmove_clock = undo.halfmove_clock;
        self.hash = undo.hash;
    }
}

fn rights_lost_at(square: usize) -> u8 {
    match square {
        0 => WHITE_QUEENSIDE,
        4 => WHITE_KINGSIDE | WHITE_QUEENSIDE,
        7 => WHITE_KINGSIDE,
        56 => BLACK_QUEENSIDE,
        60 => BLACK_KINGSIDE | BLACK_QUEENSIDE,
        63 => BLACK_KINGSIDE,
        _ => 0,
    }
}

const FIFTY_MOVE_PLIES: u32 = 100;

pub fn drawn_in_search(board: &Board, path: &[u64]) -> bool {
    board.halfmove_clock() >= FIFTY_MOVE_PLIES
        || repetitions(board, path) > 0
        || insufficient_material(board)
}

pub fn repetitions(board: &Board, path: &[u64]) -> usize {
    let reversible = board.halfmove_clock() as usize;
    path.iter()
        .rev()
        .take(reversible)
        .filter(|&&past| past == board.key())
        .count()
}

pub fn insufficient_material(board: &Board) -> bool {
    let mut knights = 0;
    let mut bishops = Vec::new();

    for square in 0..64 {
        let piece = board.piece_at(square);
        if piece == Piece::Empty {
            continue;
        }
        match piece.kind() {
            PieceKind::King => {}
            PieceKind::Knight => knights += 1,
            PieceKind::Bishop => bishops.push(square_shade(square)),
            _ => return false,
        }
    }

    match (knights, bishops.as_slice()) {
        (0, []) | (1, []) | (0, [_]) => true,
        (0, [first, second]) => first == second,
        _ => false,
    }
}

fn square_shade(square: usize) -> usize {
    (file_of(square) + rank_of(square)) % 2
}

mod zobrist {
    use super::{Piece, file_of};

    const SEED: u64 = 0x2545_F491_4F6C_DD1D;
    const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

    const PIECES: usize = 12;
    const SQUARES: usize = 64;
    const CASTLING_MASKS: usize = 16;
    const FILES: usize = 8;

    const PIECE_SQUARE_COUNTERS: u64 = (PIECES * SQUARES) as u64;
    const CASTLING_COUNTERS: u64 = CASTLING_MASKS as u64;
    const EN_PASSANT_COUNTERS: u64 = FILES as u64;

    const PIECE_SQUARE_BASE: u64 = 0;
    const CASTLING_BASE: u64 = PIECE_SQUARE_BASE + PIECE_SQUARE_COUNTERS;
    const EN_PASSANT_BASE: u64 = CASTLING_BASE + CASTLING_COUNTERS;
    const SIDE_BASE: u64 = EN_PASSANT_BASE + EN_PASSANT_COUNTERS;

    const fn scrambled(counter: u64) -> u64 {
        let mut mixed = SEED.wrapping_add(counter.wrapping_add(1).wrapping_mul(GOLDEN_GAMMA));
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    const fn piece_square_table() -> [u64; PIECES * SQUARES] {
        let mut table = [0; PIECES * SQUARES];
        let mut slot = 0;
        while slot < PIECES * SQUARES {
            table[slot] = scrambled(PIECE_SQUARE_BASE + slot as u64);
            slot += 1;
        }
        table
    }

    const fn castling_table() -> [u64; CASTLING_MASKS] {
        let mut table = [0; CASTLING_MASKS];
        let mut rights = 0;
        while rights < CASTLING_MASKS {
            table[rights] = scrambled(CASTLING_BASE + rights as u64);
            rights += 1;
        }
        table
    }

    const fn en_passant_table() -> [u64; FILES] {
        let mut table = [0; FILES];
        let mut file = 0;
        while file < FILES {
            table[file] = scrambled(EN_PASSANT_BASE + file as u64);
            file += 1;
        }
        table
    }

    const PIECE_SQUARE: [u64; PIECES * SQUARES] = piece_square_table();
    const CASTLING: [u64; CASTLING_MASKS] = castling_table();
    const EN_PASSANT: [u64; FILES] = en_passant_table();

    pub(crate) const BLACK_TO_MOVE: u64 = scrambled(SIDE_BASE);

    pub(crate) fn piece(piece: Piece, square: usize) -> u64 {
        debug_assert!(piece != Piece::Empty);
        PIECE_SQUARE[piece as usize * SQUARES + square]
    }

    pub(crate) fn castling(rights: u8) -> u64 {
        CASTLING[rights as usize]
    }

    pub(crate) fn en_passant(square: u8) -> u64 {
        EN_PASSANT[file_of(square as usize)]
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::HashSet;

        #[test]
        fn every_key_is_distinct_and_non_zero() {
            let mut keys = HashSet::new();
            keys.extend(PIECE_SQUARE);
            keys.extend(CASTLING);
            keys.extend(EN_PASSANT);
            keys.insert(BLACK_TO_MOVE);

            let expected = PIECES * SQUARES + CASTLING_MASKS + FILES + 1;
            assert_eq!(keys.len(), expected, "a key was generated twice");
            assert!(!keys.contains(&0));
        }

        #[test]
        fn empty_castling_rights_still_carry_a_key() {
            assert_ne!(castling(0), 0);
            assert_ne!(castling(0), castling(1));
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        Board, Color, PieceKind, WHITE_KINGSIDE, WHITE_QUEENSIDE, drawn_in_search,
        insufficient_material, repetitions, square_index,
    };

    pub(crate) const REFERENCE_FENS: [&str; 6] = [
        super::STARTPOS_FEN,
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3",
    ];

    fn square(name: &str) -> Option<u8> {
        square_index(name).map(|square| square as u8)
    }

    #[test]
    fn a_fen_is_accepted_only_when_the_position_could_arise() {
        let cases = [
            (
                "8/8/8/8/8/8/8/R6R w - - 0 1",
                false,
                "neither side has a king",
            ),
            ("7k/8/8/8/8/8/8/8 w - - 0 1", false, "white has no king"),
            (
                "7k/8/8/8/8/8/8/K5K1 w - - 0 1",
                false,
                "white has two kings",
            ),
            ("7k/8/8/8/8/8/8/K7 w - - 0 1", true, "one king a side"),
            (
                "4k3/8/8/8/8/8/8/4R1K1 w - - 0 1",
                false,
                "black cannot have moved into the rook's file and handed white the king",
            ),
            (
                "4k3/8/8/8/8/8/8/4R1K1 b - - 0 1",
                true,
                "the same placement is an ordinary check when black is to move",
            ),
            ("4k3/8/8/8/8/8/8/4K2R w - - 0 1", true, "nobody is in check"),
            (
                "7k/8/8/8/8/8/8/K7 x - - 0 1",
                false,
                "malformed side to move",
            ),
            ("7k/8/8/8/8/8/8/K7 w XY - 0 1", false, "malformed castling"),
            (
                "7k/8/8/8/8/8/8/K7 w - zz 0 1",
                false,
                "malformed en passant",
            ),
            (
                "7k/8/8/8/8/8/8/K7 w - - many 1",
                false,
                "malformed halfmove clock",
            ),
            (
                "7k/8/8/8/8/8/8/K7 w - - 0 later",
                false,
                "malformed fullmove number",
            ),
            (
                "P6k/8/8/8/8/8/8/K7 w - - 0 1",
                false,
                "a pawn on the eighth rank",
            ),
            (
                "7k/8/8/8/8/8/8/K6p w - - 0 1",
                false,
                "a pawn on the first rank",
            ),
            (
                "7k/8/8/8/8/8/8/K7",
                true,
                "the side to move may be left out",
            ),
        ];
        for (fen, accepted, reason) in cases {
            assert_eq!(
                Board::from_fen(fen).is_some(),
                accepted,
                "fen {fen:?}: {reason}"
            );
        }
    }

    #[test]
    fn a_parsed_position_keeps_only_the_rights_it_can_use() {
        let cases = [
            (
                "4k3/8/8/8/8/8/8/4K2R w KQkq - 0 1",
                Color::White,
                WHITE_KINGSIDE,
                None,
            ),
            (
                "4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1",
                Color::White,
                WHITE_KINGSIDE | WHITE_QUEENSIDE,
                None,
            ),
            ("4k3/8/8/8/7p/8/8/4K3 w - h3 0 1", Color::White, 0, None),
            ("4k3/8/8/8/8/8/8/4K3 w - e6 0 1", Color::White, 0, None),
            (
                "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1",
                Color::White,
                0,
                square("d6"),
            ),
            ("7k/8/8/8/8/8/8/K7", Color::White, 0, None),
        ];
        for (fen, side, castling, en_passant) in cases {
            let board = Board::from_fen(fen).unwrap();
            assert_eq!(board.side_to_move(), side, "fen {fen:?}, side to move");
            assert_eq!(board.castling, castling, "fen {fen:?}, castling");
            assert_eq!(board.en_passant, en_passant, "fen {fen:?}, en passant");
        }
    }

    #[test]
    fn a_move_leaves_behind_only_the_en_passant_offer_and_clock_it_earns() {
        let cases = [
            ("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1", "e2e4", None, 0),
            ("4k3/8/8/8/3p4/8/4P3/4K3 w - - 0 1", "e2e4", square("e3"), 0),
            ("7k/4P3/8/8/8/8/8/K7 w - - 40 1", "e7e8q", None, 0),
            ("3r3k/4P3/8/8/8/8/8/K7 w - - 40 1", "e7d8q", None, 0),
        ];
        for (fen, uci, en_passant, halfmove_clock) in cases {
            let mut board = Board::from_fen(fen).unwrap();
            board.apply_uci_move(uci).unwrap();
            assert_eq!(board.en_passant, en_passant, "fen {fen:?}, move {uci}");
            assert_eq!(
                board.halfmove_clock(),
                halfmove_clock,
                "fen {fen:?}, move {uci}"
            );
            if let Some(target) = en_passant {
                assert!(
                    board
                        .legal_moves()
                        .iter()
                        .any(|mv| mv.target() == target as usize),
                    "fen {fen:?}, move {uci}: nobody can take en passant"
                );
            }
        }
    }

    #[test]
    fn a_position_survives_a_trip_through_fen_and_back() {
        for fen in REFERENCE_FENS {
            let board = Board::from_fen(fen).unwrap();
            assert_eq!(board.to_fen(), fen, "{fen} did not render back to itself");
            assert_eq!(
                Board::from_fen(&board.to_fen()),
                Some(board),
                "{fen} did not parse back to the same position"
            );
        }
    }

    #[test]
    fn unmaking_a_null_move_restores_the_position_it_was_made_from() {
        let passable = REFERENCE_FENS
            .into_iter()
            .map(|fen| (fen, Board::from_fen(fen).unwrap()))
            .filter(|(_, board)| !board.in_check(board.side_to_move()));

        for (fen, mut board) in passable {
            let before = board.clone();
            let undo = board.make_null_move();
            board.unmake_null_move(undo);
            assert_eq!(board, before, "the null move was not undone on {fen}");
        }
    }

    #[test]
    fn a_null_move_reaches_the_position_a_real_move_would() {
        let cases = [
            (
                "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3",
                "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR b KQkq - 1 3",
            ),
            (
                "4k3/8/8/8/8/8/8/4K3 w - - 0 1",
                "4k3/8/8/8/8/8/8/4K3 b - - 1 1",
            ),
        ];
        for (fen, reached) in cases {
            let mut passing = Board::from_fen(fen).unwrap();
            passing.make_null_move();
            let expected = Board::from_fen(reached).unwrap();
            assert_eq!(passing.to_fen(), reached, "fen {fen:?}");
            assert_eq!(passing.key(), expected.key(), "fen {fen:?}");
        }
    }

    #[test]
    fn only_the_pieces_that_can_move_without_a_pawn_count_against_zugzwang() {
        let cases = [
            (
                "4k3/pppppppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1",
                Color::White,
                false,
            ),
            (
                "4k3/pppppppp/8/8/8/8/PPPPPPPP/4K3 w - - 0 1",
                Color::Black,
                false,
            ),
            (
                "4k3/pppppppp/8/8/8/8/PPPPPPPP/4KB2 w - - 0 1",
                Color::White,
                true,
            ),
            (
                "4k3/pppppppp/8/8/8/8/PPPPPPPP/4KB2 w - - 0 1",
                Color::Black,
                false,
            ),
        ];
        for (fen, side, expected) in cases {
            let board = Board::from_fen(fen).unwrap();
            assert_eq!(
                board.has_pieces_beyond_pawns(side),
                expected,
                "fen {fen:?}, side {side:?}"
            );
        }
    }

    #[test]
    fn unmaking_a_move_restores_the_position_it_was_made_from() {
        for fen in REFERENCE_FENS {
            let mut board = Board::from_fen(fen).unwrap();
            let before = board.clone();
            for mv in board.legal_moves() {
                let undo = board.make_move(mv);
                board.unmake_move(mv, undo);
                assert_eq!(board, before, "{} was not undone on {fen}", mv.uci());
            }
        }
    }

    #[test]
    fn a_promotion_field_never_names_a_king() {
        let cases = [('k', None), ('K', None), ('q', Some(PieceKind::Queen))];
        for (letter, expected) in cases {
            assert_eq!(
                PieceKind::from_promotion_letter(letter),
                expected,
                "letter {letter:?}"
            );
        }
    }

    #[test]
    fn the_incremental_key_agrees_with_a_full_recomputation() {
        for fen in REFERENCE_FENS {
            let mut board = Board::from_fen(fen).unwrap();
            for mv in board.legal_moves() {
                let undo = board.make_move(mv);
                assert_eq!(
                    board.hash,
                    board.recomputed_hash(),
                    "{} drifted the key on {fen}",
                    mv.uci()
                );
                board.unmake_move(mv, undo);
            }
        }
    }

    #[test]
    fn positions_that_repeat_one_another_share_a_key() {
        let cases: [(&str, &[&str], &str, &[&str]); 2] = [
            (
                super::STARTPOS_FEN,
                &["g1f3", "b8c6", "b1c3"],
                super::STARTPOS_FEN,
                &["b1c3", "b8c6", "g1f3"],
            ),
            (
                "4k3/8/8/8/8/8/8/R3K2R w KQ - 0 1",
                &[],
                "4k3/8/8/8/8/8/8/R3K2R w KQ - 37 40",
                &[],
            ),
        ];
        for (first_fen, first_moves, second_fen, second_moves) in cases {
            let mut first = Board::from_fen(first_fen).unwrap();
            for uci in first_moves {
                first.apply_uci_move(uci).unwrap();
            }
            let mut second = Board::from_fen(second_fen).unwrap();
            for uci in second_moves {
                second.apply_uci_move(uci).unwrap();
            }
            assert_eq!(
                first.key(),
                second.key(),
                "{first_fen:?} after {first_moves:?} against {second_fen:?} after {second_moves:?}"
            );
        }
    }

    #[test]
    fn the_key_covers_everything_repetition_compares() {
        let placement = "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR";
        let reference = Board::from_fen(&format!("{placement} w KQkq d6 0 3")).unwrap();

        for (fen, difference) in [
            (format!("{placement} b KQkq d6 0 3"), "side to move"),
            (format!("{placement} w Kkq d6 0 3"), "castling rights"),
            (format!("{placement} w KQkq - 0 3"), "en passant square"),
        ] {
            let board = Board::from_fen(&fen).unwrap();
            assert_ne!(
                board.key(),
                reference.key(),
                "the key ignores the {difference}"
            );
        }
    }

    const KNIGHTS_OUT_AND_BACK: [&str; 4] = ["g1f3", "b8c6", "f3g1", "c6b8"];

    #[test]
    fn insufficient_material_is_what_cannot_deliver_mate() {
        let cases = [
            ("7k/8/8/8/8/8/8/K7 w - - 0 1", true, "lone kings"),
            ("7k/8/8/8/8/8/8/KB6 w - - 0 1", true, "a single bishop"),
            ("7k/8/8/8/8/8/8/KN6 w - - 0 1", true, "a single knight"),
            (
                "6bk/8/8/8/8/8/8/KB6 w - - 0 1",
                true,
                "bishops on b1 and g8 are both light",
            ),
            (
                "5b1k/8/8/8/8/8/8/K1B5 w - - 0 1",
                true,
                "c1 and f8 are both dark, neither side can mate",
            ),
            (
                "6bk/8/8/8/8/8/8/K1B5 w - - 0 1",
                false,
                "c1 is dark and g8 is light, so mate stays possible",
            ),
            ("7k/8/8/8/8/8/8/KQ6 w - - 0 1", false, "a queen"),
            ("7k/8/8/8/8/8/8/KR6 w - - 0 1", false, "a rook"),
            ("7k/8/8/8/8/8/P7/K7 w - - 0 1", false, "a pawn can promote"),
            (
                "7k/8/8/8/8/8/8/KNN5 w - - 0 1",
                false,
                "two knights cannot force mate but can still deliver one",
            ),
        ];
        for (fen, expected, reason) in cases {
            assert_eq!(
                insufficient_material(&Board::from_fen(fen).unwrap()),
                expected,
                "fen {fen:?}: {reason}"
            );
        }
    }

    #[test]
    fn the_search_settles_for_a_single_repetition() {
        let cases: [(&[&str], usize, bool); 3] = [
            (&[], 0, false),
            (&KNIGHTS_OUT_AND_BACK[..2], 0, false),
            (&KNIGHTS_OUT_AND_BACK, 1, true),
        ];
        for (played, expected_repetitions, drawn) in cases {
            let mut board = Board::startpos();
            let mut path = Vec::new();
            for uci in played {
                path.push(board.key());
                board.apply_uci_move(uci).unwrap();
            }
            assert_eq!(
                repetitions(&board, &path),
                expected_repetitions,
                "moves {played:?}"
            );
            assert_eq!(drawn_in_search(&board, &path), drawn, "moves {played:?}");
        }
    }
}
