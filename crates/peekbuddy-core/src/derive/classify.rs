//! Pattern matching on a closed burst: is it an atomic save, which rename
//! pairs does it hold, and how does it expand when it is not a group?
//!
//! Patterns stay conservative. A false "Saved" group still lists every member
//! and the raw count, so the worst a wrong match does is group too much.

use crate::event::Kind;

/// One event of a burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Step {
    /// The raw event id.
    pub id: u64,
    /// The event kind.
    pub kind: Kind,
    /// Index of the path this event is about, into the burst's paths.
    pub path: usize,
    /// For a paired rename half: the id of the other half.
    pub partner: Option<u64>,
    /// A `RenamedFrom` whose `RenamedTo` has not arrived (yet).
    pub awaiting: bool,
}

/// What a closed burst becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    /// An atomic save of the path with this index; every step is a member.
    Saved { target: usize },
    /// Several events on one path in compact mode; every step is a member.
    Coalesced,
    /// No group: rename pairs become groups, the rest single rows.
    Split,
}

fn is_created(kind: Kind) -> bool {
    kind == Kind::Created
}

fn is_deleted(kind: Kind) -> bool {
    kind == Kind::Deleted
}

/// The rename pairs of the steps: `(from, to)` step indices.
pub(super) fn rename_pairs(steps: &[Step]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for (i, from) in steps.iter().enumerate() {
        if from.kind != Kind::RenamedFrom {
            continue;
        }
        let Some(partner) = from.partner else {
            continue;
        };
        if let Some(j) = steps
            .iter()
            .position(|s| s.id == partner && s.kind == Kind::RenamedTo)
        {
            pairs.push((i, j));
        }
    }
    pairs
}

/// The path a save lands on, if the steps are an atomic save.
///
/// Three patterns, checked in this order:
///
/// 1. *Temp, then rename over the target*: a file is created, then renamed
///    to another name (`Created(t)`, ..., `Renamed(t -> x)`); the target is
///    `x`. This is Word and most "write a temp file" savers.
/// 2. *Backup*: the target is renamed away, a new file is written under its
///    name and the backup is deleted (`Renamed(x -> b)`, `Created(x)`,
///    `Deleted(b)`); the target is `x`.
/// 3. *Delete and create*: a name is deleted and created again
///    (`Deleted(x)`, ..., `Created(x)`); the target is `x`.
///
/// `ReplaceFileW` (temp, backup, rename, delete) shows up as 1 and 2 together
/// and matches 1.
fn save_target(steps: &[Step], pairs: &[(usize, usize)]) -> Option<usize> {
    // The steps are in id order, so "before" and "after" are positions. A
    // slice that starts at a rename half or at the step itself only adds a
    // step that is neither a creation nor a deletion.
    // 1. temp written, then renamed to another name.
    for &(f, t) in pairs {
        let (from, to) = (steps.get(f)?, steps.get(t)?);
        if from.path != to.path
            && steps
                .iter()
                .take(f)
                .any(|s| is_created(s.kind) && s.path == from.path)
        {
            return Some(to.path);
        }
    }
    // 2. target renamed to a backup, rewritten, backup deleted.
    for &(f, t) in pairs {
        let (from, to) = (steps.get(f)?, steps.get(t)?);
        if from.path == to.path {
            continue;
        }
        let rewritten = steps
            .iter()
            .skip(t)
            .any(|s| is_created(s.kind) && s.path == from.path);
        let backup_gone = steps
            .iter()
            .skip(t)
            .any(|s| is_deleted(s.kind) && s.path == to.path);
        if rewritten && backup_gone {
            return Some(from.path);
        }
    }
    // 3. delete and create of the same name.
    for (i, d) in steps.iter().enumerate().filter(|(_, s)| is_deleted(s.kind)) {
        if steps
            .iter()
            .skip(i)
            .any(|c| is_created(c.kind) && c.path == d.path)
        {
            return Some(d.path);
        }
    }
    None
}

