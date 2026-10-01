mod board;
mod eval;
mod movegen;
mod search;
mod tt;
mod uci;

use board::{Color, STARTPOS_FEN};
use movegen::perft;
use search::{Engine, SearchLimits, mate_distance};
use std::fmt::Arguments;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tt::{DEFAULT_HASH_MB, MAX_HASH_MB, MIN_HASH_MB};
use uci::{
    GoLimits, LEVELS, Level, Score, UciCommand, format_bestmove, format_info, parse_command,
};

type Output = Arc<Mutex<dyn Write + Send>>;

macro_rules! say {
    ($output:expr) => {
        write_line($output, format_args!(""))
    };
    ($output:expr, $($line:tt)*) => {
        write_line($output, format_args!($($line)*))
    };
}

fn write_line(output: &Output, line: Arguments) {
    let mut sink = output.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = writeln!(sink, "{line}");
    let _ = sink.flush();
}

const BENCH_DEPTH: u32 = 5;
const BENCH_POSITIONS: [&str; 6] = [
    STARTPOS_FEN,
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1b1/2B1P3/P1NP1N2/1PP1QPPP/R1B2RK1 w - - 0 10",
];

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let output: Output = Arc::new(Mutex::new(io::stdout()));
    if arguments.first().is_some_and(|first| first == "bench") {
        run_bench(
            &output,
            arguments.get(1).and_then(|value| value.parse().ok()),
        );
        return ExitCode::SUCCESS;
    }

    let mut strength = Strength::default();
    if let Some(name) = flag_value(&arguments, "--level") {
        let Ok(level) = name.parse() else {
            eprintln!(
                "unknown level {name}, expected one of {}",
                level_names(", ")
            );
            return ExitCode::FAILURE;
        };
        strength.level = Some(level);
    }

    run(io::stdin().lock(), &output, strength);
    ExitCode::SUCCESS
}

fn run(input: impl BufRead, output: &Output, mut strength: Strength) {
    let mut fen = STARTPOS_FEN.to_string();
    let mut moves: Vec<String> = Vec::new();
    let stop = Arc::new(AtomicBool::new(false));
    let mut hash_mb = DEFAULT_HASH_MB;
    let mut idle = Some(Engine::with_hash_mb(hash_mb));
    let mut search: Option<JoinHandle<Engine>> = None;

    for line in input.lines() {
        let Ok(line) = line else { break };
        match parse_command(&line) {
            UciCommand::Uci => {
                say!(output, "id name chess-engine {}", env!("CARGO_PKG_VERSION"));
                say!(output, "id author geniom");
                say!(
                    output,
                    "option name Hash type spin default {DEFAULT_HASH_MB} min {MIN_HASH_MB} max {MAX_HASH_MB}"
                );
                say!(
                    output,
                    "option name Slack type spin default 0 min 0 max {MAX_SLACK_CP}"
                );
                say!(
                    output,
                    "option name Level type combo default {NO_LEVEL} var {NO_LEVEL} var {}",
                    level_names(" var ")
                );
                say!(output, "uciok");
            }
            UciCommand::IsReady => say!(output, "readyok"),
            UciCommand::SetOption(option) if option.named("hash") => {
                let Some(megabytes) = parsed_hash(&option.value) else {
                    eprintln!(
                        "unreadable hash size {}, staying at {hash_mb} MB",
                        option.value.trim()
                    );
                    continue;
                };
                hash_mb = megabytes;
                finish(&stop, &mut search, &mut idle);
                match idle.as_mut() {
                    Some(engine) => engine.resize_table(hash_mb),
                    None => idle = Some(Engine::with_hash_mb(hash_mb)),
                }
            }
            UciCommand::SetOption(option) if option.named("slack") => {
                strength.slack_cp = parsed_slack(&option.value);
            }
            UciCommand::SetOption(option) if option.named("level") => {
                match requested_level(&option.value) {
                    Ok(level) => strength.level = level,
                    Err(name) => eprintln!(
                        "unknown level {name}, staying at {}",
                        strength.level.map_or(NO_LEVEL, Level::as_str)
                    ),
                }
            }
            UciCommand::SetOption(_) => {}
            UciCommand::NewGame => {
                fen = STARTPOS_FEN.to_string();
                moves.clear();
                finish(&stop, &mut search, &mut idle);
                if let Some(engine) = idle.as_mut() {
                    engine.clear_table();
                }
            }
            UciCommand::Position(position) => {
                fen = position.fen.unwrap_or_else(|| STARTPOS_FEN.to_string());
                moves = position.moves;
            }
            UciCommand::Go(limits) => {
                finish(&stop, &mut search, &mut idle);
                stop.store(false, Ordering::Relaxed);
                let mut engine = idle.take().unwrap_or_else(|| Engine::with_hash_mb(hash_mb));
                let refs: Vec<&str> = moves.iter().map(String::as_str).collect();
                if engine.set_position(&fen, &refs).is_none() {
                    idle = Some(engine);
                    say!(output, "{}", format_bestmove("0000"));
                    continue;
                }
                let flag = Arc::clone(&stop);
                let reply = Arc::clone(output);
                search = Some(std::thread::spawn(move || {
                    run_search(&reply, &mut engine, limits, strength, &flag);
                    engine
                }));
            }
            UciCommand::GoPerft(depth) => run_perft(output, &fen, &moves, depth),
            UciCommand::Bench { depth } => run_bench(output, depth),
            UciCommand::Stop => finish(&stop, &mut search, &mut idle),
            UciCommand::Quit => break,
            UciCommand::Unknown(_) => {}
        }
    }
    finish(&stop, &mut search, &mut idle);
}

