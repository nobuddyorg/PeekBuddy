use proptest::prelude::*;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Model {
    name: Vec<u16>,
    attributes: u32,
    reparse_tag: u32,
    size: i64,
    write_time: i64,
    change_time: i64,
    file_id: u128,
}

fn put(out: &mut [u8], at: usize, bytes: &[u8]) {
    if let Some(dst) = out.get_mut(at..at + bytes.len()) {
        dst.copy_from_slice(bytes);
    }
}

/// Lays records out the way the kernel does: 8-byte aligned,
/// `NextEntryOffset` 0 on the last.
fn encode(records: &[Model]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let start = out.len();
        let name_bytes = record.name.len() * 2;
        let size = (HEADER_LEN + name_bytes).next_multiple_of(ALIGN);
        out.resize(start + size, 0);
        let next = if index + 1 == records.len() { 0 } else { size };
        let at = |offset: usize| start + offset;
        put(
            &mut out,
            at(layout::NEXT_ENTRY_OFFSET),
            &u32::try_from(next).unwrap().to_le_bytes(),
        );
        put(
            &mut out,
            at(layout::WRITE_TIME),
            &record.write_time.to_le_bytes(),
        );
        put(
            &mut out,
            at(layout::CHANGE_TIME),
            &record.change_time.to_le_bytes(),
        );
        put(
            &mut out,
            at(layout::END_OF_FILE),
            &record.size.to_le_bytes(),
        );
        put(
            &mut out,
            at(layout::ATTRIBUTES),
            &record.attributes.to_le_bytes(),
        );
        put(
            &mut out,
            at(layout::NAME_LENGTH),
            &u32::try_from(name_bytes).unwrap().to_le_bytes(),
        );
        put(
            &mut out,
            at(layout::REPARSE_TAG),
            &record.reparse_tag.to_le_bytes(),
        );
        put(&mut out, at(layout::FILE_ID), &record.file_id.to_le_bytes());
        for (i, unit) in record.name.iter().enumerate() {
            put(&mut out, at(layout::NAME + i * 2), &unit.to_le_bytes());
        }
    }
    out
}

fn model(name: &str, id: u128) -> Model {
    Model {
        name: name.encode_utf16().collect(),
        attributes: 0x20,
        reparse_tag: 0,
        size: 123,
        write_time: 456,
        change_time: 789,
        file_id: id,
    }
}

fn view(bytes: &[u8]) -> Vec<Result<Model, ParseError>> {
    parse(bytes, bytes.len())
        .map(|item| {
            item.map(|e| Model {
                name: e.name.units().collect(),
                attributes: e.attributes,
                reparse_tag: e.reparse_tag,
                size: e.size,
                write_time: e.write_time,
                change_time: e.change_time,
                file_id: e.file_id,
            })
        })
        .collect()
}

#[test]
fn records_round_trip_including_128_bit_ids() {
    let records = vec![
        model(".", 1),
        model("..", 2),
        model("héllo wörld.txt", u128::MAX - 7),
        model("a", 1 << 100),
    ];
    let parsed: Vec<Model> = view(&encode(&records))
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(parsed, records);
}

#[test]
fn dots_and_flags_are_recognised() {
    let bytes = encode(&[
        model(".", 1),
        model("..", 2),
        model(".x", 3),
        model("...", 4),
    ]);
    let dots: Vec<bool> = parse(&bytes, bytes.len())
        .map(|e| e.unwrap().is_dot())
        .collect();
    assert_eq!(dots, [true, true, false, false]);
    let mut dir = model("d", 5);
    dir.attributes = ATTR_DIRECTORY | ATTR_REPARSE_POINT;
    let bytes = encode(&[dir]);
    let entry = parse(&bytes, bytes.len()).next().unwrap().unwrap();
    assert!(entry.is_dir() && entry.is_reparse());
}

#[test]
fn empty_and_oversized_lengths() {
    assert_eq!(parse(&[], 0).count(), 0);
    let bytes = encode(&[model("a", 1)]);
    let items: Vec<_> = parse(&bytes, bytes.len() + 1).collect();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items.first().unwrap().unwrap_err().kind,
        ErrorKind::BadLength
    );
}

fn fault(mutate: impl FnOnce(&mut Vec<u8>)) -> ErrorKind {
    let mut bytes = encode(&[model("abc", 1), model("def", 2)]);
    mutate(&mut bytes);
    let results = view(&bytes);
    results
        .into_iter()
        .find_map(Result::err)
        .map(|e| e.kind)
        .unwrap()
}

#[test]
fn every_malformed_shape_is_an_error_not_a_panic() {
    assert_eq!(fault(|b| b.truncate(HEADER_LEN - 1)), ErrorKind::Truncated);
    assert_eq!(
        fault(|b| put(b, layout::NAME_LENGTH, &3_u32.to_le_bytes())),
        ErrorKind::OddNameLength
    );
    assert_eq!(
        fault(|b| put(b, layout::NAME_LENGTH, &4000_u32.to_le_bytes())),
        ErrorKind::NameOutOfBounds
    );
    assert_eq!(
        fault(|b| put(
            b,
            layout::NEXT_ENTRY_OFFSET,
            &u32::try_from(HEADER_LEN + 4).unwrap().to_le_bytes()
        )),
        ErrorKind::MisalignedNext
    );
    assert_eq!(
        fault(|b| put(b, layout::NEXT_ENTRY_OFFSET, &8_u32.to_le_bytes())),
        ErrorKind::NextTooSmall
    );
    assert_eq!(
        fault(|b| put(b, layout::NEXT_ENTRY_OFFSET, &10_000_u32.to_le_bytes())),
        ErrorKind::NextPastEnd
    );
}

#[test]
fn a_record_that_points_at_itself_ends_with_too_many_records() {
    let mut bytes = encode(&[model("abc", 1)]);
    let size = (HEADER_LEN + 6).next_multiple_of(ALIGN);
    bytes.resize(size * 2, 0);
    // Next = size, then a second record whose next points back to itself.
    put(&mut bytes, 0, &u32::try_from(size).unwrap().to_le_bytes());
    put(&mut bytes, size, &0_u32.to_le_bytes());
    let kinds: Vec<_> = view(&bytes).into_iter().map(|r| r.is_ok()).collect();
    assert!(kinds.len() <= 2);
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic_and_stay_bounded(bytes in proptest::collection::vec(any::<u8>(), 0..600), claim in 0..800_usize) {
        let mut seen = 0;
        for item in parse(&bytes, claim) {
            seen += 1;
            if let Ok(entry) = item {
                prop_assert!(entry.name.len() * 2 + HEADER_LEN <= bytes.len());
            }
            prop_assert!(seen <= bytes.len() / HEADER_LEN + 2);
        }
    }

    #[test]
    fn well_formed_buffers_round_trip(
        names in proptest::collection::vec(proptest::collection::vec(any::<u16>(), 0..20), 0..12),
        ids in proptest::collection::vec(any::<u128>(), 12),
    ) {
        let records: Vec<Model> = names
            .into_iter()
            .zip(ids)
            .map(|(name, file_id)| Model {
                name,
                attributes: 0x10,
                reparse_tag: 7,
                size: -5,
                write_time: i64::MAX,
                change_time: i64::MIN,
                file_id,
            })
            .collect();
        let parsed: Vec<Model> = view(&encode(&records)).into_iter().map(Result::unwrap).collect();
        prop_assert_eq!(parsed, records);
    }
}
