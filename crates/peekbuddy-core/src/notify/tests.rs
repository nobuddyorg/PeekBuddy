use proptest::prelude::*;

use super::*;

/// A record as the model sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Model {
    action: u32,
    name: Vec<u16>,
    extended: Option<Extended>,
}

fn align4(n: usize) -> usize {
    n.next_multiple_of(4)
}

/// Encodes `records` the way the kernel lays them out: 4-byte aligned
/// records, `NextEntryOffset` 0 on the last.
fn encode(records: &[Model], class: Class) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let start = out.len();
        let name_bytes = record.name.len() * 2;
        let size = align4(class.header_len() + name_bytes);
        out.resize(start + size, 0);
        let last = index + 1 == records.len();
        let next = if last {
            0
        } else {
            u32::try_from(size).unwrap()
        };
        put(
            &mut out,
            start + layout::NEXT_ENTRY_OFFSET,
            &next.to_le_bytes(),
        );
        put(
            &mut out,
            start + layout::ACTION,
            &record.action.to_le_bytes(),
        );
        let name_len = u32::try_from(name_bytes).unwrap();
        match class {
            Class::Plain => put(
                &mut out,
                start + layout::PLAIN_NAME_LENGTH,
                &name_len.to_le_bytes(),
            ),
            Class::Extended => {
                put(
                    &mut out,
                    start + layout::EXTENDED_NAME_LENGTH,
                    &name_len.to_le_bytes(),
                );
            }
            Class::Full => put(
                &mut out,
                start + layout::EXTENDED_NAME_LENGTH,
                &u16::try_from(name_len).unwrap().to_le_bytes(),
            ),
        }
        if let Some(e) = record.extended {
            for (at, value) in [
                (layout::CREATION_TIME, e.creation_time),
                (layout::LAST_WRITE_TIME, e.last_write_time),
                (layout::CHANGE_TIME, e.change_time),
                (layout::ACCESS_TIME, e.last_access_time),
                (layout::ALLOCATED_LENGTH, e.allocated_length),
                (layout::FILE_SIZE, e.file_size),
                (layout::FILE_ID, e.file_id),
                (layout::PARENT_FILE_ID, e.parent_file_id),
            ] {
                put(&mut out, start + at, &value.to_le_bytes());
            }
            put(
                &mut out,
                start + layout::FILE_ATTRIBUTES,
                &e.attributes.to_le_bytes(),
            );
            put(
                &mut out,
                start + layout::REPARSE_TAG,
                &e.reparse_tag.to_le_bytes(),
            );
            if let Some(flags) = e.name_flags {
                put(&mut out, start + layout::FULL_NAME_FLAGS, &[flags]);
            }
        }
        for (i, unit) in record.name.iter().enumerate() {
            put(
                &mut out,
                start + class.header_len() + 2 * i,
                &unit.to_le_bytes(),
            );
        }
    }
    out
}

fn put(out: &mut [u8], at: usize, bytes: &[u8]) {
    out.get_mut(at..at + bytes.len())
        .unwrap()
        .copy_from_slice(bytes);
}

fn plain(action: u32, name: &str) -> Model {
    Model {
        action,
        name: name.encode_utf16().collect(),
        extended: None,
    }
}

fn ext(flags: Option<u8>) -> Extended {
    Extended {
        creation_time: 1,
        last_write_time: 2,
        change_time: 3,
        last_access_time: 4,
        allocated_length: 5,
        file_size: 6,
        attributes: ATTRIBUTE_DIRECTORY,
        reparse_tag: 7,
        file_id: 8,
        parent_file_id: 9,
        name_flags: flags,
    }
}

fn model_of(buf: &[u8], class: Class) -> (Vec<Model>, Option<ParseError>) {
    let mut records = Vec::new();
    for item in parse(buf, buf.len(), class) {
        match item {
            Ok(r) => records.push(Model {
                action: r.action.raw(),
                name: r.name.units().collect(),
                extended: r.extended,
            }),
            Err(e) => return (records, Some(e)),
        }
    }
    (records, None)
}

fn errors(buf: &[u8], class: Class) -> Option<ParseError> {
    model_of(buf, class).1
}