fn flag_value<'a>(arguments: &'a [String], flag: &str) -> Option<&'a str> {
    let position = arguments.iter().position(|argument| argument == flag)?;
    arguments.get(position + 1).map(String::as_str)
}

fn level_names(separator: &str) -> String {
    LEVELS.map(Level::as_str).join(separator)
}

fn requested_level(value: &str) -> Result<Option<Level>, &str> {
    let name = value.trim();
    if name.eq_ignore_ascii_case(NO_LEVEL) {
        return Ok(None);
    }
    name.parse().map(Some).map_err(|_| name)
}

fn finish(stop: &AtomicBool, search: &mut Option<JoinHandle<Engine>>, idle: &mut Option<Engine>) {
    stop.store(true, Ordering::Relaxed);
    if let Some(worker) = search.take()
        && let Ok(engine) = worker.join()
    {
        *idle = Some(engine);
    }
}

fn prepared_engine(fen: &str, moves: &[String]) -> Option<Engine> {
    let mut engine = Engine::new();
    let refs: Vec<&str> = moves.iter().map(String::as_str).collect();
    engine.set_position(fen, &refs)?;
    Some(engine)
}

fn run_search(
    output: &Output,
    engine: &mut Engine,
    limits: GoLimits,
    strength: Strength,
    stop: &AtomicBool,
) {
    let limits = search_limits(limits, engine.board().side_to_move(), strength);
    let result = engine.search(limits, stop, &mut |progress| {
        say!(
            output,
            "{}",
            format_info(
                progress.depth,
                reported_score(progress.score_cp),
                progress.nodes,
                progress.nps,
                progress.time_ms,
                &progress.pv
            )
        );
    });
    say!(output, "{}", format_bestmove(&result.best_move));
}

fn reported_score(score_cp: i32) -> Score {
    match mate_distance(score_cp) {
        Some(moves) => Score::MateIn(moves),
        None => Score::Centipawns(score_cp),
    }
}

const MOVE_OVERHEAD_MS: u64 = 100;
const MAX_SLACK_CP: i32 = 1000;
const NO_LEVEL: &str = "none";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Strength {
    level: Option<Level>,
    slack_cp: i32,
}

impl Strength {
    fn slack(self) -> i32 {
        match self.level {
            Some(level) => level.slack_cp(),
            None => self.slack_cp,
        }
    }

    fn capped_depth(self, asked: Option<u32>) -> Option<u32> {
        let ceiling = self.level.map(Level::depth);
        match (asked, ceiling) {
            (Some(asked), Some(ceiling)) => Some(asked.min(ceiling)),
            (asked, ceiling) => asked.or(ceiling),
        }
    }

    fn capped_movetime(self, asked: Option<u64>) -> Option<u64> {
        let ceiling = self.level.map(Level::movetime_ms);
        match (asked, ceiling) {
            (Some(asked), Some(ceiling)) => Some(asked.min(ceiling)),
            (asked, ceiling) => asked.or(ceiling),
        }
    }
}

