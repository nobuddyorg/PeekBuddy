//! The rule evaluator (#81): runs for every event on the capture side.
//!
//! [`Evaluator::compile`] turns the rules of the settings into matchers and
//! fixed-size counters; [`Evaluator::on_event`] then allocates nothing (F10):
//! the glob is a precompiled [`FilterSet`] evaluated with the thread's
//! [`Scratch`], the counter of a rule is a ring of its last `threshold`
//! match times, allocated at compile.
//!
//! The clock is the event's own time (F6: no timer, and a test can drive it).
//! A rule fires when `threshold` matches fall within `window_s`; the matches
//! that follow within `cooldown_s` are only counted, and the count is
//! reported with the next firing ("N more matches"). Rule evaluation never
//! changes an event.

use crate::event::Event;
use crate::filter::{FilterSet, KindSet, Scratch};
use crate::name::WideStr;

use super::{Rule, glob_set, kind_set};

/// FILETIME ticks per second.
const SECOND: u64 = 10_000_000;

/// A rule that fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fired {
    /// The index of the rule in the settings.
    pub rule: usize,
    /// The event time of the match that completed the threshold.
    pub ts: u64,
    /// Matches counted by the firing (the threshold).
    pub matches: u32,
    /// Matches that came during the cooldown since the last firing.
    pub suppressed: u32,
}

struct Compiled {
    enabled: bool,
    /// `None`: every watch. Otherwise indexed by `RootIdx`.
    roots: Option<Box<[bool]>>,
    glob: Option<FilterSet>,
    kinds: KindSet,
    window: u64,
    cooldown: u64,
    /// The last `ring.len()` match times; `filled` of them are valid.
    ring: Box<[u64]>,
    head: usize,
    filled: usize,
    /// No firing before this time.
    quiet_until: u64,
    suppressed: u32,
}

impl Compiled {
    fn matches(&self, ev: &Event, name: WideStr<'_>, scratch: &mut Scratch) -> bool {
        if !self.enabled || !self.kinds.contains(ev.kind) {
            return false;
        }
        if let Some(roots) = &self.roots
            && !roots.get(usize::from(ev.root.0)).copied().unwrap_or(false)
        {
            return false;
        }
        self.glob
            .as_ref()
            .is_none_or(|glob| glob.eval(ev, name, scratch).matched_rule.is_some())
    }

    /// Counts a match at `ts`; the firing it completes.
    fn count(&mut self, ts: u64, rule: usize) -> Option<Fired> {
        if ts < self.quiet_until {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        let size = self.ring.len();
        if let Some(slot) = self.ring.get_mut(self.head) {
            *slot = ts;
        }
        self.head = (self.head + 1) % size;
        self.filled = (self.filled + 1).min(size);
        if self.filled < size {
            return None;
        }
        // The oldest of the last `size` matches is where the head points.
        let oldest = self.ring.get(self.head).copied().unwrap_or(ts);
        if ts.saturating_sub(oldest) > self.window {
            return None;
        }
        self.filled = 0;
        self.quiet_until = ts.saturating_add(self.cooldown);
        Some(Fired {
            rule,
            ts,
            matches: u32::try_from(size).unwrap_or(u32::MAX),
            suppressed: std::mem::take(&mut self.suppressed),
        })
    }
}

/// The compiled rules.
pub struct Evaluator {
    rules: Vec<Compiled>,
    scratch: Scratch,
}

impl std::fmt::Debug for Evaluator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Evaluator")
            .field("rules", &self.rules.len())
            .finish_non_exhaustive()
    }
}

impl Evaluator {
    /// Compiles `rules`; `watch_ids` are the ids of the watches in settings
    /// order, which is the `RootIdx` of their events.
    ///
    /// # Errors
    ///
    /// The first rule that does not compile, as `rule[i]: why`.
    pub fn compile(rules: &[Rule], watch_ids: &[String]) -> Result<Self, String> {
        let mut compiled = Vec::with_capacity(rules.len());
        for (i, rule) in rules.iter().enumerate() {
            let fail = |why: String| format!("rule[{i}]: {why}");
            let roots = if rule.watches.is_empty() {
                None
            } else {
                let mut flags = vec![false; watch_ids.len()];
                for id in &rule.watches {
                    let at = watch_ids
                        .iter()
                        .position(|candidate| candidate == id)
                        .ok_or_else(|| fail(format!("no watch has the id `{id}`")))?;
                    if let Some(flag) = flags.get_mut(at) {
                        *flag = true;
                    }
                }
                Some(flags.into_boxed_slice())
            };
            let size = usize::try_from(rule.threshold.max(1)).unwrap_or(1);
            compiled.push(Compiled {
                enabled: rule.enabled,
                roots,
                glob: glob_set(&rule.glob).map_err(fail)?,
                kinds: kind_set(&rule.kinds).map_err(fail)?,
                window: u64::from(rule.window_s) * SECOND,
                cooldown: u64::from(rule.cooldown_s) * SECOND,
                ring: vec![0; size].into_boxed_slice(),
                head: 0,
                filled: 0,
                quiet_until: 0,
                suppressed: 0,
            });
        }
        Ok(Self {
            rules: compiled,
            scratch: Scratch::new(),
        })
    }

    /// How many rules there are, enabled or not.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether there is no rule.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Evaluates one event whose delivered name is `name`; `fire` is called
    /// for each rule that fires. Allocates nothing.
    pub fn on_event(&mut self, ev: &Event, name: WideStr<'_>, mut fire: impl FnMut(Fired)) {
        let Self { rules, scratch } = self;
        for (index, rule) in rules.iter_mut().enumerate() {
            if rule.matches(ev, name, scratch)
                && let Some(fired) = rule.count(ev.ts, index)
            {
                fire(fired);
            }
        }
    }
}

#[cfg(test)]
mod tests;
