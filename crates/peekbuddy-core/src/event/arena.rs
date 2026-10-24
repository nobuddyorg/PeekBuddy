//! The name arena: UTF-16 units for one ring slot or batch.

/// The longest name that a [`NameRef`] can address, in u16 units.
pub const MAX_NAME_UNITS: usize = 32_767;

/// A name inside a [`NameArena`]: offset and length in u16 units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameRef {
    /// Offset of the first unit.
    pub offset: u32,
    /// Length in units (at most [`MAX_NAME_UNITS`]).
    pub len: u16,
}

impl NameRef {
    /// The empty name.
    pub const EMPTY: Self = Self { offset: 0, len: 0 };
}

/// Fixed-capacity storage for names, reset on reuse.
///
/// Capacity is reserved once; pushing never reallocates.
#[derive(Debug)]
pub struct NameArena {
    units: Vec<u16>,
    capacity: usize,
}

impl NameArena {
    /// An arena that holds up to `capacity` units.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            units: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Forget every name; the storage stays.
    pub fn reset(&mut self) {
        self.units.clear();
    }

    /// Units currently stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether no names are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Store `name`; `None` when it is too long or the arena is full.
    pub fn push(&mut self, name: &[u16]) -> Option<NameRef> {
        let r = self.reserve(name.len())?;
        self.units.extend_from_slice(name);
        Some(r)
    }

    /// Store `len` units taken from `iter`; `None` when they do not fit or
    /// the iterator is shorter than `len`.
    pub fn push_iter(&mut self, len: usize, iter: impl Iterator<Item = u16>) -> Option<NameRef> {
        let r = self.reserve(len)?;
        let before = self.units.len();
        self.units.extend(iter.take(len));
        if self.units.len() - before == len {
            Some(r)
        } else {
            self.units.truncate(before);
            None
        }
    }

    /// The units a reference addresses; `None` when out of range.
    #[must_use]
    pub fn get(&self, r: NameRef) -> Option<&[u16]> {
        let start = usize::try_from(r.offset).ok()?;
        self.units
            .get(start..start.checked_add(usize::from(r.len))?)
    }

    fn reserve(&self, len: usize) -> Option<NameRef> {
        if len > MAX_NAME_UNITS || self.units.len().checked_add(len)? > self.capacity {
            return None;
        }
        Some(NameRef {
            offset: u32::try_from(self.units.len()).ok()?,
            len: u16::try_from(len).ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_get_reset() {
        let mut a = NameArena::with_capacity(8);
        let r1 = a.push(&[1, 2, 3]);
        let r2 = a.push(&[4, 5]);
        assert_eq!(r1.and_then(|r| a.get(r)), Some(&[1, 2, 3][..]));
        assert_eq!(r2.and_then(|r| a.get(r)), Some(&[4, 5][..]));
        assert_eq!(a.len(), 5);
        a.reset();
        assert!(a.is_empty());
        assert_eq!(r1.and_then(|r| a.get(r)), None);
    }

    #[test]
    fn full_arena_rejects() {
        let mut a = NameArena::with_capacity(4);
        assert!(a.push(&[1, 2, 3]).is_some());
        assert!(a.push(&[1, 2]).is_none());
        assert!(a.push(&[1]).is_some());
        assert!(a.push(&[]).is_some());
    }

    #[test]
    fn max_len_is_accepted_and_one_more_is_not() {
        let big = vec![7_u16; MAX_NAME_UNITS + 1];
        let mut a = NameArena::with_capacity(MAX_NAME_UNITS + 1);
        assert!(a.push(&big).is_none());
        assert!(a.push(big.get(..MAX_NAME_UNITS).unwrap()).is_some());
    }

    #[test]
    fn short_iterator_rolls_back() {
        let mut a = NameArena::with_capacity(8);
        assert!(a.push_iter(4, [1_u16, 2].into_iter()).is_none());
        assert!(a.is_empty());
        let r = a.push_iter(2, [1_u16, 2, 3].into_iter());
        assert_eq!(r.and_then(|r| a.get(r)), Some(&[1, 2][..]));
        // A failed push after a successful one must leave the earlier name.
        assert!(a.push_iter(3, [9_u16].into_iter()).is_none());
        assert_eq!(a.len(), 2);
        assert!(!a.is_empty());
        let r2 = a.push_iter(1, [7_u16].into_iter());
        assert_eq!(r2.and_then(|r| a.get(r)), Some(&[7][..]));
    }
}