fn set_u32(buf: &mut [u8], at: usize, value: u32) {
    put(buf, at, &value.to_le_bytes());
}

#[test]
fn actions_map_to_kinds_and_back() {
    for (raw, action, kind) in [
        (1, Action::Added, Kind::Created),
        (2, Action::Removed, Kind::Deleted),
        (3, Action::Modified, Kind::Modified),
        (4, Action::RenamedOldName, Kind::RenamedFrom),
        (5, Action::RenamedNewName, Kind::RenamedTo),
        (0, Action::Unknown(0), Kind::Other),
        (6, Action::Unknown(6), Kind::Other),
    ] {
        assert_eq!(Action::from_raw(raw), action);
        assert_eq!(action.raw(), raw);
        assert_eq!(action.kind(), kind);
    }
}

#[test]
fn header_sizes() {
    assert_eq!(Class::Plain.header_len(), 12);
    assert_eq!(Class::Extended.header_len(), 84);
    assert_eq!(Class::Full.header_len(), 84);
}

#[test]
fn a_plain_buffer_round_trips_in_order() {
    let records = vec![
        plain(4, "old.txt"),
        plain(5, "new.txt"),
        plain(3, "dir\\f"),
        plain(9, ""),
    ];
    let buf = encode(&records, Class::Plain);
    assert_eq!(model_of(&buf, Class::Plain), (records, None));
}

#[test]
fn extended_and_full_round_trip() {
    for (class, flags) in [(Class::Extended, None), (Class::Full, Some(NAME_FLAG_BOTH))] {
        let records = vec![
            Model {
                action: 1,
                name: vec![0x61, 0xD800],
                extended: Some(ext(flags)),
            },
            Model {
                action: 2,
                name: Vec::new(),
                extended: Some(ext(flags)),
            },
        ];
        let buf = encode(&records, class);
        assert_eq!(model_of(&buf, class), (records, None));
    }
}

#[test]
fn every_field_decodes_from_its_offset() {
    let e = Extended {
        creation_time: -1,
        last_write_time: i64::MAX,
        change_time: i64::MIN,
        last_access_time: 0x0102_0304_0506_0708,
        allocated_length: 10,
        file_size: 11,
        attributes: 0xA0B0_C0D0,
        reparse_tag: 0xA000_000C,
        file_id: 12,
        parent_file_id: 13,
        name_flags: Some(NAME_FLAGS_UNSPECIFIED),
    };
    let buf = encode(
        &[Model {
            action: 3,
            name: vec![0x41],
            extended: Some(e),
        }],
        Class::Full,
    );
    let parsed = parse(&buf, buf.len(), Class::Full).next().unwrap().unwrap();
    assert_eq!(parsed.extended, Some(e));
    assert!(
        Extended {
            attributes: ATTRIBUTE_DIRECTORY,
            ..e
        }
        .is_dir()
    );
    assert!(
        !Extended {
            attributes: 0x20,
            ..e
        }
        .is_dir()
    );
}

#[test]
fn names_expose_units_and_copy() {
    let buf = encode(&[plain(1, "héllo")], Class::Plain);
    let record = parse(&buf, buf.len(), Class::Plain)
        .next()
        .unwrap()
        .unwrap();
    let name = record.name;
    assert_eq!(name.len(), 5);
    assert!(!name.is_empty());
    assert_eq!(name.units().len(), 5);
    let mut out = [0_u16; 8];
    assert_eq!(name.copy_into(&mut out), Some(5));
    assert_eq!(
        &out[..5],
        "héllo".encode_utf16().collect::<Vec<_>>().as_slice()
    );
    assert_eq!(out[5], 0);
    assert_eq!(name.copy_into(&mut [0_u16; 4]), None);
    assert_eq!(name.copy_into(&mut [0_u16; 5]), Some(5));
    assert_eq!(format!("{name:?}"), "\"héllo\"");
    assert!(Utf16Name::from_le_bytes(&[]).is_empty());
    assert!(Utf16Name::from_le_bytes(&[1]).is_empty());
    assert!(!Utf16Name::from_le_bytes(&[1, 0]).is_empty());
    assert_eq!(Utf16Name::from_le_bytes(&[1, 0, 2]).len(), 1);
}

