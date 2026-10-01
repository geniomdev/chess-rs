use crate::board::{Board, Color, Piece, PieceKind, drawn_in_search};
use crate::eval::{evaluate, piece_value};
use crate::movegen::Move;
use crate::tt::{Bound, Hit, TranspositionTable};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAX_DEPTH: u32 = 64;
const MAX_PLY: usize = MAX_DEPTH as usize + 1;
const MAX_QUIESCENCE_PLY: usize = 2 * MAX_PLY;
const INFINITY: i32 = 1_000_000;
const ABORT_CHECK_INTERVAL: u64 = 1024;
const DELTA_MARGIN: i32 = 200;

pub const MATE_SCORE: i32 = 100_000;
const MATE_BOUND: i32 = MATE_SCORE - 2 * MAX_DEPTH as i32;

const TABLE_MOVE_PRIORITY: i32 = i32::MAX;
const TACTICAL_PRIORITY: i32 = 1 << 20;
const KILLER_PRIORITY: i32 = 1 << 19;
const KILLER_SLOTS: usize = 2;
const HISTORY_SLOTS: usize = 2 * 64 * 64;
const HISTORY_LIMIT: i32 = KILLER_PRIORITY - (MAX_DEPTH * MAX_DEPTH) as i32 - 1;

const SLACK_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

const NULL_MOVE_MIN_DEPTH: u32 = 3;
const NULL_MOVE_REDUCTION: u32 = 2;
const NULL_MOVE_DEPTH_DIVISOR: u32 = 6;

const ASPIRATION_MIN_DEPTH: u32 = 4;
const ASPIRATION_WINDOW: i32 = 25;

const LMR_MIN_DEPTH: u32 = 3;
const LMR_MIN_RANK: usize = 3;
const LMR_BASE: f64 = 0.75;
const LMR_DIVISOR: f64 = 2.25;

pub fn mate_distance(score_cp: i32) -> Option<i32> {
    let plies = MATE_SCORE - score_cp.abs();
    if !(0..=(2 * MAX_DEPTH as i32)).contains(&plies) {
        return None;
    }
    let moves = (plies + 1) / 2;
    Some(if score_cp > 0 { moves } else { -moves })
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SearchLimits {
    pub depth: Option<u32>,
    pub movetime_ms: Option<u64>,
    pub nodes: Option<u64>,
    pub slack_cp: i32,
}

#[derive(Debug, Clone)]
pub struct SearchInfo {
    pub depth: u32,
    pub score_cp: i32,
    pub nodes: u64,
    pub nps: u64,
    pub time_ms: u64,
    pub pv: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub best_move: String,
    pub nodes: u64,
}

pub struct Engine {
    board: Board,
    history: Vec<u64>,
    table: TranspositionTable,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            board: Board::startpos(),
            history: Vec::new(),
            table: TranspositionTable::new(),
        }
    }

    pub fn clear_table(&mut self) {
        self.table.clear();
    }

    pub fn set_position(&mut self, fen: &str, moves: &[&str]) -> Option<()> {
        let mut board = Board::from_fen(fen)?;
        let mut history = Vec::new();
        for uci in moves {
            history.push(board.key());
            board.apply_uci_move(uci).ok()?;
        }
        self.board = board;
        self.history = history;
        Some(())
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn search(
        &mut self,
        limits: SearchLimits,
        stop: &AtomicBool,
        on_info: &mut dyn FnMut(SearchInfo),
    ) -> SearchResult {
        let Engine {
            board,
            history,
            table,
        } = self;
        let max_depth = limits.depth.unwrap_or(MAX_DEPTH).clamp(1, MAX_DEPTH);
        let started = Instant::now();
        table.bump_generation();
        let mut context = Context {
            stop,
            table,
            deadline: limits
                .movetime_ms
                .map(|ms| started + Duration::from_millis(ms)),
            node_limit: limits.nodes,
            nodes: 0,
            aborted: false,
            forced: true,
            path: history.clone(),
            lines: vec![Move::NONE; MAX_PLY * MAX_PLY],
            line_lengths: vec![0; MAX_PLY],
            killers: vec![[None; KILLER_SLOTS]; MAX_PLY],
            quiet_history: vec![0; HISTORY_SLOTS],
        };

        let mut best: Option<(Move, i32, Vec<Move>)> = None;
        for depth in 1..=max_depth {
            context.forced = depth == 1;
            let searched = if limits.slack_cp > 0 {
                search_root_within_slack(board, depth, limits.slack_cp, &mut context)
            } else {
                let previous = best.as_ref().map(|(_, score, _)| *score);
                search_root_aspirated(board, depth, previous, &mut context)
            };
            let Some(found) = searched else {
                break;
            };
            if context.aborted {
                if best.is_none() {
                    best = Some(found);
                }
                break;
            }
            let time_ms = started.elapsed().as_millis() as u64;
            on_info(SearchInfo {
                depth,
                score_cp: found.1,
                nodes: context.nodes,
                nps: context.nodes * 1000 / time_ms.max(1),
                time_ms,
                pv: found.2.iter().map(Move::uci).collect(),
            });
            best = Some(found);
            if context.out_of_budget() {
                break;
            }
        }

        match best {
            Some((mv, _, _)) => SearchResult {
                best_move: mv.uci(),
                nodes: context.nodes,
            },
            None => SearchResult {
                best_move: "0000".to_string(),
                nodes: context.nodes,
            },
        }
    }
}

