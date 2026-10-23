//! Named workload profiles (referenced by `perf/budgets.toml`) and the
//! achieved-rate verdict.

use crate::ops::OpKind;

/// How fast a profile drives operations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rate {
    /// A steady target of raw events per second for `seconds`.
    Steady {
        /// Raw events per second.
        events_per_s: u64,
        /// Run length.
        seconds: u64,
    },
    /// As fast as possible until `events` raw events are produced; the
    /// target is `events / within_s` per second.
    Burst {
        /// Raw events to produce.
        events: u64,
        /// Seconds they should take at most.
        within_s: u64,
    },
}

impl Rate {
    /// Target raw events per second.
    #[must_use]
    pub fn target_per_s(self) -> u64 {
        match self {
            Self::Steady { events_per_s, .. } => events_per_s,
            Self::Burst { events, within_s } => events / within_s.max(1),
        }
    }
}

/// Name styles a profile draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Names {
    /// ASCII only (perf profiles: names do not change the measurement).
    Ascii,
    /// ASCII, CJK, combining marks, emoji, lone surrogates and long names
    /// that push paths past 260 characters.
    Exotic,
}

/// A named workload.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Name used by `--profile` and `perf/budgets.toml`.
    pub name: &'static str,
    /// Relative weight of each operation class; classes not listed never
    /// run.
    pub mix: &'static [(OpKind, u32)],
    /// Speed target.
    pub rate: Rate,
    /// Name styles.
    pub names: Names,
}

/// The perf mix: file churn as editors and build tools produce it.
const PERF_MIX: &[(OpKind, u32)] = &[
    (OpKind::CreateFile, 20),
    (OpKind::Write, 30),
    (OpKind::Append, 15),
    (OpKind::Rename, 10),
    (OpKind::Delete, 15),
    (OpKind::CreateDir, 5),
    (OpKind::SetTimes, 5),
];

/// Every class with the same weight.
const EDGE_MIX: &[(OpKind, u32)] = &[
    (OpKind::CreateFile, 1),
    (OpKind::Write, 1),
    (OpKind::Append, 1),
    (OpKind::Truncate, 1),
    (OpKind::SetSize, 1),
    (OpKind::Rename, 1),
    (OpKind::RenameChain, 1),
    (OpKind::Move, 1),
    (OpKind::Delete, 1),
    (OpKind::CreateDir, 1),
    (OpKind::DeleteTree, 1),
    (OpKind::SetAttributes, 1),
    (OpKind::SetTimes, 1),
    (OpKind::HardLink, 1),
    (OpKind::DeleteLink, 1),
    (OpKind::CaseRename, 1),
    (OpKind::Junction, 1),
];

/// All named profiles.
pub const PROFILES: &[Profile] = &[
    Profile {
        name: "burst-100k",
        mix: PERF_MIX,
        rate: Rate::Burst {
            events: 1_000_000,
            within_s: 10,
        },
        names: Names::Ascii,
    },
    Profile {
        name: "sustained-20k",
        mix: PERF_MIX,
        rate: Rate::Steady {
            events_per_s: 20_000,
            seconds: 60,
        },
        names: Names::Ascii,
    },
    Profile {
        name: "latency-10k",
        mix: PERF_MIX,
        rate: Rate::Steady {
            events_per_s: 10_000,
            seconds: 30,
        },
        names: Names::Ascii,
    },
    Profile {
        name: "edge-mix",
        mix: EDGE_MIX,
        rate: Rate::Steady {
            events_per_s: 200,
            seconds: 10,
        },
        names: Names::Exotic,
    },
];

/// Looks up a profile by name.
#[must_use]
pub fn profile(name: &str) -> Option<&'static Profile> {
    PROFILES.iter().find(|profile| profile.name == name)
}

/// Whether a run reached its profile's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The achieved raw event rate met the target.
    Exercised,
    /// The generator was too slow: the run says nothing about the budget
    /// and must not count as passed.
    NotExercised,
}

/// Raw events per second achieved by `events` raw events in
/// `elapsed_100ns` (FILETIME ticks).
#[must_use]
pub fn achieved_per_s(events: u64, elapsed_100ns: u64) -> u64 {
    if elapsed_100ns == 0 {
        return 0;
    }
    let rate = u128::from(events) * 10_000_000 / u128::from(elapsed_100ns);
    u64::try_from(rate).unwrap_or(u64::MAX)
}

/// The verdict for a run that achieved `achieved_per_s` against `rate`.
#[must_use]
pub fn verdict(rate: Rate, achieved_per_s: u64) -> Verdict {
    if achieved_per_s >= rate.target_per_s() {
        Verdict::Exercised
    } else {
        Verdict::NotExercised
    }
}

#[cfg(test)]
mod tests {
    use super::{EDGE_MIX, Names, PROFILES, Rate, Verdict, achieved_per_s, profile, verdict};
    use crate::ops::OpKind;

    #[test]
    fn profiles_are_found_by_name() {
        for name in ["burst-100k", "sustained-20k", "latency-10k", "edge-mix"] {
            assert_eq!(profile(name).map(|p| p.name), Some(name));
        }
        assert!(profile("nope").is_none());
        assert_eq!(PROFILES.len(), 4);
    }

    #[test]
    fn targets_match_the_names() {
        let target = |name| profile(name).unwrap().rate.target_per_s();
        assert_eq!(target("burst-100k"), 100_000);
        assert_eq!(target("sustained-20k"), 20_000);
        assert_eq!(target("latency-10k"), 10_000);
    }

    #[test]
    fn edge_mix_covers_every_class_with_exotic_names() {
        let mut classes: Vec<OpKind> = EDGE_MIX.iter().map(|(kind, _)| *kind).collect();
        classes.sort();
        assert_eq!(classes, OpKind::ALL);
        assert_eq!(profile("edge-mix").unwrap().names, Names::Exotic);
    }

    #[test]
    fn achieved_rate_and_verdict() {
        // 50,000 events in 0.5 s.
        assert_eq!(achieved_per_s(50_000, 5_000_000), 100_000);
        assert_eq!(achieved_per_s(1, 0), 0);
        let steady = Rate::Steady {
            events_per_s: 100,
            seconds: 1,
        };
        assert_eq!(verdict(steady, 100), Verdict::Exercised);
        assert_eq!(verdict(steady, 99), Verdict::NotExercised);
        let burst = Rate::Burst {
            events: 10,
            within_s: 0,
        };
        assert_eq!(burst.target_per_s(), 10);
    }
}
