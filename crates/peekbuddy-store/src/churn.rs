//! The churn sketches the writer keeps (#82): which paths and which file
//! extensions changed most in each hour of each root.
//!
//! One pair of [`Sketch`]es (K = [`K`]) per `(hour, root)`: one follows the
//! interned node of every event (a path), the other the extension of its
//! name. Offering an event allocates nothing once a pair exists. The sketches
//! are written into the `churn` table inside the commit that moves them on:
//! when their hour is over, at most once a minute of event time while it
//! runs (F6: no timer), and when the writer rotates or closes. A crash loses
//! at most the last minute of one sketch; the counts are approximate by
//! design (see [`peekbuddy_core::sketch`]).

use std::collections::HashMap;

use peekbuddy_core::changes::{Op, op_for};
use peekbuddy_core::event::Event;
use peekbuddy_core::name::fold_upper;
use peekbuddy_core::sketch::{Counter, Sketch};
use rusqlite::{Transaction, params};

use crate::schema::TICKS_PER_HOUR;

/// Counters per sketch.
pub const K: usize = 100;

/// FILETIME ticks between two writes of a running sketch: one minute.
const FLUSH_TICKS: i64 = 600_000_000;

const BACKSLASH: u16 = 0x5C;
const DOT: u16 = 0x2E;

/// The `dim` column of a path sketch.
pub const DIM_PATH: i64 = 0;
/// The `dim` column of an extension sketch.
pub const DIM_EXT: i64 = 1;

/// The longest extension that is followed, in UTF-16 units; longer tails are
/// part of a name, not an extension.
const MAX_EXT: usize = 12;

#[derive(Debug)]
struct Pair {
    paths: Sketch,
    exts: Sketch,
    /// Event time of the last write; 0 when never written.
    flushed_at: i64,
    dirty: bool,
}

impl Pair {
    fn new() -> Self {
        Self {
            paths: Sketch::new(K),
            exts: Sketch::new(K),
            flushed_at: 0,
            dirty: false,
        }
    }
}

/// The sketches of the writer.
#[derive(Debug, Default)]
pub struct Churn {
    pairs: HashMap<(i64, i64), Pair>,
    /// The text of every extension key seen, for the table.
    labels: HashMap<u64, String>,
    newest_hour: i64,
}

/// The extension of the last component of `name` (after its last dot, not
/// the first character), upper-cased and hashed, with its units.
fn extension(name: &[u16]) -> Option<(u64, &[u16])> {
    let leaf_start = name
        .iter()
        .rposition(|&u| u == BACKSLASH)
        .map_or(0, |i| i + 1);
    let leaf = name.get(leaf_start..)?;
    let dot = leaf.iter().rposition(|&u| u == DOT)?;
    let ext = leaf.get(dot + 1..)?;
    if dot == 0 || ext.is_empty() || ext.len() > MAX_EXT {
        return None;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &unit in ext {
        for byte in fold_upper(unit).to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Some((hash, ext))
}

/// Whether a row counts as a change of a path.
fn counts(event: &Event) -> bool {
    matches!(
        op_for(event.kind, event.is_dir, (), 0, 0),
        Some(
            Op::Created(())
                | Op::Modified(())
                | Op::Deleted(())
                | Op::RenamedFrom { .. }
                | Op::RenamedTo { .. }
        )
    )
}

impl Churn {
    /// Counts one committed event of `root_id` whose node is `node_id`.
    pub fn observe(&mut self, root_id: i64, node_id: i64, event: &Event, name: &[u16]) {
        if !counts(event) {
            return;
        }
        let ts = event.ts.cast_signed();
        let hour = ts.div_euclid(TICKS_PER_HOUR) * TICKS_PER_HOUR;
        self.newest_hour = self.newest_hour.max(hour);
        let pair = self.pairs.entry((hour, root_id)).or_insert_with(Pair::new);
        pair.paths.offer(node_id.cast_unsigned());
        if let Some((key, units)) = extension(name) {
            pair.exts.offer(key);
            self.labels
                .entry(key)
                .or_insert_with(|| String::from_utf16_lossy(units).to_uppercase());
        }
        pair.dirty = true;
        if pair.flushed_at == 0 {
            // Never written: the first write is due a minute from now.
            pair.flushed_at = ts;
        }
    }

    /// Writes the sketches that are due (all of them with `force`) and drops
    /// the ones whose hour is over. `now` is the event time of the commit.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub fn flush(&mut self, tx: &Transaction<'_>, now: i64, force: bool) -> rusqlite::Result<()> {
        let newest = self.newest_hour;
        let due: Vec<(i64, i64)> = self
            .pairs
            .iter()
            .filter(|((hour, _), pair)| {
                pair.dirty && (force || *hour < newest || now - pair.flushed_at >= FLUSH_TICKS)
            })
            .map(|(key, _)| *key)
            .collect();
        for key in due {
            if let Some(pair) = self.pairs.get_mut(&key) {
                write_pair(tx, key, pair, &self.labels)?;
                pair.flushed_at = now;
                pair.dirty = false;
            }
        }
        self.pairs.retain(|(hour, _), _| *hour >= newest);
        Ok(())
    }
}

fn write_pair(
    tx: &Transaction<'_>,
    (hour, root): (i64, i64),
    pair: &Pair,
    labels: &HashMap<u64, String>,
) -> rusqlite::Result<()> {
    tx.prepare_cached("DELETE FROM churn WHERE hour_ts = ?1 AND root_id = ?2")?
        .execute(params![hour, root])?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO churn (hour_ts, root_id, dim, key, label, n, err) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let put = |insert: &mut rusqlite::CachedStatement<'_>,
               dim: i64,
               counter: &Counter,
               label: Option<&String>| {
        insert.execute(params![
            hour,
            root,
            dim,
            counter.key.cast_signed(),
            label,
            i64::try_from(counter.count).unwrap_or(i64::MAX),
            i64::try_from(counter.error).unwrap_or(i64::MAX),
        ])
    };
    for counter in pair.paths.top() {
        put(&mut insert, DIM_PATH, &counter, None)?;
    }
    for counter in pair.exts.top() {
        put(&mut insert, DIM_EXT, &counter, labels.get(&counter.key))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::extension;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn the_extension_is_the_tail_of_the_last_component_in_any_case() {
        let key = |text: &str| extension(&units(text)).map(|(key, _)| key);
        assert!(key("dir\\a.TXT").is_some());
        assert_eq!(key("dir\\a.TXT"), key("other\\b.txt"));
        assert_ne!(key("a.txt"), key("a.log"));
        // The last dot counts, not the first.
        assert_eq!(key("a.tar.gz"), key("x.gz"));
    }

    #[test]
    fn names_without_a_real_extension_have_none() {
        for text in [
            "",
            "readme",
            ".gitignore",
            "dir.d\\file",
            "trailing.",
            "a.waytoolongtail",
        ] {
            assert!(extension(&units(text)).is_none(), "{text:?}");
        }
        assert_eq!(
            extension(&units("a.TXT")).map(|(_, e)| String::from_utf16_lossy(e)),
            Some(String::from("TXT"))
        );
    }
}