struct Context<'a> {
    stop: &'a AtomicBool,
    table: &'a mut TranspositionTable,
    deadline: Option<Instant>,
    node_limit: Option<u64>,
    nodes: u64,
    aborted: bool,
    forced: bool,
    path: Vec<u64>,
    lines: Vec<Move>,
    line_lengths: Vec<usize>,
    killers: Vec<[Option<Move>; KILLER_SLOTS]>,
    quiet_history: Vec<i32>,
}

impl Context<'_> {
    fn clear_line(&mut self, ply: usize) {
        self.line_lengths[ply] = 0;
    }

    fn extend_line(&mut self, ply: usize, mv: Move) {
        let base = ply * MAX_PLY;
        self.lines[base] = mv;
        if ply + 1 < MAX_PLY {
            let followed = self.line_lengths[ply + 1].min(MAX_PLY - 1);
            let child = (ply + 1) * MAX_PLY;
            self.lines.copy_within(child..child + followed, base + 1);
            self.line_lengths[ply] = followed + 1;
        } else {
            self.line_lengths[ply] = 1;
        }
    }

    fn line(&self, ply: usize) -> &[Move] {
        let base = ply * MAX_PLY;
        &self.lines[base..base + self.line_lengths[ply]]
    }

    fn killer_rank(&self, ply: usize, mv: Move) -> Option<usize> {
        let slots = self.killers.get(ply)?;
        slots.iter().position(|killer| *killer == Some(mv))
    }

    fn remember_killer(&mut self, ply: usize, mv: Move) {
        let Some(slots) = self.killers.get_mut(ply) else {
            return;
        };
        if slots[0] == Some(mv) {
            return;
        }
        slots.copy_within(0..KILLER_SLOTS - 1, 1);
        slots[0] = Some(mv);
    }

    fn quiet_score(&self, mover: Color, mv: Move) -> i32 {
        self.quiet_history[history_slot(mover, mv)]
    }

    fn reward_quiet(&mut self, mover: Color, mv: Move, depth: u32) {
        let slot = history_slot(mover, mv);
        self.quiet_history[slot] += (depth * depth) as i32;
        if self.quiet_history[slot] > HISTORY_LIMIT {
            for score in &mut self.quiet_history {
                *score /= 2;
            }
        }
    }
}

fn history_slot(mover: Color, mv: Move) -> usize {
    (mover as usize) * 64 * 64 + mv.origin() * 64 + mv.target()
}

fn is_quiet(board: &Board, mv: Move) -> bool {
    mv.promotion().is_none() && !board.is_capture(mv)
}

impl Context<'_> {
    fn out_of_budget(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            || self.node_limit.is_some_and(|limit| self.nodes >= limit)
    }

    fn should_abort(&mut self) -> bool {
        if self.forced {
            return false;
        }
        if !self.aborted && self.nodes.is_multiple_of(ABORT_CHECK_INTERVAL) && self.out_of_budget()
        {
            self.aborted = true;
        }
        self.aborted
    }
}

fn search_root_aspirated(
    board: &mut Board,
    depth: u32,
    previous: Option<i32>,
    context: &mut Context,
) -> Option<(Move, i32, Vec<Move>)> {
    let Some(center) =
        previous.filter(|score| depth >= ASPIRATION_MIN_DEPTH && score.abs() < MATE_BOUND)
    else {
        return search_root(board, depth, -INFINITY, INFINITY, context);
    };
    let mut delta = ASPIRATION_WINDOW;
    let mut alpha = center - delta;
    let mut beta = center + delta;
    loop {
        let found = search_root(board, depth, alpha, beta, context)?;
        if context.aborted {
            return Some(found);
        }
        if found.1 <= alpha {
            alpha = (alpha - delta).max(-INFINITY);
        } else if found.1 >= beta {
            beta = (beta + delta).min(INFINITY);
        } else {
            return Some(found);
        }
        delta *= 2;
    }
}

fn search_root(
    board: &mut Board,
    depth: u32,
    mut alpha: i32,
    beta: i32,
    context: &mut Context,
) -> Option<(Move, i32, Vec<Move>)> {
    let mut moves = board.legal_moves();
    if moves.is_empty() {
        return None;
    }
    let key = board.key();
    let first = remembered_move(context.table.probe(key));
    order_moves(board, &mut moves, first, 0, context);

    let original_alpha = alpha;
    let mut best_score = -INFINITY;
    let mut best = moves[0];
    context.clear_line(0);
    context.path.push(board.key());
    for (rank, mv) in moves.into_iter().enumerate() {
        let undo = board.make_move(mv);
        let score = if rank == 0 {
            -negamax(board, depth - 1, 1, -beta, -alpha, true, context)
        } else {
            let narrowed = -negamax(board, depth - 1, 1, -alpha - 1, -alpha, true, context);
            if narrowed > alpha && narrowed < beta && !context.aborted {
                -negamax(board, depth - 1, 1, -beta, -alpha, true, context)
            } else {
                narrowed
            }
        };
        board.unmake_move(mv, undo);
        if context.aborted {
            break;
        }
        if score > best_score {
            best_score = score;
            best = mv;
        }
        if score > alpha {
            alpha = score;
            context.extend_line(0, mv);
        }
        if alpha >= beta {
            break;
        }
    }
    context.path.pop();
    if context.line_lengths[0] == 0 {
        context.extend_line(0, best);
    }
    if !context.aborted {
        let bound = if best_score >= beta {
            Bound::Lower
        } else if best_score > original_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        context
            .table
            .store(key, depth, score_to_table(best_score, 0), bound, best);
    }
    Some((best, best_score, context.line(0).to_vec()))
}