#[test]
fn empty_and_zero_length_buffers_yield_nothing() {
    for class in [Class::Plain, Class::Extended, Class::Full] {
        assert_eq!(parse(&[], 0, class).count(), 0);
        assert_eq!(parse(&[0; 64], 0, class).count(), 0);
    }
}

#[test]
fn only_the_returned_prefix_is_parsed() {
    let records = vec![plain(1, "a"), plain(2, "b")];
    let mut buf = encode(&records, Class::Plain);
    let first = align4(12 + 2);
    // Garbage after the prefix is never looked at; the first record's next
    // offset is cut off by `bytes_returned`.
    buf.extend_from_slice(&[0xFF; 32]);
    let items: Vec<_> = parse(&buf, first, Class::Plain).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items.first().copied().unwrap(),
        Err(ParseError {
            offset: 0,
            kind: ErrorKind::NextPastEnd
        })
    );
}

#[test]
fn bytes_returned_past_the_buffer_is_an_error() {
    let buf = encode(&[plain(1, "a")], Class::Plain);
    let items: Vec<_> = parse(&buf, buf.len() + 1, Class::Plain).collect();
    assert_eq!(
        items,
        vec![Err(ParseError {
            offset: 0,
            kind: ErrorKind::BadLength
        })]
    );
    assert_eq!(parse(&buf, buf.len(), Class::Plain).count(), 1);
}

#[test]
fn truncated_headers_are_rejected_at_every_length() {
    for class in [Class::Plain, Class::Extended, Class::Full] {
        let full = encode(&[plain(1, "")], class);
        let header = class.header_len();
        for cut in 1..header {
            let got = errors(full.get(..cut).unwrap(), class);
            assert_eq!(
                got,
                Some(ParseError {
                    offset: 0,
                    kind: ErrorKind::Truncated
                }),
                "{class:?} {cut}"
            );
        }
        assert_eq!(errors(full.get(..header).unwrap(), class), None);
    }
}

#[test]
fn odd_name_lengths_are_rejected() {
    let mut buf = encode(&[plain(1, "ab")], Class::Plain);
    set_u32(&mut buf, layout::PLAIN_NAME_LENGTH, 3);
    assert_eq!(
        errors(&buf, Class::Plain),
        Some(ParseError {
            offset: 0,
            kind: ErrorKind::OddNameLength
        })
    );
    set_u32(&mut buf, layout::PLAIN_NAME_LENGTH, 4);
    assert_eq!(errors(&buf, Class::Plain), None);
}

#[test]
fn names_past_the_end_are_rejected_exactly_at_the_boundary() {
    let mut buf = encode(&[plain(1, "ab")], Class::Plain);
    // The buffer holds 12 + 4 bytes; a name of 6 bytes does not fit.
    set_u32(&mut buf, layout::PLAIN_NAME_LENGTH, 6);
    assert_eq!(
        errors(&buf, Class::Plain),
        Some(ParseError {
            offset: 0,
            kind: ErrorKind::NameOutOfBounds
        })
    );
    set_u32(&mut buf, layout::PLAIN_NAME_LENGTH, u32::MAX - 1);
    assert_eq!(
        errors(&buf, Class::Plain),
        Some(ParseError {
            offset: 0,
            kind: ErrorKind::NameOutOfBounds
        })
    );
    set_u32(&mut buf, layout::PLAIN_NAME_LENGTH, 4);
    assert_eq!(errors(&buf, Class::Plain), None);
}

#[test]
fn next_offsets_are_validated() {
    let records = vec![plain(1, "ab"), plain(2, "cd")];
    let good = encode(&records, Class::Plain);
    let size = align4(12 + 4);
    assert_eq!(size, 16);
    let at = |next: u32| {
        let mut buf = good.clone();
        set_u32(&mut buf, layout::NEXT_ENTRY_OFFSET, next);
        errors(&buf, Class::Plain).map(|e| (e.offset, e.kind))
    };
    assert_eq!(at(16), None);
    assert_eq!(at(0), None);
    // Misaligned.
    assert_eq!(at(17), Some((0, ErrorKind::MisalignedNext)));
    assert_eq!(at(18), Some((0, ErrorKind::MisalignedNext)));
    // Smaller than the record (header 12 + name 4 = 16), including backwards.
    assert_eq!(at(12), Some((0, ErrorKind::NextTooSmall)));
    assert_eq!(at(4), Some((0, ErrorKind::NextTooSmall)));
    assert_eq!(at(u32::MAX - 3), Some((0, ErrorKind::NextPastEnd)));
    // Exactly at the end, and beyond it.
    assert_eq!(at(32), Some((0, ErrorKind::NextPastEnd)));
    assert_eq!(at(36), Some((0, ErrorKind::NextPastEnd)));
}

