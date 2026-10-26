use super::*;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Plain-class records the way the kernel lays them out.
fn plain(records: &[(u32, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, (action, name)) in records.iter().enumerate() {
        let name = units(name);
        let size = (12 + name.len() * 2).next_multiple_of(4);
        let start = out.len();
        out.resize(start + size, 0);
        let next = if index + 1 == records.len() { 0 } else { size };
        let mut put = |at: usize, bytes: &[u8]| {
            out.get_mut(start + at..start + at + bytes.len())
                .unwrap()
                .copy_from_slice(bytes);
        };
        put(0, &u32::try_from(next).unwrap().to_le_bytes());
        put(4, &action.to_le_bytes());
        put(8, &u32::try_from(name.len() * 2).unwrap().to_le_bytes());
        for (i, unit) in name.iter().enumerate() {
            put(12 + i * 2, &unit.to_le_bytes());
        }
    }
    out
}

/// A sink with room for `room` events.
#[derive(Default)]
struct Ring {
    room: usize,
    got: Vec<(Kind, String)>,
}

impl EventSink for Ring {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        if self.got.len() >= self.room {
            return false;
        }
        assert_eq!(usize::from(event.name.len), name.len());
        self.got.push((event.kind, String::from_utf16_lossy(name)));
        true
    }
}

fn decoder(excluded: &[&str]) -> Decoder {
    let excluded: Vec<Vec<u16>> = excluded.iter().map(|p| units(p)).collect();
    Decoder::new(RootIdx(3), VolIdx(1), Class::Plain, &excluded)
}

const STAMP: Stamp = Stamp { ts: 10, qpc: 20 };

#[test]
fn records_keep_their_order_and_kinds() {
    let buf = plain(&[(4, "old.txt"), (5, "new.txt"), (1, "a"), (2, "b"), (3, "c")]);
    let mut ring = Ring {
        room: 10,
        ..Ring::default()
    };
    let mut d = decoder(&[]);
    let outcome = d.decode(&buf, buf.len(), STAMP, &mut ring);
    assert_eq!(outcome, Outcome::default());
    let kinds: Vec<Kind> = ring.got.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        [
            Kind::RenamedFrom,
            Kind::RenamedTo,
            Kind::Created,
            Kind::Deleted,
            Kind::Modified
        ]
    );
    assert_eq!(ring.got.first().map(|(_, n)| n.as_str()), Some("old.txt"));
    assert_eq!(d.counters().records, 5);
}

#[test]
fn zero_bytes_is_an_overflow_with_a_gap_row_and_a_reconcile() {
    let mut ring = Ring {
        room: 4,
        ..Ring::default()
    };
    let mut d = decoder(&[]);
    let outcome = d.decode(&[], 0, STAMP, &mut ring);
    assert_eq!(outcome.reconcile, Some(GapReason::BufferOverflow));
    assert_eq!(ring.got.len(), 1);
    assert_eq!(
        ring.got.first().map(|(k, _)| *k),
        Some(Kind::Overflow(GapReason::BufferOverflow))
    );
    let c = d.counters();
    assert_eq!((c.overflows, c.gap_rows, c.gaps_owed_total), (1, 1, 1));
    assert_eq!(d.owed(), 0);
}

#[test]
fn a_full_ring_counts_the_loss_owes_a_gap_and_pays_it_first_later() {
    let buf = plain(&[(1, "a"), (1, "b"), (1, "c"), (1, "d")]);
    let mut ring = Ring {
        room: 2,
        ..Ring::default()
    };
    let mut d = decoder(&[]);
    let outcome = d.decode(&buf, buf.len(), STAMP, &mut ring);
    assert_eq!(outcome.reconcile, Some(GapReason::RingFull));
    let c = d.counters();
    assert_eq!((c.records, c.dropped_full), (2, 2));
    // The gap row itself did not fit either.
    assert_eq!(d.owed(), 1);
    assert_eq!(c.gap_rows, 0);

    ring.room = 10;
    let again = plain(&[(3, "e")]);
    d.decode(&again, again.len(), STAMP, &mut ring);
    let kinds: Vec<Kind> = ring.got.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        [
            Kind::Created,
            Kind::Created,
            Kind::Overflow(GapReason::RingFull),
            Kind::Modified
        ]
    );
    assert_eq!(d.owed(), 0);
    assert_eq!(d.counters().gap_rows, d.counters().gaps_owed_total);
}

#[test]
fn a_malformed_buffer_is_a_parser_gap() {
    let mut buf = plain(&[(1, "a"), (1, "b")]);
    // Break the second record's name length.
    let second = 16;
    buf.get_mut(second + 8..second + 12)
        .unwrap()
        .copy_from_slice(&1000_u32.to_le_bytes());
    let mut ring = Ring {
        room: 10,
        ..Ring::default()
    };
    let mut d = decoder(&[]);
    let outcome = d.decode(&buf, buf.len(), STAMP, &mut ring);
    assert_eq!(outcome.reconcile, Some(GapReason::ParserError));
    assert_eq!(d.counters().parse_errors, 1);
    assert_eq!(ring.got.len(), 2, "the good record and the gap row");
}

#[test]
fn excluded_directories_are_dropped_and_counted() {
    let buf = plain(&[
        (1, "Data"),
        (3, r"data\segment.db"),
        (3, r"DATAX\keep.txt"),
        (3, r"other\data"),
        (3, "keep.txt"),
    ]);
    let mut ring = Ring {
        room: 10,
        ..Ring::default()
    };
    let mut d = decoder(&["DATA"]);
    d.decode(&buf, buf.len(), STAMP, &mut ring);
    let names: Vec<&str> = ring.got.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, [r"DATAX\keep.txt", r"other\data", "keep.txt"]);
    assert_eq!(d.counters().excluded, 2);
}

#[test]
fn an_empty_prefix_excludes_the_whole_root() {
    let buf = plain(&[(1, "a"), (1, "b")]);
    let mut ring = Ring {
        room: 10,
        ..Ring::default()
    };
    let mut d = decoder(&[""]);
    d.decode(&buf, buf.len(), STAMP, &mut ring);
    assert_eq!(ring.got, []);
    assert_eq!(d.counters().excluded, 2);
}

#[test]
fn relative_prefix_compares_ignoring_case() {
    let root = units(r"\\?\C:\Users\me");
    assert_eq!(
        relative_prefix(&root, &units(r"\\?\c:\users\ME\AppData\PeekBuddy")),
        Some(units(r"AppData\PeekBuddy"))
    );
    assert_eq!(
        relative_prefix(&root, &units(r"\\?\C:\Users\me\")),
        Some(vec![])
    );
    assert_eq!(relative_prefix(&root, &units(r"\\?\C:\Users\mex")), None);
    assert_eq!(relative_prefix(&root, &units(r"\\?\C:\Users")), None);
    assert_eq!(relative_prefix(&root, &units(r"\\?\D:\Users\me\x")), None);
}
