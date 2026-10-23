//! The ground-truth ledger: JSONL, one header line, then one record per
//! operation. Kept in memory during the run and written after it, outside
//! the watched root.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::ops::{Expected, Op, OpKind, RawKind, WidePath};

/// Prefix of a path that is not valid Unicode, followed by 4 hex digits per
/// UTF-16 unit.
const HEX_PREFIX: &str = "utf16:";

/// A UTF-16 path as ledger text: the string itself when it is valid
/// Unicode, otherwise [`HEX_PREFIX`] and the units in hex. A valid path that
/// happens to start with the prefix is hex-encoded too, so decoding is
/// unambiguous.
#[must_use]
pub fn encode_path(path: &[u16]) -> String {
    match String::from_utf16(path) {
        Ok(text) if !text.starts_with(HEX_PREFIX) => text,
        _ => {
            let mut out = String::with_capacity(HEX_PREFIX.len() + path.len() * 4);
            out.push_str(HEX_PREFIX);
            for unit in path {
                let _ = write!(out, "{unit:04x}");
            }
            out
        }
    }
}

/// Inverse of [`encode_path`]; `None` for malformed hex.
#[must_use]
pub fn decode_path(text: &str) -> Option<WidePath> {
    match text.strip_prefix(HEX_PREFIX) {
        None => Some(text.encode_utf16().collect()),
        Some(hex) => {
            if hex.len() % 4 != 0 {
                return None;
            }
            (0..hex.len() / 4)
                .map(|i| u16::from_str_radix(hex.get(i * 4..i * 4 + 4)?, 16).ok())
                .collect()
        }
    }
}

/// First line of a ledger: the conditions of the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// Ledger format version.
    pub version: u32,
    /// Profile name.
    pub profile: String,
    /// Seed.
    pub seed: u64,
    /// Generator threads.
    pub threads: u32,
    /// OS build, for example `10.0.26100.6584`.
    pub os_build: String,
    /// Runner label or machine name.
    pub runner: String,
    /// Logical processors.
    pub cpus: u32,
    /// File system of the root volume (`NTFS`, `ReFS`, ...).
    pub file_system: String,
    /// Defender real-time protection, when readable.
    pub defender_realtime: Option<bool>,
    /// 8.3 short-name generation on the volume, when readable.
    pub short_names: Option<bool>,
}

/// An expected raw record in ledger form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedText {
    /// Encoded path ([`encode_path`]).
    pub path: String,
    /// Record kind.
    pub kind: RawKind,
}

/// One performed operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Global sequence number, in the order operations finished.
    pub seq: u64,
    /// Generator thread.
    pub thread: u32,
    /// Operation class.
    pub op: OpKind,
    /// Encoded operand paths.
    pub paths: Vec<String>,
    /// Start, UTC FILETIME ticks (`GetSystemTimePreciseAsFileTime`).
    pub t_start: u64,
    /// End, UTC FILETIME ticks.
    pub t_end: u64,
    /// Win32 error code; 0 is success. Failed operations expect nothing.
    pub result: u32,
    /// Records the watcher must log.
    pub expected: Vec<ExpectedText>,
}

impl Record {
    /// The record for `op` run by `thread`.
    #[must_use]
    pub fn new(seq: u64, thread: u32, op: &Op, t_start: u64, t_end: u64, result: u32) -> Self {
        let expected = if result == 0 {
            op.expected()
        } else {
            Vec::new()
        };
        Self {
            seq,
            thread,
            op: op.kind,
            paths: op.paths.iter().map(|p| encode_path(p)).collect(),
            t_start,
            t_end,
            result,
            expected: expected
                .into_iter()
                .map(|e| ExpectedText {
                    path: encode_path(&e.path),
                    kind: e.kind,
                })
                .collect(),
        }
    }

    /// The expected records with decoded paths; `None` for a malformed path.
    #[must_use]
    pub fn expected_records(&self) -> Option<Vec<Expected>> {
        self.expected
            .iter()
            .map(|e| {
                Some(Expected {
                    path: decode_path(&e.path)?,
                    kind: e.kind,
                })
            })
            .collect()
    }
}

/// Serialises a ledger as JSONL.
///
/// # Errors
///
/// Returns the serialisation error (only for values JSON cannot hold).
pub fn to_jsonl(header: &Header, records: &[Record]) -> serde_json::Result<String> {
    let mut out = serde_json::to_string(header)?;
    out.push('\n');
    for record in records {
        out.push_str(&serde_json::to_string(record)?);
        out.push('\n');
    }
    Ok(out)
}