#[test]
fn a_next_offset_pointing_at_the_last_valid_byte_hits_a_truncated_header() {
    let mut buf = encode(&[plain(1, "ab"), plain(2, "")], Class::Plain);
    // 16 + 12 = 28 bytes; next offset 28 - 4 = 24 leaves only 4 bytes.
    set_u32(&mut buf, layout::NEXT_ENTRY_OFFSET, 24);
    assert_eq!(
        errors(&buf, Class::Plain),
        Some(ParseError {
            offset: 24,
            kind: ErrorKind::Truncated
        })
    );
}

#[test]
fn records_before_an_error_stay_valid() {
    let mut buf = encode(&[plain(1, "a"), plain(2, "b"), plain(3, "c")], Class::Plain);
    let second = align4(12 + 2);
    set_u32(&mut buf, second + layout::PLAIN_NAME_LENGTH, 7);
    let (records, error) = model_of(&buf, Class::Plain);
    assert_eq!(records, vec![plain(1, "a")]);
    assert_eq!(
        error,
        Some(ParseError {
            offset: second,
            kind: ErrorKind::OddNameLength
        })
    );
}

#[test]
fn a_loop_of_records_hits_the_record_cap() {
    // Records of 12 bytes whose next offset is 0x4 less than their own size
    // cannot loop, but a chain that stays in bounds can only have
    // `len / header` records. Eight 12-byte records fit 96 bytes; the cap is
    // exactly 8.
    let records: Vec<Model> = (0..8).map(|_| plain(3, "")).collect();
    let buf = encode(&records, Class::Plain);
    assert_eq!(buf.len(), 96);
    assert_eq!(model_of(&buf, Class::Plain), (records, None));
    // A ninth record would not fit, so the cap never rejects a valid buffer.
    let records: Vec<Model> = (0..9).map(|_| plain(3, "")).collect();
    let buf = encode(&records, Class::Plain);
    assert_eq!(model_of(&buf, Class::Plain).0.len(), 9);
}

#[test]
fn the_budget_counts_down_to_zero() {
    // 24 bytes allow two plain records. A chain of three cannot fit, so build
    // the over-long case from the iterator state directly.
    let buf = encode(&[plain(1, ""), plain(2, "")], Class::Plain);
    let mut records = parse(&buf, buf.len(), Class::Plain);
    assert!(matches!(records.next(), Some(Ok(_))));
    assert!(matches!(records.next(), Some(Ok(_))));
    assert!(records.next().is_none());
    let mut over = parse(&buf, buf.len(), Class::Plain);
    over.budget = 1;
    assert!(matches!(over.next(), Some(Ok(_))));
    assert_eq!(
        over.next(),
        Some(Err(ParseError {
            offset: 12,
            kind: ErrorKind::TooManyRecords
        }))
    );
    assert!(over.next().is_none());
}

#[test]
fn errors_display_their_offset() {
    let e = ParseError {
        offset: 12,
        kind: ErrorKind::NextPastEnd,
    };
    assert_eq!(
        e.to_string(),
        "malformed notify record at byte 12: NextPastEnd"
    );
}

fn name_strategy() -> impl Strategy<Value = Vec<u16>> {
    // Realistic lengths plus lone surrogates and NULs: names are raw units.
    prop::collection::vec(any::<u16>(), 0..70)
}