fn search_root_within_slack(
    board: &mut Board,
    depth: u32,
    slack_cp: i32,
    context: &mut Context,
) -> Option<(Move, i32, Vec<Move>)> {
    let mut moves = board.legal_moves();
    if moves.is_empty() {
        return None;
    }
    let key = board.key();
    let first = remembered_move(context.table.probe(key));
    order_moves(board, &mut moves, first, 0, context);

    let fallback = moves[0];
    let mut candidates: Vec<(Move, i32, Vec<Move>)> = Vec::new();
    let mut best = -INFINITY;
    context.clear_line(0);
    context.path.push(key);
    for mv in moves {
        let floor = best.saturating_sub(slack_cp);
        let undo = board.make_move(mv);
        let score = -negamax(board, depth - 1, 1, -INFINITY, -floor, true, context);
        board.unmake_move(mv, undo);
        if context.aborted {
            break;
        }
        if score > floor {
            best = best.max(score);
            context.extend_line(0, mv);
            candidates.push((mv, score, context.line(0).to_vec()));
        }
    }
    context.path.pop();

    if candidates.is_empty() {
        context.extend_line(0, fallback);
        return Some((fallback, best, context.line(0).to_vec()));
    }
    if !context.aborted
        && let Some((mv, score, _)) = candidates.iter().max_by_key(|(_, score, _)| *score)
    {
        context
            .table
            .store(key, depth, score_to_table(*score, 0), Bound::Exact, *mv);
    }

    let threshold = best.saturating_sub(slack_cp);
    let admissible: Vec<&(Move, i32, Vec<Move>)> = candidates
        .iter()
        .filter(|(_, score, _)| *score >= threshold)
        .collect();
    let drawn = (scrambled(key) % admissible.len() as u64) as usize;
    let (mv, score, line) = admissible[drawn];
    Some((*mv, *score, line.clone()))
}