/// Classifies a burst.
pub(super) fn classify(steps: &[Step], compact: bool) -> Outcome {
    let pairs = rename_pairs(steps);
    if let Some(target) = save_target(steps, &pairs) {
        return Outcome::Saved { target };
    }
    let unmatched = steps.iter().any(|s| s.awaiting);
    if compact && steps.len() >= 2 && pairs.is_empty() && !unmatched {
        return Outcome::Coalesced;
    }
    Outcome::Split
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: u64, kind: Kind, path: usize) -> Step {
        Step {
            id,
            kind,
            path,
            partner: None,
            awaiting: false,
        }
    }

    fn pair(from_id: u64, to_id: u64, from_path: usize, to_path: usize) -> [Step; 2] {
        [
            Step {
                partner: Some(to_id),
                ..step(from_id, Kind::RenamedFrom, from_path)
            },
            Step {
                partner: Some(from_id),
                ..step(to_id, Kind::RenamedTo, to_path)
            },
        ]
    }

    #[test]
    fn temp_write_then_rename() {
        let mut steps = vec![step(1, Kind::Created, 0), step(2, Kind::Modified, 0)];
        steps.extend(pair(3, 4, 0, 1));
        assert_eq!(classify(&steps, false), Outcome::Saved { target: 1 });
        // Without the creation it is only a rename.
        let steps = pair(3, 4, 0, 1);
        assert_eq!(classify(&steps, false), Outcome::Split);
        // Created after the rename starts is not a temp write.
        let mut steps = pair(3, 4, 0, 1).to_vec();
        steps.push(step(5, Kind::Created, 0));
        assert_eq!(classify(&steps, false), Outcome::Split);
    }

    #[test]
    fn backup_pattern() {
        // x -> b, new x, delete b.
        let mut steps = pair(1, 2, 0, 1).to_vec();
        steps.push(step(3, Kind::Created, 0));
        steps.push(step(4, Kind::Modified, 0));
        steps.push(step(5, Kind::Deleted, 1));
        assert_eq!(classify(&steps, false), Outcome::Saved { target: 0 });
        // Backup never deleted: not a save.
        steps.pop();
        assert_eq!(classify(&steps, false), Outcome::Split);
        // New file never written: not a save.
        let mut steps = pair(1, 2, 0, 1).to_vec();
        steps.push(step(5, Kind::Deleted, 1));
        assert_eq!(classify(&steps, false), Outcome::Split);
    }

    #[test]
    fn delete_and_create() {
        let steps = [
            step(1, Kind::Deleted, 0),
            step(2, Kind::Created, 0),
            step(3, Kind::Modified, 0),
        ];
        assert_eq!(classify(&steps, false), Outcome::Saved { target: 0 });
        // Create before delete is not a replacement.
        let steps = [step(1, Kind::Created, 0), step(2, Kind::Deleted, 0)];
        assert_eq!(classify(&steps, false), Outcome::Split);
        // Different names are two events.
        let steps = [step(1, Kind::Deleted, 0), step(2, Kind::Created, 1)];
        assert_eq!(classify(&steps, false), Outcome::Split);
    }

    #[test]
    fn replace_file_matches_the_temp_pattern() {
        // C(t) M(t), R(x -> b), R(t -> x), D(b)
        let mut steps = vec![step(1, Kind::Created, 0), step(2, Kind::Modified, 0)];
        steps.extend(pair(3, 4, 1, 2)); // x -> b
        steps.extend(pair(5, 6, 0, 1)); // t -> x
        steps.push(step(7, Kind::Deleted, 2));
        assert_eq!(classify(&steps, false), Outcome::Saved { target: 1 });
    }

    #[test]
    fn coalescing_only_in_compact_mode_without_renames() {
        let steps = [step(1, Kind::Modified, 0), step(2, Kind::Modified, 0)];
        assert_eq!(classify(&steps, true), Outcome::Coalesced);
        assert_eq!(classify(&steps, false), Outcome::Split);
        assert_eq!(classify(&steps[..1], true), Outcome::Split);
        let mut with_pair = steps.to_vec();
        with_pair.extend(pair(3, 4, 0, 1));
        assert_eq!(classify(&with_pair, true), Outcome::Split);
        let mut waiting = steps.to_vec();
        waiting.push(Step {
            awaiting: true,
            ..step(3, Kind::RenamedFrom, 0)
        });
        assert_eq!(classify(&waiting, true), Outcome::Split);
    }

    #[test]
    fn pairs_need_both_halves() {
        let lone = [Step {
            partner: Some(9),
            ..step(1, Kind::RenamedFrom, 0)
        }];
        assert_eq!(rename_pairs(&lone), Vec::<(usize, usize)>::new());
        let steps = pair(1, 2, 0, 1);
        assert_eq!(rename_pairs(&steps), vec![(0, 1)]);
        // A pair whose "to" is missing from the burst is not a pair.
        assert_eq!(rename_pairs(&steps[..1]), Vec::<(usize, usize)>::new());
    }
}