/// Parses a JSONL ledger.
///
/// # Errors
///
/// Returns the parse error of the first bad line, or an error for an empty
/// input.
pub fn from_jsonl(text: &str) -> Result<(Header, Vec<Record>), String> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().ok_or("empty ledger")?;
    let header: Header = serde_json::from_str(header).map_err(|e| format!("header: {e}"))?;
    let records = lines
        .enumerate()
        .map(|(i, line)| serde_json::from_str(line).map_err(|e| format!("record {}: {e}", i + 1)))
        .collect::<Result<_, _>>()?;
    Ok((header, records))
}

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    reason = "tests index values of known size; a panic is a test failure"
)]
mod tests {
    use super::{Header, Record, decode_path, encode_path, from_jsonl, to_jsonl};
    use crate::ops::{Op, OpKind, RawKind};
    use crate::plan::{Planner, Shape};
    use crate::profile::profile;
    use proptest::prelude::{any, prop, proptest};

    fn header() -> Header {
        Header {
            version: 1,
            profile: "edge-mix".into(),
            seed: 1,
            threads: 1,
            os_build: "10.0.26100.1".into(),
            runner: "test".into(),
            cpus: 4,
            file_system: "NTFS".into(),
            defender_realtime: None,
            short_names: Some(false),
        }
    }

    proptest! {
        #[test]
        fn paths_round_trip(path in prop::collection::vec(any::<u16>(), 0..40)) {
            let text = encode_path(&path);
            assert_eq!(decode_path(&text), Some(path));
        }
    }

    #[test]
    fn valid_paths_stay_readable() {
        let path: Vec<u16> = "t0\\文件\u{1F600}f1".encode_utf16().collect();
        assert_eq!(encode_path(&path), "t0\\文件\u{1F600}f1");
        assert_eq!(encode_path(&[0x61, 0xD800]), "utf16:0061d800");
        let tricky: Vec<u16> = "utf16:0061".encode_utf16().collect();
        assert_eq!(decode_path(&encode_path(&tricky)), Some(tricky));
        assert_eq!(decode_path("utf16:006"), None);
        assert_eq!(decode_path("utf16:zzzz"), None);
    }

    #[test]
    fn failed_operations_expect_nothing() {
        let op = Op {
            kind: OpKind::Delete,
            paths: vec!["f".encode_utf16().collect()],
            removed_below: Vec::new(),
        };
        assert_eq!(Record::new(0, 0, &op, 1, 2, 5).expected, []);
        let ok = Record::new(0, 0, &op, 1, 2, 0);
        assert_eq!(ok.expected.len(), 1);
        assert_eq!(ok.expected[0].kind, RawKind::Removed);
        assert_eq!(ok.expected_records().unwrap()[0].path, op.paths[0]);
    }

    /// Same seed, one thread: the ledger is identical except for times.
    #[test]
    fn same_seed_same_ledger_minus_times() {
        let ledger = |t0: u64| {
            let mut planner = Planner::new(1, 0, profile("edge-mix").unwrap());
            let mut ops = planner.setup(Shape::default());
            ops.extend((0..300).map(|_| planner.next_op()));
            let records: Vec<Record> = ops
                .iter()
                .zip(0u64..)
                .map(|(op, seq)| Record::new(seq, 0, op, t0 + seq * 10, t0 + seq * 10 + 3, 0))
                .collect();
            to_jsonl(&header(), &records).unwrap()
        };
        let strip = |text: String| -> Vec<Record> {
            let (_, mut records) = from_jsonl(&text).unwrap();
            for record in &mut records {
                record.t_start = 0;
                record.t_end = 0;
            }
            records
        };
        let first = ledger(1_000);
        assert_ne!(first, ledger(9_999));
        assert_eq!(strip(first), strip(ledger(9_999)));
    }

    #[test]
    fn jsonl_round_trip_and_errors() {
        let op = Op {
            kind: OpKind::Rename,
            paths: vec![vec![0xD800, 0x61], "b".encode_utf16().collect()],
            removed_below: Vec::new(),
        };
        let records = vec![Record::new(7, 2, &op, 10, 20, 0)];
        let text = to_jsonl(&header(), &records).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(from_jsonl(&text).unwrap(), (header(), records));
        assert!(from_jsonl("").is_err());
        assert!(from_jsonl("{}").is_err());
        let bad = format!("{}\nnot json\n", text.lines().next().unwrap());
        assert_eq!(
            from_jsonl(&bad).unwrap_err().split(':').next(),
            Some("record 1")
        );
    }
}
