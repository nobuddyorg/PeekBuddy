//! File ids in SQLite's signed 64-bit integers (R3).
//!
//! NTFS ids are 64-bit; `ReFS` ids are 128-bit. The low half goes into
//! `file_id` (bit-cast to `i64`, so values above `i64::MAX` survive) and the
//! high half into `file_id_hi`, which is NULL when it is zero.

/// The two columns of a file id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdColumns {
    /// The low 64 bits as `file_id`.
    pub lo: i64,
    /// The high 64 bits as `file_id_hi`; `None` when zero.
    pub hi: Option<i64>,
}

/// Splits a 128-bit file id into its columns.
#[must_use]
pub fn split(id: u128) -> FileIdColumns {
    let lo = u64::try_from(id & u128::from(u64::MAX)).unwrap_or_default();
    let hi = u64::try_from(id >> 64).unwrap_or_default();
    FileIdColumns {
        lo: lo.cast_signed(),
        hi: (hi != 0).then(|| hi.cast_signed()),
    }
}

/// Joins the columns back into the file id.
#[must_use]
pub fn join(columns: FileIdColumns) -> u128 {
    let lo = u128::from(columns.lo.cast_unsigned());
    let hi = columns.hi.map_or(0, |hi| u128::from(hi.cast_unsigned()));
    (hi << 64) + lo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntfs_ids_leave_the_high_column_null() {
        let c = split(0x1234);
        assert_eq!(
            c,
            FileIdColumns {
                lo: 0x1234,
                hi: None
            }
        );
        assert_eq!(join(c), 0x1234);
    }

    #[test]
    fn ids_above_i64_max_bit_cast() {
        let id = u128::from(u64::MAX - 5);
        let c = split(id);
        assert_eq!(c.lo, -6);
        assert_eq!(c.hi, None);
        assert_eq!(join(c), id);
    }

    #[test]
    fn refs_ids_use_both_columns() {
        let id = (0x8000_0000_0000_0001_u128 << 64) | 0xFFFF_0000_0000_0002;
        let c = split(id);
        assert_eq!(c.hi, Some(i64::MIN + 1));
        assert_eq!(c.lo, 0xFFFF_0000_0000_0002_u64.cast_signed());
        assert_eq!(join(c), id);
    }
}