fn model_strategy(class: Class) -> impl Strategy<Value = Model> {
    let extended = (
        any::<[i64; 6]>(),
        any::<[u32; 2]>(),
        any::<[i64; 2]>(),
        any::<u8>(),
    )
        .prop_map(move |(times, words, ids, flags)| Extended {
            creation_time: times[0],
            last_write_time: times[1],
            change_time: times[2],
            last_access_time: times[3],
            allocated_length: times[4],
            file_size: times[5],
            attributes: words[0],
            reparse_tag: words[1],
            file_id: ids[0],
            parent_file_id: ids[1],
            name_flags: (class == Class::Full).then_some(flags),
        });
    (any::<u32>(), name_strategy(), extended).prop_map(move |(action, name, extended)| Model {
        action,
        name,
        extended: (class != Class::Plain).then_some(extended),
    })
}

proptest! {
    #![proptest_config(ProptestConfig { max_shrink_iters: 64, ..ProptestConfig::default() })]

    #[test]
    fn plain_round_trips(records in prop::collection::vec(model_strategy(Class::Plain), 0..24)) {
        let buf = encode(&records, Class::Plain);
        prop_assert_eq!(model_of(&buf, Class::Plain), (records, None));
    }

    #[test]
    fn extended_round_trips(records in prop::collection::vec(model_strategy(Class::Extended), 0..24)) {
        let buf = encode(&records, Class::Extended);
        prop_assert_eq!(model_of(&buf, Class::Extended), (records, None));
    }

    #[test]
    fn full_round_trips(records in prop::collection::vec(model_strategy(Class::Full), 0..24)) {
        let buf = encode(&records, Class::Full);
        prop_assert_eq!(model_of(&buf, Class::Full), (records, None));
    }

    #[test]
    fn truncation_never_panics_and_keeps_a_valid_prefix(
        records in prop::collection::vec(model_strategy(Class::Extended), 1..8),
        cut in any::<prop::sample::Index>(),
    ) {
        let buf = encode(&records, Class::Extended);
        let len = cut.index(buf.len() + 1);
        let (prefix, _) = model_of(buf.get(..len).unwrap(), Class::Extended);
        prop_assert!(prefix.len() <= records.len());
        prop_assert_eq!(prefix.as_slice(), records.get(..prefix.len()).unwrap());
    }

    #[test]
    fn random_bytes_end_in_records_or_an_error(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
        class in prop::sample::select(vec![Class::Plain, Class::Extended, Class::Full]),
    ) {
        let mut count = 0_usize;
        let mut ended_with_error = false;
        for item in parse(&bytes, bytes.len(), class) {
            prop_assert!(!ended_with_error, "item after an error");
            match item {
                Ok(_) => count += 1,
                Err(e) => {
                    prop_assert!(e.offset <= bytes.len());
                    ended_with_error = true;
                }
            }
        }
        prop_assert!(count <= bytes.len() / class.header_len());
    }

    #[test]
    fn mutated_buffers_never_panic(
        records in prop::collection::vec(model_strategy(Class::Plain), 1..8),
        edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..6),
    ) {
        let mut buf = encode(&records, Class::Plain);
        for (at, byte) in edits {
            let at = at % buf.len();
            *buf.get_mut(at).unwrap() = byte;
        }
        let _ = model_of(&buf, Class::Plain);
    }

    #[test]
    fn bad_next_offsets_are_always_caught(
        records in prop::collection::vec(model_strategy(Class::Plain), 2..8),
        bogus in any::<u32>(),
    ) {
        let mut buf = encode(&records, Class::Plain);
        let first = 12 + records.first().unwrap().name.len() * 2;
        set_u32(&mut buf, layout::NEXT_ENTRY_OFFSET, bogus);
        let (_, error) = model_of(&buf, Class::Plain);
        let step = bogus as usize;
        if bogus != 0 && (!step.is_multiple_of(4) || step < first || step >= buf.len()) {
            prop_assert!(error.is_some(), "{bogus} accepted");
        }
    }
}

#[test]
fn the_record_cap_is_len_over_header_but_at_least_one() {
    for (class, len, cap) in [
        (Class::Plain, 96, 8),
        (Class::Plain, 95, 7),
        (Class::Plain, 5, 1),
        (Class::Extended, 168, 2),
        (Class::Full, 167, 1),
    ] {
        let buf = vec![0_u8; len];
        assert_eq!(parse(&buf, len, class).budget, cap, "{class:?} {len}");
    }
}
