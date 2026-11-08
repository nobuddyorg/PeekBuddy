//! The Space-Saving heavy-hitter sketch (#82): the busiest items of a stream
//! in fixed memory.
//!
//! Metwally, Agrawal and El Abbadi, "Efficient Computation of Frequent and
//! Top-k Elements in Data Streams". `K` counters follow a stream of `N`
//! items. An item that is followed again has its counter raised; an item that
//! is not followed takes over the counter with the smallest count, starting
//! at that count plus one and remembering it as its possible overcount.
//!
//! Guarantees, all tested:
//!
//! - the counts add up to `N`;
//! - for a followed item, `count - error <= true count <= count`;
//! - the error of any counter is at most `N / K`, so every item that occurs
//!   more than `N / K` times is followed.
//!
//! The counts are approximate by design; a view that shows them says so.
//! Nothing allocates after [`Sketch::new`]: offering an item is a hash
//! lookup, plus a scan of the `K` counters when an item takes over one.

use std::collections::HashMap;

/// One followed item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counter {
    /// The item.
    pub key: u64,
    /// Its count, possibly too high by at most [`Counter::error`].
    pub count: u64,
    /// How much of the count may belong to items it took the place of.
    pub error: u64,
}

/// The sketch.
#[derive(Debug, Clone)]
pub struct Sketch {
    slots: Vec<Counter>,
    index: HashMap<u64, usize>,
    capacity: usize,
    seen: u64,
}

impl Sketch {
    /// A sketch that follows at most `capacity` items (at least one).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            slots: Vec::with_capacity(capacity),
            index: HashMap::with_capacity(capacity * 2),
            capacity,
            seen: 0,
        }
    }

    /// How many items were offered.
    #[must_use]
    pub const fn seen(&self) -> u64 {
        self.seen
    }

    /// The most a counter can be too high: `seen / capacity`, rounded up.
    #[must_use]
    pub fn max_error(&self) -> u64 {
        self.seen.div_ceil(self.capacity as u64)
    }

    /// Counts one occurrence of `key`.
    pub fn offer(&mut self, key: u64) {
        self.seen += 1;
        if let Some(&slot) = self.index.get(&key) {
            if let Some(counter) = self.slots.get_mut(slot) {
                counter.count += 1;
            }
            return;
        }
        if self.slots.len() < self.capacity {
            self.index.insert(key, self.slots.len());
            self.slots.push(Counter {
                key,
                count: 1,
                error: 0,
            });
            return;
        }
        // Take over the counter with the smallest count.
        let Some((slot, floor)) = self
            .slots
            .iter()
            .enumerate()
            .min_by_key(|(_, counter)| counter.count)
            .map(|(slot, counter)| (slot, counter.count))
        else {
            return;
        };
        if let Some(counter) = self.slots.get_mut(slot) {
            self.index.remove(&counter.key);
            self.index.insert(key, slot);
            *counter = Counter {
                key,
                count: floor + 1,
                error: floor,
            };
        }
    }

    /// The followed items, busiest first (ties by key, so the order is
    /// stable).
    #[must_use]
    pub fn top(&self) -> Vec<Counter> {
        let mut out = self.slots.clone();
        out.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
        out
    }

    /// Forgets everything; the memory stays.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.index.clear();
        self.seen = 0;
    }
}

#[cfg(test)]
mod tests;
