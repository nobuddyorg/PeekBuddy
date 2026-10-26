//! A batch of events with their names: the unit that travels from the capture
//! side to the writer (#52).
//!
//! Capacity for events and name units is reserved once; filling a batch never
//! reallocates, and a batch returned by the writer is cleared and reused, so
//! the capture path allocates nothing per event (F10).

use peekbuddy_core::event::{Event, NameArena};

/// Why an event did not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Full {
    /// The batch holds as many events as it can.
    Events,
    /// The name does not fit the name storage.
    Names,
}

/// Events and the names they refer to.
#[derive(Debug)]
pub struct Batch {
    events: Vec<Event>,
    names: NameArena,
    capacity: usize,
}

impl Batch {
    /// A batch for `events` events and `name_units` UTF-16 units of names.
    #[must_use]
    pub fn with_capacity(events: usize, name_units: usize) -> Self {
        Self {
            events: Vec::with_capacity(events),
            names: NameArena::with_capacity(name_units),
            capacity: events,
        }
    }

    /// The default shape used by capture: 256 events, 32 Ki name units.
    #[must_use]
    pub fn standard() -> Self {
        Self::with_capacity(256, 32 * 1024)
    }

    /// Adds `event` with its `name` (a path relative to the watch root,
    /// separated by `\`; empty for events that name the root itself).
    ///
    /// # Errors
    ///
    /// [`Full`] when it does not fit; the batch is unchanged.
    pub fn push(&mut self, mut event: Event, name: &[u16]) -> Result<(), Full> {
        if self.events.len() >= self.capacity {
            return Err(Full::Events);
        }
        event.name = self.names.push(name).ok_or(Full::Names)?;
        self.events.push(event);
        Ok(())
    }

    /// Number of events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the batch has no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Forgets the events and names; the storage stays.
    pub fn clear(&mut self) {
        self.events.clear();
        self.names.reset();
    }

    /// The events with their names, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&Event, &[u16])> {
        self.events
            .iter()
            .map(|event| (event, self.names.get(event.name).unwrap_or_default()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use peekbuddy_core::event::{Event, Flags, Kind, NameKind, NameRef, RootIdx, Source, VolIdx};

    use super::{Batch, Full};

    /// A plain created-file event for `root`.
    pub(crate) fn event(ts: u64, root: u16) -> Event {
        Event {
            ts,
            qpc: ts,
            src_ts: 0,
            source: Source::Rdcw,
            kind: Kind::Created,
            raw_mask: 1,
            source_info: 0,
            is_dir: false,
            size: 10,
            attrs: 0x20,
            reparse_tag: 0,
            file_id: u128::from(ts),
            parent_file_id: 5,
            usn: 0,
            vol: VolIdx(0),
            root: RootIdx(root),
            name: NameRef::EMPTY,
            name_kind: NameKind::Long,
            flags: Flags::NONE,
        }
    }

    pub(crate) fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn events_keep_their_names_and_order() {
        let mut batch = Batch::with_capacity(4, 64);
        batch.push(event(1, 0), &units("a\\b.txt")).unwrap();
        batch.push(event(2, 0), &[]).unwrap();
        let rows: Vec<_> = batch.iter().map(|(e, n)| (e.ts, n.to_vec())).collect();
        assert_eq!(rows, [(1, units("a\\b.txt")), (2, vec![])]);
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn a_full_batch_refuses_without_changing() {
        let mut batch = Batch::with_capacity(1, 8);
        batch.push(event(1, 0), &units("abc")).unwrap();
        assert_eq!(batch.push(event(2, 0), &[]), Err(Full::Events));
        let mut small = Batch::with_capacity(4, 2);
        assert_eq!(small.push(event(1, 0), &units("abc")), Err(Full::Names));
        assert!(small.is_empty());
    }

    #[test]
    fn clearing_keeps_the_storage() {
        let mut batch = Batch::with_capacity(8, 64);
        for ts in 0..8 {
            batch.push(event(ts, 0), &units("x")).unwrap();
        }
        let before = batch.events.as_ptr();
        batch.clear();
        assert!(batch.is_empty());
        batch.push(event(9, 0), &units("y")).unwrap();
        assert_eq!(batch.events.as_ptr(), before);
    }
}