fn parsed_slack(value: &str) -> i32 {
    value.trim().parse().unwrap_or(0).clamp(0, MAX_SLACK_CP)
}

fn parsed_hash(value: &str) -> Option<usize> {
    let megabytes: usize = value.trim().parse().ok()?;
    Some(megabytes.clamp(MIN_HASH_MB, MAX_HASH_MB))
}

fn search_limits(limits: GoLimits, side: Color, strength: Strength) -> SearchLimits {
    if limits.infinite {
        return SearchLimits {
            depth: strength.capped_depth(limits.depth),
            movetime_ms: strength.capped_movetime(None),
            nodes: limits.nodes,
            slack_cp: strength.slack(),
        };
    }

    let (time, increment) = match side {
        Color::White => (limits.wtime_ms, limits.winc_ms),
        Color::Black => (limits.btime_ms, limits.binc_ms),
    };
    let clock_budget = time.map(|remaining| {
        (remaining / 30 + increment.unwrap_or(0))
            .min(remaining.saturating_sub(MOVE_OVERHEAD_MS))
            .max(1)
    });
    SearchLimits {
        depth: strength.capped_depth(limits.depth),
        movetime_ms: strength.capped_movetime(limits.movetime_ms.or(clock_budget)),
        nodes: limits.nodes,
        slack_cp: strength.slack(),
    }
}

fn run_bench(output: &Output, depth: Option<u32>) {
    let depth = depth.unwrap_or(BENCH_DEPTH);
    let stop = AtomicBool::new(false);
    let started = Instant::now();
    let mut total_nodes = 0;

    for (number, fen) in BENCH_POSITIONS.iter().enumerate() {
        let Some(mut engine) = prepared_engine(fen, &[]) else {
            continue;
        };
        let limits = SearchLimits {
            depth: Some(depth),
            ..SearchLimits::default()
        };
        let result = engine.search(limits, &stop, &mut |_| {});
        say!(
            output,
            "position {}/{}: nodes {:>10}  bestmove {}",
            number + 1,
            BENCH_POSITIONS.len(),
            result.nodes,
            result.best_move
        );
        total_nodes += result.nodes;
    }

    let elapsed = started.elapsed();
    say!(output, "===========================");
    say!(output, "depth           : {depth}");
    say!(output, "total time (ms) : {}", elapsed.as_millis());
    say!(output, "nodes searched  : {total_nodes}");
    say!(
        output,
        "nodes/second    : {}",
        nodes_per_second(total_nodes, elapsed)
    );
}

fn nodes_per_second(nodes: u64, elapsed: Duration) -> u64 {
    nodes * 1000 / (elapsed.as_millis() as u64).max(1)
}

fn run_perft(output: &Output, fen: &str, moves: &[String], depth: u32) {
    let Some(engine) = prepared_engine(fen, moves) else {
        say!(output, "Nodes searched: 0");
        return;
    };
    if depth == 0 {
        say!(output);
        say!(output, "Nodes searched: 1");
        return;
    }
    let mut board = engine.board().clone();
    let mut total = 0;
    let started = Instant::now();
    for mv in board.legal_moves() {
        let undo = board.make_move(mv);
        let count = perft(&board, depth - 1);
        board.unmake_move(mv, undo);
        say!(output, "{}: {count}", mv.uci());
        total += count;
    }
    let elapsed = started.elapsed();
    say!(output);
    say!(output, "Nodes searched: {total}");
    say!(output, "time (ms)     : {}", elapsed.as_millis());
    say!(
        output,
        "nodes/second  : {}",
        nodes_per_second(total, elapsed)
    );
}
#[cfg(test)]
mod tests {
    use super::{
        MAX_HASH_MB, MAX_SLACK_CP, MIN_HASH_MB, MOVE_OVERHEAD_MS, Strength, flag_value,
        level_names, nodes_per_second, parsed_hash, parsed_slack, reported_score, requested_level,
        run, search_limits,
    };
    use crate::board::Color;
    use crate::search::MATE_SCORE;
    use crate::uci::{GoLimits, Level, Score};
    use std::io::{self, BufRead, BufReader, PipeReader, PipeWriter, Write};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    const SEARCH: &str = "go depth 5";

    struct EngineSession {
        input: Option<PipeWriter>,
        output: BufReader<PipeReader>,
        worker: Option<JoinHandle<()>>,
    }

