use crate::movegen::Move;

pub(crate) const DEFAULT_HASH_MB: usize = 16;
pub(crate) const MIN_HASH_MB: usize = 1;
pub(crate) const MAX_HASH_MB: usize = 4096;
const ENTRY_BYTES: usize = std::mem::size_of::<Entry>();

fn entry_count(megabytes: usize) -> usize {
    let requested = (megabytes.clamp(MIN_HASH_MB, MAX_HASH_MB) << 20) / ENTRY_BYTES;
    if requested.is_power_of_two() {
        requested
    } else {
        requested.next_power_of_two() >> 1
    }
}

const BOUND_MASK: u8 = 0b11;
const GENERATION_SHIFT: u32 = 2;
const GENERATION_LIMIT: u8 = 1 << (8 - GENERATION_SHIFT);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Bound {
    Exact = 1,
    Lower = 2,
    Upper = 3,
}

impl Bound {
    fn from_flags(flags: u8) -> Option<Self> {
        match flags & BOUND_MASK {
            1 => Some(Bound::Exact),
            2 => Some(Bound::Lower),
            3 => Some(Bound::Upper),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Entry {
    key: u64,
    score: i32,
    mv: Move,
    depth: u8,
    flags: u8,
}

impl Entry {
    fn generation(&self) -> u8 {
        self.flags >> GENERATION_SHIFT
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub(crate) score: i32,
    pub(crate) depth: u32,
    pub(crate) bound: Bound,
    pub(crate) mv: Option<Move>,
}

pub(crate) struct TranspositionTable {
    entries: Vec<Entry>,
    mask: usize,
    generation: u8,
}

impl TranspositionTable {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_megabytes(DEFAULT_HASH_MB)
    }

    pub(crate) fn with_megabytes(megabytes: usize) -> Self {
        let count = entry_count(megabytes);
        TranspositionTable {
            entries: vec![Entry::default(); count],
            mask: count - 1,
            generation: 0,
        }
    }

    pub(crate) fn resize(&mut self, megabytes: usize) {
        self.entries = Vec::new();
        *self = Self::with_megabytes(megabytes);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.fill(Entry::default());
        self.generation = 0;
    }

    pub(crate) fn bump_generation(&mut self) {
        self.generation = (self.generation + 1) % GENERATION_LIMIT;
    }

    pub(crate) fn probe(&self, key: u64) -> Option<Hit> {
        let entry = &self.entries[key as usize & self.mask];
        if entry.key != key {
            return None;
        }
        Some(Hit {
            score: entry.score,
            depth: entry.depth as u32,
            bound: Bound::from_flags(entry.flags)?,
            mv: (entry.mv != Move::NONE).then_some(entry.mv),
        })
    }

    pub(crate) fn store(&mut self, key: u64, depth: u32, score: i32, bound: Bound, mv: Move) {
        let slot = &mut self.entries[key as usize & self.mask];
        let same_position = slot.key == key;
        let stale = slot.generation() != self.generation;
        if Bound::from_flags(slot.flags).is_some()
            && same_position
            && !stale
            && (depth as u8) < slot.depth
        {
            return;
        }
        let kept = if mv == Move::NONE && same_position {
            slot.mv
        } else {
            mv
        };
        *slot = Entry {
            key,
            score,
            mv: kept,
            depth: depth.min(u8::MAX as u32) as u8,
            flags: bound as u8 | self.generation << GENERATION_SHIFT,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{Bound, TranspositionTable};
    use crate::movegen::Move;

    const KEY: u64 = 0x0f0f_0f0f_0f0f_0f0f;

    #[derive(Clone, Copy)]
    enum Step {
        Store(u64, u32, i32, Bound, Move),
        NewSearch(usize),
        Clear,
    }

    type Probed = (u32, i32, Bound, Option<Move>);

    fn quiet(origin: usize, target: usize) -> Move {
        Move::new(origin, target, None)
    }

    #[test]
    fn a_probe_answers_with_what_the_table_kept() {
        let mv = quiet(12, 28);
        let cases: [(&str, Vec<Step>, u64, Option<Probed>); 10] = [
            ("an untouched key misses", vec![], KEY, None),
            (
                "a stored entry comes back from a probe",
                vec![Step::Store(KEY, 5, 42, Bound::Exact, mv)],
                KEY,
                Some((5, 42, Bound::Exact, Some(mv))),
            ),
            (
                "an entry stored without a move reads back as no move",
                vec![Step::Store(KEY, 5, 42, Bound::Exact, Move::NONE)],
                KEY,
                Some((5, 42, Bound::Exact, None)),
            ),
            (
                "a colliding key does not answer for another position",
                vec![Step::Store(0xff, 5, 42, Bound::Exact, Move::NONE)],
                0xff ^ (1 << 63),
                None,
            ),
            (
                "a shallower result does not evict a deeper one",
                vec![
                    Step::Store(KEY, 8, 42, Bound::Exact, Move::NONE),
                    Step::Store(KEY, 3, -99, Bound::Upper, Move::NONE),
                ],
                KEY,
                Some((8, 42, Bound::Exact, None)),
            ),
            (
                "a new search may overwrite a deeper entry from the previous one",
                vec![
                    Step::Store(KEY, 8, 42, Bound::Exact, Move::NONE),
                    Step::NewSearch(1),
                    Step::Store(KEY, 3, -99, Bound::Upper, Move::NONE),
                ],
                KEY,
                Some((3, -99, Bound::Upper, None)),
            ),
            (
                "a boundless store keeps the move of the same position",
                vec![
                    Step::Store(KEY, 8, 42, Bound::Exact, mv),
                    Step::Store(KEY, 9, -99, Bound::Upper, Move::NONE),
                ],
                KEY,
                Some((9, -99, Bound::Upper, Some(mv))),
            ),
            (
                "clearing forgets everything",
                vec![
                    Step::Store(KEY, 8, 42, Bound::Exact, Move::NONE),
                    Step::Clear,
                ],
                KEY,
                None,
            ),
            (
                "the generation counter cycles without overflowing",
                vec![
                    Step::NewSearch(1_000),
                    Step::Store(KEY, 8, 42, Bound::Exact, Move::NONE),
                ],
                KEY,
                Some((8, 42, Bound::Exact, None)),
            ),
            (
                "a store from an older search still answers after a new one starts",
                vec![
                    Step::Store(KEY, 8, 42, Bound::Exact, mv),
                    Step::NewSearch(1),
                ],
                KEY,
                Some((8, 42, Bound::Exact, Some(mv))),
            ),
        ];
        for (reason, steps, probed, expected) in cases {
            let mut table = TranspositionTable::new();
            for step in steps {
                match step {
                    Step::Store(key, depth, score, bound, mv) => {
                        table.store(key, depth, score, bound, mv)
                    }
                    Step::NewSearch(times) => {
                        for _ in 0..times {
                            table.bump_generation();
                        }
                    }
                    Step::Clear => table.clear(),
                }
            }
            let hit = table
                .probe(probed)
                .map(|hit| (hit.depth, hit.score, hit.bound, hit.mv));
            assert_eq!(hit, expected, "{reason}");
        }
    }

    #[test]
    fn four_entries_share_a_cache_line() {
        assert_eq!(super::ENTRY_BYTES, 16);
    }

    #[test]
    fn the_table_holds_as_many_entries_as_fit_in_the_asked_megabytes() {
        let cases = [
            (super::DEFAULT_HASH_MB, 1 << 20),
            (1, 1 << 16),
            (24, 1 << 20),
            (0, 1 << 16),
            (usize::MAX, super::MAX_HASH_MB << 16),
        ];
        for (megabytes, expected) in cases {
            assert_eq!(super::entry_count(megabytes), expected, "{megabytes} MB");
        }
    }

    #[test]
    fn resizing_forgets_what_the_table_held() {
        let mut table = TranspositionTable::new();
        table.store(KEY, 8, 42, Bound::Exact, Move::NONE);
        table.resize(1);
        assert_eq!(table.entries.len(), 1 << 16);
        assert!(table.probe(KEY).is_none());
    }
}
