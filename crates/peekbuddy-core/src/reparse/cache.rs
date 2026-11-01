//! Churn detection for sources without a reason mask (#213).
//!
//! `ReadDirectoryChangesExW` reports "modified" with the attributes after the
//! change and nothing else, so [`classify`] has no reason and no "before"
//! to look at. [`ChurnCache`] supplies the before: the last attributes seen
//! per cloud file. A row is churn when the attribute delta is exactly a flip
//! of the sync-state bits. A file seen for the first time is never churn
//! (nothing to compare with), and a data write leaves the attributes alone,
//! so it is not churn either; the row stays in the log whatever the answer.
//!
//! The cache is direct-mapped and allocated when the first cloud file shows
//! up: a collision loses a "before", which only makes a row visible, never
//! hides one.

use super::{Change, classify, is_cloud};

/// Slots in the cache: 2048 files, 40 KiB.
const SLOTS: usize = 2048;

#[derive(Debug)]
struct Table {
    ids: Box<[u128]>,
    attrs: Box<[u32]>,
}

/// The last attributes seen per cloud file.
#[derive(Debug, Default)]
pub struct ChurnCache {
    table: Option<Table>,
}

impl ChurnCache {
    /// An empty cache; nothing is allocated until a cloud file is observed.
    #[must_use]
    pub const fn new() -> Self {
        Self { table: None }
    }

    /// Records the attributes of the file `file_id` after a change and tells
    /// whether this change is sync churn. Files without a cloud tag are not
    /// looked at.
    pub fn observe(&mut self, file_id: u128, tag: u32, attrs: u32) -> bool {
        if !is_cloud(tag) {
            return false;
        }
        let table = self.table.get_or_insert_with(|| Table {
            ids: vec![0; SLOTS].into_boxed_slice(),
            attrs: vec![0; SLOTS].into_boxed_slice(),
        });
        let slot = slot_of(file_id);
        let before = match (table.ids.get(slot), table.attrs.get(slot)) {
            (Some(&id), Some(&before)) if id == file_id => Some(before),
            _ => None,
        };
        if let Some(id) = table.ids.get_mut(slot) {
            *id = file_id;
        }
        if let Some(stored) = table.attrs.get_mut(slot) {
            *stored = attrs;
        }
        classify(&Change {
            tag,
            tag_before: None,
            reason: 0,
            source_info: 0,
            attrs_before: before,
            attrs_after: attrs,
        })
        .is_some()
    }
}

/// A multiplicative hash of the id into the table.
fn slot_of(file_id: u128) -> usize {
    let low = u64::try_from(file_id & u128::from(u64::MAX)).unwrap_or(0);
    let high = u64::try_from(file_id >> 64).unwrap_or(0);
    let folded = low ^ high;
    let mixed = folded.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    usize::try_from(mixed >> 40).map_or(0, |n| n % SLOTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reparse::{attr, tag};

    const CLOUD: u32 = tag::CLOUD;
    const ARCHIVE: u32 = 0x20;

    #[test]
    fn the_first_sighting_is_never_churn() {
        let mut cache = ChurnCache::new();
        assert!(!cache.observe(1, CLOUD, ARCHIVE | attr::RECALL_ON_DATA_ACCESS));
    }

    #[test]
    fn a_flip_of_the_sync_bits_is_churn() {
        let mut cache = ChurnCache::new();
        let dehydrated = ARCHIVE | attr::RECALL_ON_DATA_ACCESS | attr::OFFLINE;
        assert!(!cache.observe(1, CLOUD, dehydrated));
        // Hydrated: the recall bits go away.
        assert!(cache.observe(1, CLOUD, ARCHIVE));
        // Nothing changed: not a flip.
        assert!(!cache.observe(1, CLOUD, ARCHIVE));
        // Pinned.
        assert!(cache.observe(1, CLOUD, ARCHIVE | attr::PINNED));
    }

    #[test]
    fn a_change_of_other_attributes_is_not_churn() {
        let mut cache = ChurnCache::new();
        assert!(!cache.observe(1, CLOUD, ARCHIVE));
        // READONLY flipped together with a sync bit: the user did something.
        assert!(!cache.observe(1, CLOUD, ARCHIVE | 0x1 | attr::PINNED));
    }

    #[test]
    fn files_are_told_apart_and_non_cloud_files_are_ignored() {
        let mut cache = ChurnCache::new();
        assert!(!cache.observe(1, CLOUD, ARCHIVE));
        assert!(!cache.observe(2, CLOUD, ARCHIVE | attr::PINNED));
        // File 2 had no before; file 1 flips now.
        assert!(cache.observe(1, CLOUD, ARCHIVE | attr::PINNED));
        assert!(!cache.observe(3, 0, ARCHIVE));
        assert!(!cache.observe(3, tag::SYMLINK, ARCHIVE | attr::PINNED));
        assert!(cache.table.is_some());
    }

    #[test]
    fn nothing_is_allocated_before_a_cloud_file_appears() {
        let mut cache = ChurnCache::new();
        assert!(!cache.observe(1, 0, ARCHIVE));
        assert!(cache.table.is_none());
    }

    #[test]
    fn a_colliding_id_loses_its_before_and_shows_the_row() {
        let mut cache = ChurnCache::new();
        let other = (2..u128::from(u32::MAX))
            .find(|&id| slot_of(id) == slot_of(1))
            .unwrap();
        assert!(!cache.observe(1, CLOUD, ARCHIVE));
        assert!(!cache.observe(other, CLOUD, ARCHIVE));
        // File 1's slot was taken: its flip has no before, so it is shown.
        assert!(!cache.observe(1, CLOUD, ARCHIVE | attr::PINNED));
    }

    #[test]
    fn every_id_maps_into_the_table() {
        for id in [0, 1, u128::MAX, 1 << 64, 0xDEAD_BEEF_u128 << 40] {
            assert!(slot_of(id) < SLOTS);
        }
    }
}