    impl EngineSession {
        fn start() -> Self {
            let (commands, input) = io::pipe().expect("a pipe for the commands");
            let (replies, output) = io::pipe().expect("a pipe for the replies");
            let worker = std::thread::spawn(move || {
                run(
                    BufReader::new(commands),
                    &(Arc::new(Mutex::new(output)) as super::Output),
                    Strength::default(),
                );
            });
            EngineSession {
                input: Some(input),
                output: BufReader::new(replies),
                worker: Some(worker),
            }
        }

        fn send(&mut self, command: &str) {
            let input = self.input.as_mut().expect("the session is still open");
            writeln!(input, "{command}").expect("the engine is still listening");
            input.flush().expect("the command reaches the engine");
        }

        fn greeting(&mut self) -> Vec<String> {
            self.send("uci");
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                let read = self
                    .output
                    .read_line(&mut line)
                    .expect("the engine keeps answering");
                assert!(read > 0, "the engine closed before it said uciok");
                let line = line.trim().to_string();
                let done = line == "uciok";
                lines.push(line);
                if done {
                    return lines;
                }
            }
        }

        fn searched(&mut self, position: &str, go: &str) -> Searched {
            self.send(position);
            self.send(go);

            let mut nodes = 0;
            loop {
                let mut line = String::new();
                let read = self
                    .output
                    .read_line(&mut line)
                    .expect("the engine keeps answering");
                assert!(read > 0, "the engine closed before it reported a move");

                if let Some(counted) = info_nodes(&line) {
                    nodes = counted;
                }
                if let Some(best) = line.strip_prefix("bestmove ") {
                    return Searched {
                        nodes,
                        best_move: best
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .to_string(),
                    };
                }
            }
        }
    }

    impl Drop for EngineSession {
        fn drop(&mut self) {
            drop(self.input.take());
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn info_nodes(info: &str) -> Option<u64> {
        let mut words = info.split_whitespace();
        words.find(|word| *word == "nodes")?;
        words.next()?.parse().ok()
    }

    struct Searched {
        nodes: u64,
        best_move: String,
    }

    fn on_clock(wtime_ms: u64, winc_ms: u64) -> GoLimits {
        GoLimits {
            wtime_ms: Some(wtime_ms),
            winc_ms: Some(winc_ms),
            ..GoLimits::default()
        }
    }

    fn asking(depth: u32, movetime_ms: u64) -> GoLimits {
        GoLimits {
            depth: Some(depth),
            movetime_ms: Some(movetime_ms),
            ..GoLimits::default()
        }
    }

    fn forever() -> GoLimits {
        GoLimits {
            infinite: true,
            ..GoLimits::default()
        }
    }

    fn with_slack(slack_cp: i32) -> Strength {
        Strength {
            level: None,
            slack_cp,
        }
    }

    fn at(level: Level) -> Strength {
        Strength {
            level: Some(level),
            slack_cp: 0,
        }
    }

    #[test]
    fn a_go_is_turned_into_the_limits_the_strength_allows() {
        let full = Strength::default();
        let cases = [
            (
                on_clock(60_000, 1_000),
                Color::White,
                full,
                (None, Some(3_000), 0),
                "a comfortable clock yields a share plus the increment",
            ),
            (
                on_clock(500, 3_000),
                Color::White,
                full,
                (None, Some(500 - MOVE_OVERHEAD_MS), 0),
                "the budget never exceeds the time left on the clock",
            ),
            (
                on_clock(50, 0),
                Color::White,
                full,
                (None, Some(1), 0),
                "a nearly flagged clock still gets a moment to move",
            ),
            (
                GoLimits {
                    movetime_ms: Some(700),
                    ..on_clock(500, 3_000)
                },
                Color::White,
                full,
                (None, Some(700), 0),
                "an explicit movetime wins over the clock",
            ),
            (
                GoLimits {
                    btime_ms: Some(600),
                    binc_ms: Some(5_000),
                    ..GoLimits::default()
                },
                Color::Black,
                full,
                (None, Some(600 - MOVE_OVERHEAD_MS), 0),
                "the black side reads the black clock",
            ),
            (
                on_clock(600_000, 0),
                Color::White,
                full,
                (None, Some(20_000), 0),
                "an unrestricted clock budget is a thirtieth of the time left",
            ),
            (
                asking(30, 6_000),
                Color::White,
                full,
                (Some(30), Some(6_000), 0),
                "an unconfigured engine is left at full strength",
            ),
            (
                on_clock(60_000, 0),
                Color::White,
                with_slack(120),
                (None, Some(2_000), 120),
                "the slack option reaches a search on a clock",
            ),
            (
                forever(),
                Color::White,
                with_slack(120),
                (None, None, 120),
                "the slack option reaches an infinite search",
            ),
            (
                asking(30, 6_000),
                Color::White,
                at(Level::Decaf),
                (Some(2), Some(200), 150),
                "a level caps a deeper and longer request down to its own",
            ),
            (
                asking(3, 50),
                Color::White,
                at(Level::Espresso),
                (Some(3), Some(50), 0),
                "a level never raises a request that already asks for less",
            ),
            (
                GoLimits::default(),
                Color::White,
                at(Level::Cappuccino),
                (Some(6), Some(1_000), 0),
                "a level supplies the limits when the go carries none",
            ),
            (
                forever(),
                Color::White,
                at(Level::Latte),
                (Some(4), Some(400), 75),
                "a level bounds an infinite go that would otherwise run forever",
            ),
            (
                on_clock(600_000, 0),
                Color::White,
                at(Level::Latte),
                (Some(4), Some(400), 75),
                "a level caps a clock budget the same way it caps a movetime",
            ),
            (
                GoLimits::default(),
                Color::White,
                Strength {
                    level: Some(Level::Cappuccino),
                    slack_cp: 400,
                },
                (Some(6), Some(1_000), 0),
                "a level owns the slack and overrules the option",
            ),
        ];
        for (go, side, strength, expected, reason) in cases {
            let limits = search_limits(go, side, strength);
            assert_eq!(
                (limits.depth, limits.movetime_ms, limits.slack_cp),
                expected,
                "{go:?} for {side:?} at {strength:?}: {reason}"
            );
        }
    }

    #[test]
    fn the_level_option_reads_every_name_it_advertises_and_refuses_the_rest() {
        let mut cases = vec![
            ("espresso", Ok(Some(Level::Espresso))),
            (" Decaf ", Ok(Some(Level::Decaf))),
            ("none", Ok(None)),
            ("NONE", Ok(None)),
            ("ristretto", Err("ristretto")),
        ];
        let advertised = level_names(" ");
        cases.extend(
            advertised
                .split(' ')
                .map(|name| (name, Ok(name.parse::<Level>().ok()))),
        );
        for (value, expected) in cases {
            assert_eq!(requested_level(value), expected, "value {value:?}");
        }
    }

    #[test]
    fn the_level_flag_is_read_off_the_command_line() {
        let arguments = ["--level".to_string(), "latte".to_string()];
        let cases = [
            (&arguments[..], "--level", Some("latte")),
            (&arguments[..], "--depth", None),
            (&arguments[..1], "--level", None),
        ];
        for (given, flag, expected) in cases {
            assert_eq!(flag_value(given, flag), expected, "{given:?} for {flag}");
        }
    }

    #[test]
    fn a_slack_value_that_makes_no_sense_leaves_the_engine_at_full_strength() {
        let cases = [
            ("0", 0),
            (" 150 ", 150),
            ("-40", 0),
            ("nonsense", 0),
            ("", 0),
            ("999999", MAX_SLACK_CP),
        ];
        for (value, expected) in cases {
            assert_eq!(parsed_slack(value), expected, "value {value:?}");
        }
    }

    #[test]
    fn a_hash_size_is_kept_within_bounds_and_an_unreadable_one_is_refused() {
        let cases = [
            ("64", Some(64)),
            (" 128 ", Some(128)),
            ("0", Some(MIN_HASH_MB)),
            ("999999", Some(MAX_HASH_MB)),
            ("-16", None),
            ("nonsense", None),
            ("", None),
        ];
        for (value, expected) in cases {
            assert_eq!(parsed_hash(value), expected, "value {value:?}");
        }
    }

    #[test]
    fn a_score_reaches_the_protocol_as_a_move_count_or_in_centipawns() {
        let cases = [
            (MATE_SCORE - 3, Score::MateIn(2)),
            (3 - MATE_SCORE, Score::MateIn(-2)),
            (45, Score::Centipawns(45)),
            (-1200, Score::Centipawns(-1200)),
        ];
        for (score_cp, expected) in cases {
            assert_eq!(reported_score(score_cp), expected, "score {score_cp}");
        }
    }

    #[test]
    fn the_node_rate_is_counted_per_second() {
        let cases = [
            (2_000, 500, 4_000, "a half-second run"),
            (
                7,
                0,
                7_000,
                "a run too short to measure does not divide by zero",
            ),
        ];
        for (nodes, elapsed_ms, expected, reason) in cases {
            assert_eq!(
                nodes_per_second(nodes, Duration::from_millis(elapsed_ms)),
                expected,
                "{reason}"
            );
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        Send(&'static str),
        Search(&'static str),
    }

    #[derive(Clone, Copy, Debug)]
    enum Against {
        FewerNodes,
        SameNodes,
        SameMove,
        AnotherMove,
    }

    #[test]
    fn what_a_session_went_through_shows_in_its_next_search() {
        let cases: [(&[Step], &str, Against, &str); 6] = [
            (
                &[Step::Search(SEARCH)],
                SEARCH,
                Against::FewerNodes,
                "the table carries over from one go to the next",
            ),
            (
                &[Step::Search(SEARCH), Step::Send("ucinewgame")],
                SEARCH,
                Against::SameNodes,
                "a new game forgets what the last one filled in",
            ),
            (
                &[
                    Step::Search(SEARCH),
                    Step::Send("setoption name Hash value 16"),
                ],
                SEARCH,
                Against::SameNodes,
                "resizing the table forgets what it held",
            ),
            (
                &[
                    Step::Search(SEARCH),
                    Step::Send("setoption name Hash value nonsense"),
                ],
                SEARCH,
                Against::FewerNodes,
                "an unreadable hash size leaves the table as it was",
            ),
            (
                &[Step::Send("setoption name Slack value 100")],
                "go depth 1",
                Against::AnotherMove,
                "the slack option changes what the engine plays",
            ),
            (
                &[
                    Step::Send("setoption name Slack value nonsense"),
                    Step::Send("setoption name Mocha value 100"),
                ],
                "go depth 1",
                Against::SameMove,
                "a slack value the engine cannot read leaves it at full strength",
            ),
        ];
        for (before, go, against, reason) in cases {
            let fresh = EngineSession::start().searched("position startpos", go);

            let mut engine = EngineSession::start();
            for step in before {
                match *step {
                    Step::Send(command) => engine.send(command),
                    Step::Search(earlier) => {
                        engine.searched("position startpos", earlier);
                    }
                }
            }
            let after = engine.searched("position startpos", go);

            let held = match against {
                Against::FewerNodes => after.nodes < fresh.nodes,
                Against::SameNodes => after.nodes == fresh.nodes,
                Against::SameMove => after.best_move == fresh.best_move,
                Against::AnotherMove => after.best_move != fresh.best_move,
            };
            assert!(
                held,
                "{reason}: {against:?} failed, fresh {} in {} nodes, after {before:?} {} in {} nodes",
                fresh.best_move, fresh.nodes, after.best_move, after.nodes
            );
        }
    }

    #[test]
    fn a_position_the_engine_cannot_read_is_refused_without_taking_it_down() {
        let cases = [
            ("position fen not a position at all", false),
            ("position startpos", true),
        ];
        let mut engine = EngineSession::start();
        for (position, answered) in cases {
            let reply = engine.searched(position, SEARCH);
            assert_eq!(reply.best_move != "0000", answered, "{position:?}");
            assert_eq!(reply.nodes > 0, answered, "{position:?}");
        }
    }

    #[test]
    fn the_handshake_advertises_every_option() {
        let greeting = EngineSession::start().greeting();
        for advertised in [
            "id name chess-engine",
            "option name Hash type spin",
            "option name Slack type spin",
            "option name Level type combo",
        ] {
            assert!(
                greeting.iter().any(|line| line.starts_with(advertised)),
                "the handshake never said {advertised:?}: {greeting:?}"
            );
        }
    }

    #[test]
    fn nodes_are_read_back_out_of_an_info_line() {
        let cases = [
            (
                "info depth 6 score cp 34 nodes 1500000 nps 870000 time 1724",
                Some(1_500_000),
            ),
            ("info depth 6 nodes", None),
            ("info string thinking", None),
        ];
        for (line, expected) in cases {
            assert_eq!(info_nodes(line), expected, "line {line:?}");
        }
    }
}
