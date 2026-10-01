use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UciCommand {
    Uci,
    IsReady,
    NewGame,
    SetOption(EngineOption),
    Position(Position),
    Go(GoLimits),
    GoPerft(u32),
    Bench { depth: Option<u32> },
    Stop,
    Quit,
    Unknown(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOption {
    pub name: String,
    pub value: String,
}

impl EngineOption {
    pub fn named(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Position {
    pub fen: Option<String>,
    pub moves: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoLimits {
    pub movetime_ms: Option<u64>,
    pub depth: Option<u32>,
    pub nodes: Option<u64>,
    pub wtime_ms: Option<u64>,
    pub btime_ms: Option<u64>,
    pub winc_ms: Option<u64>,
    pub binc_ms: Option<u64>,
    pub infinite: bool,
}

pub fn parse_command(line: &str) -> UciCommand {
    let mut words = line.split_whitespace();
    match words.next() {
        Some("uci") => UciCommand::Uci,
        Some("isready") => UciCommand::IsReady,
        Some("ucinewgame") => UciCommand::NewGame,
        Some("setoption") => parse_setoption(words),
        Some("position") => parse_position(words),
        Some("go") => parse_go(words),
        Some("bench") => UciCommand::Bench {
            depth: next_number(&mut words),
        },
        Some("stop") => UciCommand::Stop,
        Some("quit") => UciCommand::Quit,
        _ => UciCommand::Unknown(line.to_string()),
    }
}

fn parse_setoption<'a>(words: impl Iterator<Item = &'a str>) -> UciCommand {
    let mut name = Vec::new();
    let mut value = Vec::new();
    let mut in_value = false;
    for word in words {
        match word {
            "name" if !in_value && name.is_empty() => {}
            "value" if !in_value => in_value = true,
            other if in_value => value.push(other),
            other => name.push(other),
        }
    }
    UciCommand::SetOption(EngineOption {
        name: name.join(" "),
        value: value.join(" "),
    })
}

fn parse_position<'a>(words: impl Iterator<Item = &'a str>) -> UciCommand {
    let mut fen_fields = Vec::new();
    let mut moves = Vec::new();
    let mut in_moves = false;
    let mut startpos = false;
    for word in words {
        match word {
            "moves" if !in_moves => in_moves = true,
            "startpos" if !in_moves => startpos = true,
            "fen" if !in_moves => {}
            other if in_moves => moves.push(other.to_string()),
            other => fen_fields.push(other),
        }
    }
    let fen = (!startpos && !fen_fields.is_empty()).then(|| fen_fields.join(" "));
    UciCommand::Position(Position { fen, moves })
}

fn parse_go<'a>(mut words: impl Iterator<Item = &'a str>) -> UciCommand {
    let mut limits = GoLimits::default();
    while let Some(word) = words.next() {
        match word {
            "perft" => return UciCommand::GoPerft(next_number(&mut words).unwrap_or(1)),
            "movetime" => limits.movetime_ms = next_number(&mut words),
            "depth" => limits.depth = next_number(&mut words),
            "nodes" => limits.nodes = next_number(&mut words),
            "wtime" => limits.wtime_ms = next_number(&mut words),
            "btime" => limits.btime_ms = next_number(&mut words),
            "winc" => limits.winc_ms = next_number(&mut words),
            "binc" => limits.binc_ms = next_number(&mut words),
            "infinite" => limits.infinite = true,
            _ => {}
        }
    }
    UciCommand::Go(limits)
}

fn next_number<'a, T: FromStr>(words: &mut impl Iterator<Item = &'a str>) -> Option<T> {
    words.next().and_then(|word| word.parse().ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Score {
    Centipawns(i32),
    MateIn(i32),
}

impl std::fmt::Display for Score {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Score::Centipawns(value) => write!(formatter, "cp {value}"),
            Score::MateIn(moves) => write!(formatter, "mate {moves}"),
        }
    }
}

pub fn format_info(
    depth: u32,
    score: Score,
    nodes: u64,
    nps: u64,
    time_ms: u64,
    pv: &[String],
) -> String {
    let mut line =
        format!("info depth {depth} score {score} nodes {nodes} nps {nps} time {time_ms}");
    if !pv.is_empty() {
        line.push_str(" pv ");
        line.push_str(&pv.join(" "));
    }
    line
}

pub fn format_bestmove(uci: &str) -> String {
    format!("bestmove {uci}")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Decaf,
    Latte,
    Cappuccino,
    Espresso,
}

pub const LEVELS: [Level; 4] = [
    Level::Decaf,
    Level::Latte,
    Level::Cappuccino,
    Level::Espresso,
];

impl Level {
    pub fn depth(self) -> u32 {
        match self {
            Level::Decaf => 2,
            Level::Latte => 4,
            Level::Cappuccino => 6,
            Level::Espresso => 10,
        }
    }

    pub fn movetime_ms(self) -> u64 {
        match self {
            Level::Decaf => 200,
            Level::Latte => 400,
            Level::Cappuccino => 1000,
            Level::Espresso => 5000,
        }
    }

    pub fn slack_cp(self) -> i32 {
        match self {
            Level::Decaf => 150,
            Level::Latte => 75,
            Level::Cappuccino | Level::Espresso => 0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Decaf => "decaf",
            Level::Latte => "latte",
            Level::Cappuccino => "cappuccino",
            Level::Espresso => "espresso",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct UnknownLevel;

impl FromStr for Level {
    type Err = UnknownLevel;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let name = name.trim().to_ascii_lowercase();
        LEVELS
            .into_iter()
            .find(|level| level.as_str() == name)
            .ok_or(UnknownLevel)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EngineOption, GoLimits, LEVELS, Level, Position, Score, UciCommand, format_info,
        parse_command,
    };

    const LONE_KING_FEN: &str = "8/8/8/8/8/8/8/4K3 w - - 0 1";

    fn moves(list: &[&str]) -> Vec<String> {
        list.iter().map(|entry| entry.to_string()).collect()
    }

    fn option(name: &str, value: impl std::fmt::Display) -> EngineOption {
        EngineOption {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    fn startpos(moves: &[String]) -> Position {
        Position {
            fen: None,
            moves: moves.to_vec(),
        }
    }

    fn from_fen(fen: &str, moves: &[String]) -> Position {
        Position {
            fen: Some(fen.to_string()),
            moves: moves.to_vec(),
        }
    }

    fn flag(name: &str) -> EngineOption {
        EngineOption {
            name: name.to_string(),
            value: String::new(),
        }
    }

    #[test]
    fn a_command_line_is_parsed_into_what_it_asks_for() {
        let cases = [
            (
                "position startpos moves e2e4 e7e5",
                UciCommand::Position(startpos(&moves(&["e2e4", "e7e5"]))),
            ),
            (
                "position fen 8/8/8/8/8/8/8/4K3 w - - 0 1 moves e1e2",
                UciCommand::Position(from_fen(LONE_KING_FEN, &moves(&["e1e2"]))),
            ),
            (
                "go movetime 500 depth 6",
                UciCommand::Go(GoLimits {
                    movetime_ms: Some(500),
                    depth: Some(6),
                    ..GoLimits::default()
                }),
            ),
            ("go perft 3", UciCommand::GoPerft(3)),
            ("bench", UciCommand::Bench { depth: None }),
            ("bench 7", UciCommand::Bench { depth: Some(7) }),
            (
                "setoption name Slack value 120",
                UciCommand::SetOption(option("Slack", 120)),
            ),
            (
                "setoption name Skill Level value 3",
                UciCommand::SetOption(option("Skill Level", 3)),
            ),
            (
                "setoption name Ponder",
                UciCommand::SetOption(flag("Ponder")),
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_command(line), expected, "line {line:?}");
        }
    }

    #[test]
    fn an_info_line_is_written_the_way_a_gui_expects_it() {
        let cases = [
            (
                format_info(
                    6,
                    Score::Centipawns(34),
                    1_500_000,
                    870_000,
                    1724,
                    &moves(&["e2e4", "e7e5"]),
                ),
                "info depth 6 score cp 34 nodes 1500000 nps 870000 time 1724 pv e2e4 e7e5",
            ),
            (
                format_info(9, Score::MateIn(-3), 42, 42, 1, &[]),
                "info depth 9 score mate -3 nodes 42 nps 42 time 1",
            ),
        ];
        for (written, expected) in cases {
            assert_eq!(written, expected);
        }
    }

    #[test]
    fn an_option_is_matched_by_name_whatever_the_case() {
        let option = option("Slack", 100);
        let cases = [("slack", true), ("SLACK", true), ("slack cp", false)];
        for (name, expected) in cases {
            assert_eq!(option.named(name), expected, "name {name:?}");
        }
    }

    #[test]
    fn the_levels_climb_in_depth_and_think_time_and_give_up_their_slack() {
        for pair in LEVELS.windows(2) {
            let (weaker, stronger) = (pair[0], pair[1]);
            assert!(
                weaker.depth() < stronger.depth(),
                "{weaker:?} to {stronger:?}"
            );
            assert!(
                weaker.movetime_ms() < stronger.movetime_ms(),
                "{weaker:?} to {stronger:?}"
            );
            assert!(
                weaker.slack_cp() >= stronger.slack_cp(),
                "{weaker:?} to {stronger:?}"
            );
        }
    }

    #[test]
    fn a_level_is_read_only_from_a_name_it_carries() {
        let mut cases: Vec<(&str, Option<Level>)> = LEVELS
            .iter()
            .map(|level| (level.as_str(), Some(*level)))
            .collect();
        cases.extend([
            ("  ESPRESSO ", Some(Level::Espresso)),
            ("ristretto", None),
            ("", None),
        ]);
        for (name, expected) in cases {
            assert_eq!(name.parse::<Level>().ok(), expected, "name {name:?}");
        }
    }
}