fn scrambled(seed: u64) -> u64 {
    let mut value = seed.wrapping_add(SLACK_MIX);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn negamax(
    board: &mut Board,
    depth: u32,
    ply: usize,
    mut alpha: i32,
    beta: i32,
    null_allowed: bool,
    context: &mut Context,
) -> i32 {
    context.clear_line(ply);
    if depth == 0 {
        return quiescence(board, ply, alpha, beta, context);
    }
    context.nodes += 1;
    if context.should_abort() {
        return 0;
    }
    if drawn_in_search(board, &context.path) {
        return 0;
    }

    let key = board.key();
    let hit = context.table.probe(key);
    if let Some(hit) = hit
        && hit.depth >= depth
    {
        let score = score_from_table(hit.score, ply);
        match hit.bound {
            Bound::Exact => return score,
            Bound::Lower if score >= beta => return score,
            Bound::Upper if score <= alpha => return score,
            _ => {}
        }
    }

    let mover = board.side_to_move();
    let in_check = board.in_check(mover);
    let may_pass = null_allowed
        && beta - alpha == 1
        && depth >= NULL_MOVE_MIN_DEPTH
        && board.has_pieces_beyond_pawns(mover)
        && !in_check;

    if may_pass {
        let reduction = NULL_MOVE_REDUCTION + depth / NULL_MOVE_DEPTH_DIVISOR;
        let undo = board.make_null_move();
        context.path.push(key);
        let score = -negamax(
            board,
            depth.saturating_sub(1 + reduction),
            ply + 1,
            -beta,
            -beta + 1,
            false,
            context,
        );
        context.path.pop();
        board.unmake_null_move(undo);
        if !context.aborted && score >= beta {
            return if score >= MATE_BOUND { beta } else { score };
        }
    }

    let mut moves = board.legal_moves();
    if moves.is_empty() {
        return terminal_score(board, ply);
    }
    order_moves(board, &mut moves, remembered_move(hit), ply, context);

    let original_alpha = alpha;
    let mut best = -INFINITY;
    let mut best_move = Move::NONE;
    context.path.push(key);
    for (rank, mv) in moves.into_iter().enumerate() {
        let reducible = rank >= LMR_MIN_RANK
            && depth >= LMR_MIN_DEPTH
            && !in_check
            && is_quiet(board, mv)
            && context.killer_rank(ply, mv).is_none();
        let undo = board.make_move(mv);
        let score = if rank == 0 {
            -negamax(board, depth - 1, ply + 1, -beta, -alpha, true, context)
        } else {
            let reduction = if reducible && !board.in_check(board.side_to_move()) {
                late_move_reduction(depth, rank)
            } else {
                0
            };
            let mut narrowed = -negamax(
                board,
                depth - 1 - reduction,
                ply + 1,
                -alpha - 1,
                -alpha,
                true,
                context,
            );
            if reduction > 0 && narrowed > alpha && !context.aborted {
                narrowed = -negamax(board, depth - 1, ply + 1, -alpha - 1, -alpha, true, context);
            }
            if narrowed > alpha && narrowed < beta && !context.aborted {
                -negamax(board, depth - 1, ply + 1, -beta, -alpha, true, context)
            } else {
                narrowed
            }
        };
        board.unmake_move(mv, undo);
        if context.aborted {
            break;
        }
        if score > best {
            best = score;
            best_move = mv;
        }
        if score > alpha {
            alpha = score;
            context.extend_line(ply, mv);
        }
        if alpha >= beta {
            if is_quiet(board, mv) {
                context.remember_killer(ply, mv);
                context.reward_quiet(board.side_to_move(), mv, depth);
            }
            break;
        }
    }
    context.path.pop();
    if context.aborted {
        return 0;
    }
    let bound = if best >= beta {
        Bound::Lower
    } else if best > original_alpha {
        Bound::Exact
    } else {
        Bound::Upper
    };
    context
        .table
        .store(key, depth, score_to_table(best, ply), bound, best_move);
    best
}

fn late_move_reduction(depth: u32, rank: usize) -> u32 {
    let reduction = LMR_BASE + (depth as f64).ln() * (rank as f64).ln() / LMR_DIVISOR;
    (reduction as u32).clamp(1, depth - 2)
}

fn remembered_move(hit: Option<Hit>) -> Option<Move> {
    hit.and_then(|hit| hit.mv)
}

fn score_to_table(score: i32, ply: usize) -> i32 {
    if score >= MATE_BOUND {
        score + ply as i32
    } else if score <= -MATE_BOUND {
        score - ply as i32
    } else {
        score
    }
}

fn score_from_table(score: i32, ply: usize) -> i32 {
    if score >= MATE_BOUND {
        score - ply as i32
    } else if score <= -MATE_BOUND {
        score + ply as i32
    } else {
        score
    }
}

fn quiescence(
    board: &mut Board,
    ply: usize,
    mut alpha: i32,
    beta: i32,
    context: &mut Context,
) -> i32 {
    context.nodes += 1;
    if context.should_abort() {
        return 0;
    }
    if drawn_in_search(board, &context.path) {
        return 0;
    }
    if ply >= MAX_QUIESCENCE_PLY {
        return evaluate(board);
    }

    let evading = board.in_check(board.side_to_move());
    let stand_pat = if evading { -INFINITY } else { evaluate(board) };
    if stand_pat >= beta {
        return stand_pat;
    }
    let mut best = stand_pat;
    alpha = alpha.max(stand_pat);

    let mut moves = if evading {
        board.legal_moves()
    } else {
        board.legal_captures()
    };
    if evading && moves.is_empty() {
        return terminal_score(board, ply);
    }
    order_moves(board, &mut moves, None, ply, context);

    context.path.push(board.key());
    for mv in moves {
        if !evading && cannot_reach(board, mv, stand_pat, alpha) {
            continue;
        }
        let undo = board.make_move(mv);
        let score = -quiescence(board, ply + 1, -beta, -alpha, context);
        board.unmake_move(mv, undo);
        if context.aborted {
            break;
        }
        best = best.max(score);
        alpha = alpha.max(score);
        if alpha >= beta {
            break;
        }
    }
    context.path.pop();
    if context.aborted {
        return 0;
    }
    best
}

fn cannot_reach(board: &Board, mv: Move, stand_pat: i32, alpha: i32) -> bool {
    mv.promotion().is_none() && stand_pat + captured_value(board, mv) + DELTA_MARGIN < alpha
}

fn captured_value(board: &Board, mv: Move) -> i32 {
    match board.piece_at(mv.target()) {
        Piece::Empty if board.is_capture(mv) => piece_value(PieceKind::Pawn),
        Piece::Empty => 0,
        victim => piece_value(victim.kind()),
    }
}

fn terminal_score(board: &Board, ply: usize) -> i32 {
    if board.in_check(board.side_to_move()) {
        ply as i32 - MATE_SCORE
    } else {
        0
    }
}

fn order_moves(
    board: &Board,
    moves: &mut [Move],
    first: Option<Move>,
    ply: usize,
    context: &Context,
) {
    moves.sort_by_cached_key(|mv| -move_priority(board, *mv, first, ply, context));
}

fn move_priority(
    board: &Board,
    mv: Move,
    first: Option<Move>,
    ply: usize,
    context: &Context,
) -> i32 {
    if first == Some(mv) {
        return TABLE_MOVE_PRIORITY;
    }
    if !is_quiet(board, mv) {
        let attacker = board.piece_at(mv.origin());
        let attacker_value = if attacker == Piece::Empty {
            0
        } else {
            piece_value(attacker.kind())
        };
        let promoted = mv.promotion().map_or(0, piece_value);
        return TACTICAL_PRIORITY + 10 * captured_value(board, mv) - attacker_value + promoted;
    }
    match context.killer_rank(ply, mv) {
        Some(rank) => KILLER_PRIORITY + (KILLER_SLOTS - rank) as i32,
        None => context.quiet_score(board.side_to_move(), mv),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::STARTPOS_FEN;

    const MATE_IN_ONE_FEN: &str = "6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1";
    const MATED_IN_ONE_FEN: &str = "6k1/R7/8/8/8/8/8/1R5K b - - 0 1";
    const CHECKMATED_ROOT_FEN: &str =
        "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3";
    const STALEMATED_ROOT_FEN: &str = "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1";
    const EN_PASSANT_FEN: &str = "7k/8/8/3pP3/8/8/8/K7 w - d6 0 1";
    const DEFENDED_PAWN_FEN: &str = "4k3/8/4p3/3p4/8/8/8/3QK3 w - - 0 1";
    const DEFENDED_PAWN_WITH_CHECK_FEN: &str = "k7/8/4p3/3p4/8/8/8/3QK3 w - - 0 1";
    const CAPTURE_FEN: &str = "4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1";
    const MIDDLEGAME_FEN: &str =
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    const ENDGAME_FEN: &str = "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1";
    const OPEN_GAME_FEN: &str =
        "r4rk1/1pp1qppp/p1np1n2/2b1p1b1/2B1P3/P1NP1N2/1PP1QPPP/R1B2RK1 w - - 0 10";
    const SLACK_CP: i32 = 100;

    fn scratch<'a>(stop: &'a AtomicBool, table: &'a mut TranspositionTable) -> Context<'a> {
        Context {
            stop,
            table,
            deadline: None,
            node_limit: None,
            nodes: 0,
            aborted: false,
            forced: true,
            path: Vec::new(),
            lines: vec![Move::NONE; MAX_PLY * MAX_PLY],
            line_lengths: vec![0; MAX_PLY],
            killers: vec![[None; KILLER_SLOTS]; MAX_PLY],
            quiet_history: vec![0; HISTORY_SLOTS],
        }
    }

    type KillerCase<'a> = (&'a [(usize, Move)], usize, Move, Option<usize>, &'a str);
    type SearchCase<'a> = (&'a str, &'a [&'a str], SearchLimits, &'a [Check], &'a str);

    fn quiet(origin: usize, target: usize) -> Move {
        Move::new(origin, target, None)
    }

    fn depth(d: u32) -> SearchLimits {
        SearchLimits {
            depth: Some(d),
            ..SearchLimits::default()
        }
    }

    fn within(nodes: u64) -> SearchLimits {
        SearchLimits {
            depth: Some(64),
            nodes: Some(nodes),
            ..SearchLimits::default()
        }
    }

    fn within_ms(movetime_ms: u64) -> SearchLimits {
        SearchLimits {
            depth: Some(64),
            movetime_ms: Some(movetime_ms),
            ..SearchLimits::default()
        }
    }

    fn slack(d: u32, slack_cp: i32) -> SearchLimits {
        SearchLimits {
            depth: Some(d),
            slack_cp,
            ..SearchLimits::default()
        }
    }

    fn prepared(fen: &str, moves: &[&str]) -> Engine {
        let mut engine = Engine::new();
        engine.set_position(fen, moves).unwrap();
        engine
    }

    fn scored_search(engine: &mut Engine, limits: SearchLimits) -> (SearchResult, Option<i32>) {
        let stop = AtomicBool::new(false);
        let mut score_cp = None;
        let result = engine.search(limits, &stop, &mut |info| score_cp = Some(info.score_cp));
        (result, score_cp)
    }

    fn searched(fen: &str, limits: SearchLimits) -> (SearchResult, Option<i32>) {
        scored_search(&mut prepared(fen, &[]), limits)
    }

    fn is_playable(fen: &str, uci: &str) -> bool {
        Board::from_fen(fen).unwrap().apply_uci_move(uci).is_ok()
    }

    #[test]
    fn the_killer_slots_hold_the_most_recent_moves_of_their_own_ply() {
        let earlier = quiet(12, 28);
        let later = quiet(11, 27);
        let cases: [KillerCase; 4] = [
            (
                &[(3, earlier), (3, later)],
                3,
                later,
                Some(0),
                "the latest killer comes first",
            ),
            (
                &[(3, earlier), (3, later)],
                3,
                earlier,
                Some(1),
                "the earlier killer moves down a slot",
            ),
            (
                &[(3, earlier), (3, later), (3, later)],
                3,
                earlier,
                Some(1),
                "a killer stored twice must not evict the other one",
            ),
            (
                &[(3, earlier)],
                4,
                earlier,
                None,
                "a killer is only remembered for the ply that cut off",
            ),
        ];
        for (remembered, ply, mv, expected, reason) in cases {
            let stop = AtomicBool::new(false);
            let mut table = TranspositionTable::new();
            let mut context = scratch(&stop, &mut table);
            for (at, killer) in remembered {
                context.remember_killer(*at, *killer);
            }
            assert_eq!(context.killer_rank(ply, mv), expected, "{reason}");
        }
    }

    #[test]
    fn a_late_move_is_reduced_more_the_deeper_and_later_it_comes() {
        let cases = [
            (3, 3, 1, "the shallowest reducible node still loses a ply"),
            (
                3,
                200,
                1,
                "the reduction never drops a move straight into quiescence",
            ),
            (
                8,
                3,
                1,
                "an early quiet move at moderate depth loses a single ply",
            ),
            (8, 20, 3, "a late quiet move loses more"),
            (64, 64, 8, "the deepest and latest moves lose the most"),
        ];
        for (depth, rank, expected, reason) in cases {
            assert_eq!(
                late_move_reduction(depth, rank),
                expected,
                "depth {depth}, rank {rank}: {reason}"
            );
        }
    }

    #[test]
    fn a_window_that_misses_the_score_widens_until_it_holds_it() {
        let cases = [
            (STARTPOS_FEN, 5, 800, "the guess is far too high"),
            (STARTPOS_FEN, 5, -800, "the guess is far too low"),
            (MIDDLEGAME_FEN, 4, 0, "the guess is close"),
            (ENDGAME_FEN, 5, -3000, "the guess is wildly off"),
        ];
        for (fen, deepest, guess, reason) in cases {
            let stop = AtomicBool::new(false);
            let mut board = Board::from_fen(fen).unwrap();

            let mut table = TranspositionTable::new();
            let mut context = scratch(&stop, &mut table);
            let (_, full_score, _) =
                search_root(&mut board, deepest, -INFINITY, INFINITY, &mut context).unwrap();

            let mut table = TranspositionTable::new();
            let mut context = scratch(&stop, &mut table);
            let (mv, aspirated_score, _) =
                search_root_aspirated(&mut board, deepest, Some(guess), &mut context).unwrap();

            assert_eq!(
                aspirated_score, full_score,
                "fen {fen:?}, depth {deepest}, guess {guess}: {reason}"
            );
            assert!(is_playable(fen, &mv.uci()), "fen {fen:?}: {reason}");
        }
    }

    #[test]
    fn the_move_order_puts_the_likeliest_refutation_first() {
        let cases = [
            (
                STARTPOS_FEN,
                Some(quiet(11, 27)),
                Some(quiet(12, 28)),
                None,
                quiet(11, 27),
                quiet(12, 28),
                "a history score that has saturated still ranks below a killer",
            ),
            (
                CAPTURE_FEN,
                Some(quiet(4, 5)),
                None,
                None,
                quiet(28, 35),
                quiet(4, 5),
                "a capture outranks a killer",
            ),
            (
                CAPTURE_FEN,
                Some(quiet(4, 5)),
                None,
                None,
                quiet(4, 5),
                quiet(4, 3),
                "a killer outranks an unrewarded quiet move",
            ),
            (
                CAPTURE_FEN,
                None,
                None,
                Some(quiet(4, 5)),
                quiet(4, 5),
                quiet(28, 35),
                "the table move outranks every heuristic",
            ),
        ];
        for (fen, killer, rewarded, remembered, higher, lower, reason) in cases {
            let stop = AtomicBool::new(false);
            let mut table = TranspositionTable::new();
            let mut context = scratch(&stop, &mut table);
            let board = Board::from_fen(fen).unwrap();
            if let Some(killer) = killer {
                context.remember_killer(0, killer);
            }
            if let Some(rewarded) = rewarded {
                for _ in 0..10_000 {
                    context.reward_quiet(board.side_to_move(), rewarded, MAX_DEPTH);
                }
                assert!(
                    context.quiet_score(board.side_to_move(), rewarded) > 0,
                    "{reason}: the reward did not register"
                );
            }
            assert!(
                move_priority(&board, higher, remembered, 0, &context)
                    > move_priority(&board, lower, remembered, 0, &context),
                "{reason}"
            );
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Check {
        Plays(&'static str),
        Avoids(&'static str),
        ScoreBetween(i32, i32),
        MateIn(i32),
    }

    #[test]
    fn the_search_answers_each_position_with_the_move_and_score_it_deserves() {
        let cases: [SearchCase; 18] = [
            (
                STARTPOS_FEN,
                &[],
                depth(1),
                &[Check::ScoreBetween(-199, 199)],
                "nothing hangs in the opening position",
            ),
            (
                MATE_IN_ONE_FEN,
                &[],
                depth(2),
                &[Check::Plays("a1a8"), Check::MateIn(1)],
                "the back rank mate is found and scored as a move count",
            ),
            (
                MATE_IN_ONE_FEN,
                &[],
                depth(8),
                &[Check::ScoreBetween(401, MATE_SCORE)],
                "a mate that needs a quiet move survives the passed turn",
            ),
            (
                MATED_IN_ONE_FEN,
                &[],
                depth(4),
                &[Check::MateIn(-1)],
                "a mate against the side to move counts down from the other side",
            ),
            (
                CHECKMATED_ROOT_FEN,
                &[],
                depth(3),
                &[Check::Plays("0000")],
                "a checkmated root is answered with a null move",
            ),
            (
                STALEMATED_ROOT_FEN,
                &[],
                depth(3),
                &[Check::Plays("0000")],
                "a stalemated root is answered with a null move",
            ),
            (
                EN_PASSANT_FEN,
                &[],
                depth(4),
                &[Check::Plays("e5d6")],
                "an en passant capture is worth the pawn it takes",
            ),
            (
                DEFENDED_PAWN_FEN,
                &[],
                depth(1),
                &[Check::Avoids("d1d5"), Check::ScoreBetween(501, MATE_SCORE)],
                "the last ply no longer hides the recapture from e6",
            ),
            (
                DEFENDED_PAWN_WITH_CHECK_FEN,
                &[],
                depth(1),
                &[Check::Avoids("d1d5")],
                "exd5 is legal while the king is in check and wins the queen",
            ),
            (
                "7k/8/8/8/8/8/8/KN6 w - - 0 1",
                &[],
                depth(4),
                &[Check::ScoreBetween(0, 0)],
                "K+N vs K is a dead draw, not a knight up",
            ),
            (
                "8/8/8/p7/1p6/1P6/P7/K1k5 w - - 0 1",
                &[],
                depth(12),
                &[Check::ScoreBetween(-MATE_SCORE, 0)],
                "white is in zugzwang with only pawns to move and is never offered the passed turn",
            ),
            (
                "7k/8/7q/8/8/8/8/K7 w - - 0 1",
                &[],
                depth(4),
                &[Check::ScoreBetween(-MATE_SCORE, -501)],
                "white is a queen down",
            ),
            (
                "7k/8/7q/8/8/8/8/K7 w - - 0 1",
                &["a1b1", "h8g8", "b1a1", "g8h8"],
                depth(4),
                &[Check::ScoreBetween(0, 0)],
                "stepping back to b1 repeats a position the history already holds",
            ),
            (
                MATE_IN_ONE_FEN,
                &[],
                slack(1, SLACK_CP),
                &[Check::Plays("a1a8")],
                "a mate stays out of reach of the slack",
            ),
            (
                STARTPOS_FEN,
                &[],
                within(3_000),
                &[],
                "a node budget cuts the search short and still answers",
            ),
            (
                STARTPOS_FEN,
                &[],
                within_ms(50),
                &[],
                "a time budget cuts the search short and still answers",
            ),
            (
                MIDDLEGAME_FEN,
                &[],
                depth(3),
                &[],
                "a middlegame search answers with a legal move",
            ),
            (
                ENDGAME_FEN,
                &[],
                depth(3),
                &[],
                "an endgame search answers with a legal move",
            ),
        ];
        for (fen, moves, limits, checks, reason) in cases {
            let started = Instant::now();
            let (result, score_cp) = scored_search(&mut prepared(fen, moves), limits);
            let context = format!("fen {fen:?}, moves {moves:?}, {limits:?}: {reason}");

            assert!(
                started.elapsed() < Duration::from_secs(10),
                "{context}: took {:?}",
                started.elapsed()
            );
            if let Some(budget) = limits.nodes {
                assert!(
                    result.nodes < budget * 30,
                    "{context}: ran to {} nodes",
                    result.nodes
                );
            }
            if result.best_move != "0000" {
                let mut board = Board::from_fen(fen).unwrap();
                for uci in moves {
                    board.apply_uci_move(uci).unwrap();
                }
                assert!(
                    board.apply_uci_move(&result.best_move).is_ok(),
                    "{context}: answered {:?}, which cannot be played",
                    result.best_move
                );
            }
            for check in checks {
                let held = match *check {
                    Check::Plays(uci) => result.best_move == uci,
                    Check::Avoids(uci) => result.best_move != uci,
                    Check::ScoreBetween(low, high) => {
                        score_cp.is_some_and(|score| (low..=high).contains(&score))
                    }
                    Check::MateIn(moves) => score_cp.and_then(mate_distance) == Some(moves),
                };
                assert!(
                    held,
                    "{context}: {check:?} failed, got {} scored {score_cp:?}",
                    result.best_move
                );
            }
        }
    }

    #[test]
    fn a_search_already_told_to_stop_still_answers() {
        for fen in [STARTPOS_FEN, MIDDLEGAME_FEN, ENDGAME_FEN] {
            let stop = AtomicBool::new(true);
            let result = prepared(fen, &[]).search(depth(64), &stop, &mut |_| {});
            assert!(
                is_playable(fen, &result.best_move),
                "fen {fen:?}: answered {:?}",
                result.best_move
            );
        }
    }

    #[test]
    fn a_position_is_set_only_when_it_and_every_move_after_it_can_be_read() {
        let cases: [(&str, &[&str], Option<&str>); 4] = [
            (STARTPOS_FEN, &[], Some(STARTPOS_FEN)),
            (
                STARTPOS_FEN,
                &["e2e4", "e7e5"],
                Some("rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2"),
            ),
            ("not a position at all", &[], None),
            (STARTPOS_FEN, &["e2e5"], None),
        ];
        for (fen, moves, expected) in cases {
            let mut engine = Engine::new();
            let accepted = engine.set_position(fen, moves).is_some();
            assert_eq!(accepted, expected.is_some(), "fen {fen:?}, moves {moves:?}");
            if let Some(reached) = expected {
                assert_eq!(
                    engine.board().to_fen(),
                    reached,
                    "fen {fen:?}, moves {moves:?}"
                );
            }
        }
    }

    #[test]
    fn mate_distance_reads_only_mate_scores_as_move_counts() {
        let cases = [
            (45, None),
            (-1200, None),
            (MATE_SCORE - 1, Some(1)),
            (MATE_SCORE - 3, Some(2)),
            (2 - MATE_SCORE, Some(-1)),
            (3 - MATE_SCORE, Some(-2)),
        ];
        for (score_cp, expected) in cases {
            assert_eq!(mate_distance(score_cp), expected, "score {score_cp}");
        }
    }

    #[test]
    fn every_depth_is_reported_with_a_line_that_runs_to_it() {
        for (fen, deepest) in [(STARTPOS_FEN, 3), (STARTPOS_FEN, 4), (MIDDLEGAME_FEN, 3)] {
            let stop = AtomicBool::new(false);
            let mut reported = Vec::new();
            prepared(fen, &[]).search(depth(deepest), &stop, &mut |info| {
                reported.push((info.depth, info.pv.clone()));
            });

            let depths: Vec<u32> = reported.iter().map(|(depth, _)| *depth).collect();
            assert_eq!(
                depths,
                (1..=deepest).collect::<Vec<_>>(),
                "fen {fen:?}, depth {deepest}"
            );
            let (_, line) = reported.last().unwrap();
            assert_eq!(
                line.len(),
                deepest as usize,
                "fen {fen:?}: a {deepest} ply search reports {deepest} ply"
            );
            let mut board = Board::from_fen(fen).unwrap();
            for uci in line {
                assert!(
                    board.apply_uci_move(uci).is_ok(),
                    "fen {fen:?}: line {line:?} leaves the board at {uci}"
                );
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Before {
        Searched,
        SearchedThenCleared,
        CutShort,
    }

    #[test]
    fn what_the_table_remembers_never_changes_the_answer() {
        let cases = [
            (STARTPOS_FEN, 5, Before::Searched),
            (MIDDLEGAME_FEN, 5, Before::Searched),
            (ENDGAME_FEN, 5, Before::Searched),
            (OPEN_GAME_FEN, 5, Before::Searched),
            (MATE_IN_ONE_FEN, 5, Before::Searched),
            (MATED_IN_ONE_FEN, 4, Before::Searched),
            (STARTPOS_FEN, 5, Before::SearchedThenCleared),
            (STARTPOS_FEN, 5, Before::CutShort),
        ];
        for (fen, deepest, before) in cases {
            let stop = AtomicBool::new(false);
            let (cold, cold_score) = searched(fen, depth(deepest));

            let mut engine = prepared(fen, &[]);
            match before {
                Before::Searched => {
                    engine.search(depth(deepest), &stop, &mut |_| {});
                }
                Before::SearchedThenCleared => {
                    engine.search(depth(deepest), &stop, &mut |_| {});
                    engine.clear_table();
                }
                Before::CutShort => {
                    engine.search(within(3_000), &stop, &mut |_| {});
                }
            }
            let (again, again_score) = scored_search(&mut engine, depth(deepest));
            let context = format!("fen {fen:?}, depth {deepest}, after {before:?}");

            assert_eq!(again_score, cold_score, "{context}: the score moved");
            match before {
                Before::Searched => assert!(
                    again.nodes < cold.nodes,
                    "{context}: warm search took {} nodes against {} cold",
                    again.nodes,
                    cold.nodes
                ),
                Before::SearchedThenCleared => {
                    assert_eq!(again.nodes, cold.nodes, "{context}");
                    assert_eq!(again.best_move, cold.best_move, "{context}");
                }
                Before::CutShort => {}
            }
        }
    }

    #[test]
    fn a_slack_search_trades_at_most_the_slack_for_variety() {
        let cases = [
            STARTPOS_FEN,
            MIDDLEGAME_FEN,
            OPEN_GAME_FEN,
            DEFENDED_PAWN_FEN,
        ];
        let mut varied = Vec::new();
        for fen in cases {
            let (full, full_score) = searched(fen, depth(1));
            let (weakened, weakened_score) = searched(fen, slack(1, SLACK_CP));
            let (repeated, _) = searched(fen, slack(1, SLACK_CP));
            let (full_score, weakened_score) = (full_score.unwrap(), weakened_score.unwrap());

            assert!(
                is_playable(fen, &weakened.best_move),
                "slack answered {:?}, which is not legal in {fen}",
                weakened.best_move
            );
            assert!(
                weakened_score >= full_score - SLACK_CP,
                "slack gave away {} in {fen}",
                full_score - weakened_score
            );
            assert_eq!(
                weakened.best_move, repeated.best_move,
                "{fen} drew two moves"
            );
            if weakened.best_move != full.best_move {
                varied.push(fen);
            }
        }
        assert!(
            !varied.is_empty(),
            "a pawn of slack never picked anything but the best move"
        );
    }
}
