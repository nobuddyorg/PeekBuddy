//! Which link targets an opt-in `follow_link_target` watch adds (#265, F18).
//!
//! A link in a watched tree is logged as the link object and never followed.
//! A watch that asks for it also watches where a directory link points, as a
//! root of its own. Two links to one directory must not watch it twice, and a
//! link back into a watched tree (or to one of its parents) must not make a
//! loop, so a target is admitted once by its identity (volume serial and
//! file id, whichever path reaches it) and only when it does not overlap a
//! root that is watched already.

use std::collections::HashSet;

/// The file identity of a target: volume serial number and 128-bit file id.
pub type Identity = (u64, u128);

/// What happens to a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Watch it.
    New,
    /// The same directory is watched already, reached by another path.
    Duplicate,
    /// It is inside a watched root, or contains one: events would come twice
    /// or loop.
    Overlaps,
}

/// What a discovery pass saw, for the log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Targets added as roots.
    pub followed: u32,
    /// Targets reached by a second link.
    pub duplicates: u32,
    /// Targets inside or around a watched root.
    pub overlapping: u32,
    /// Links that could not be resolved or opened (the compatibility
    /// junctions of a user profile deny access by design).
    pub unreadable: u32,
}

/// The roots and identities seen so far.
#[derive(Debug, Default)]
pub struct LinkTargets {
    seen: HashSet<Identity>,
    roots: Vec<String>,
    report: Report,
}

/// A path folded for comparison: no `\\?\` prefix, no trailing separator,
/// upper case.
fn folded(path: &[u16]) -> String {
    let text = String::from_utf16_lossy(path);
    let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
    plain.trim_end_matches('\\').to_uppercase()
}

/// Whether `inner` is `outer` or below it.
fn within(inner: &str, outer: &str) -> bool {
    inner == outer
        || inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.starts_with('\\'))
}

impl LinkTargets {
    /// No roots yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A target (or a configured root) with `identity` at `path`: whether it
    /// is to be watched. A root that is configured is admitted the same way,
    /// so a link to it is a duplicate.
    pub fn admit(&mut self, identity: Identity, path: &[u16]) -> Admit {
        let path = folded(path);
        let verdict = if self.seen.contains(&identity) {
            Admit::Duplicate
        } else if self
            .roots
            .iter()
            .any(|root| within(&path, root) || within(root, &path))
        {
            Admit::Overlaps
        } else {
            Admit::New
        };
        match verdict {
            Admit::New => {
                self.seen.insert(identity);
                self.roots.push(path);
                self.report.followed = self.report.followed.saturating_add(1);
            }
            Admit::Duplicate => {
                self.report.duplicates = self.report.duplicates.saturating_add(1);
            }
            Admit::Overlaps => {
                self.report.overlapping = self.report.overlapping.saturating_add(1);
            }
        }
        verdict
    }

    /// A root that is watched already (configured by the user): it counts
    /// for identity and overlap, not as a followed target.
    pub fn claim(&mut self, identity: Identity, path: &[u16]) {
        self.seen.insert(identity);
        self.roots.push(folded(path));
    }

    /// A link that could not be resolved.
    pub fn unreadable(&mut self) {
        self.report.unreadable = self.report.unreadable.saturating_add(1);
    }

    /// What was seen.
    #[must_use]
    pub fn report(&self) -> Report {
        self.report
    }
}

#[cfg(test)]
mod tests {
    use super::{Admit, LinkTargets, Report};

    fn w(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn a_target_reached_by_two_links_is_admitted_once() {
        let mut targets = LinkTargets::new();
        targets.claim((7, 1), &w(r"C:\tree"));
        assert_eq!(targets.admit((7, 5), &w(r"C:\other\a")), Admit::New);
        assert_eq!(targets.admit((7, 5), &w(r"C:\other\a")), Admit::Duplicate);
        // The same directory under another name (a second link, a short name).
        assert_eq!(targets.admit((7, 5), &w(r"C:\OTHER~1\A")), Admit::Duplicate);
        assert_eq!(
            targets.report(),
            Report {
                followed: 1,
                duplicates: 2,
                ..Report::default()
            }
        );
    }

    #[test]
    fn a_link_to_a_configured_root_is_a_duplicate() {
        let mut targets = LinkTargets::new();
        targets.claim((7, 1), &w(r"C:\tree"));
        assert_eq!(targets.admit((7, 1), &w(r"\\?\C:\tree")), Admit::Duplicate);
    }

    #[test]
    fn the_same_file_id_on_another_volume_is_another_directory() {
        let mut targets = LinkTargets::new();
        assert_eq!(targets.admit((7, 5), &w(r"C:\a")), Admit::New);
        assert_eq!(targets.admit((8, 5), &w(r"D:\a")), Admit::New);
    }

    #[test]
    fn a_target_inside_or_around_a_root_would_loop() {
        let mut targets = LinkTargets::new();
        targets.claim((7, 1), &w(r"C:\tree"));
        // Inside the watched tree: events would come twice.
        assert_eq!(targets.admit((7, 2), &w(r"C:\tree\sub")), Admit::Overlaps);
        // Around it: a link to a parent loops through the root.
        assert_eq!(targets.admit((7, 3), &w(r"C:\")), Admit::Overlaps);
        assert_eq!(targets.admit((7, 4), &w(r"c:\TREE")), Admit::Overlaps);
        assert_eq!(targets.report().overlapping, 3);
    }

    #[test]
    fn a_name_that_only_starts_alike_is_not_inside() {
        let mut targets = LinkTargets::new();
        targets.claim((7, 1), &w(r"C:\tree"));
        assert_eq!(targets.admit((7, 2), &w(r"C:\tree2")), Admit::New);
        assert_eq!(targets.admit((7, 3), &w(r"C:\tre")), Admit::New);
    }

    #[test]
    fn a_trailing_separator_does_not_matter() {
        let mut targets = LinkTargets::new();
        targets.claim((7, 1), &w(r"C:\tree\"));
        assert_eq!(targets.admit((7, 2), &w(r"C:\tree\sub")), Admit::Overlaps);
    }

    #[test]
    fn unreadable_links_are_counted() {
        let mut targets = LinkTargets::new();
        targets.unreadable();
        targets.unreadable();
        assert_eq!(targets.report().unreadable, 2);
        assert_eq!(targets.report().followed, 0);
    }
}
