#![expect(clippy::indexing_slicing, reason = "tests index buffers they built")]

use proptest::prelude::*;

use super::encode::{Spec, buffer, record, record_v4};
use super::*;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn spec(major: u16, frn: u128, name: &str) -> Spec {
    Spec {
        major,
        frn,
        parent: 5,
        usn: 100,
        timestamp: 133_000_000_000_000_000,
        reason: 0x100,
        source_info: 0,
        attributes: 0x20,
        name: units(name),
    }
}

fn decode(buf: &[u8]) -> (Vec<Record<'_>>, Option<DecodeError>) {
    let mut iter = records(buf).unwrap();
    let mut out = Vec::new();
    let mut error = None;
    for item in iter.by_ref() {
        match item {
            Ok(rec) => out.push(rec),
            Err(e) => error = Some(e),
        }
    }
    (out, error)
}

#[test]
fn a_buffer_with_only_the_header_has_no_records() {
    let buf = buffer(777, &[]);
    let iter = records(&buf).unwrap();
    assert_eq!(iter.next_usn(), 777);
    assert_eq!(iter.count(), 0);
}

#[test]
fn a_buffer_shorter_than_the_header_is_an_error() {
    assert_eq!(records(&[1, 2, 3]).unwrap_err(), DecodeError::NoHeader);
}

#[test]
fn v2_and_v3_records_decode_with_widened_numbers() {
    let a = spec(2, 0x0005_0000_0000_0042, "a.txt");
    let b = spec(3, 0x0007_0000_0000_0000_0000_0000_0000_0042, "b.txt");
    let buf = buffer(9, &[record(&a), record(&b)]);
    let (recs, error) = decode(&buf);
    assert_eq!(error, None);
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0].major, 2);
    assert_eq!(recs[0].frn, a.frn);
    assert_eq!(recs[0].name_units(), units("a.txt"));
    assert_eq!(recs[1].major, 3);
    assert_eq!(recs[1].frn, b.frn);
    assert_eq!(recs[1].usn, 100);
}

#[test]
fn v4_records_are_skipped_and_counted() {
    let a = spec(3, 1, "a");
    let b = spec(3, 2, "b");
    let buf = buffer(0, &[record(&a), record_v4(), record(&b), record_v4()]);
    let mut iter = records(&buf).unwrap();
    let frns: Vec<u128> = iter.by_ref().map(|r| r.unwrap().frn).collect();
    assert_eq!(frns, [1, 2]);
    assert_eq!(iter.skipped_v4(), 2);
}

#[test]
fn an_empty_name_is_withheld() {
    let buf = buffer(0, &[record(&spec(3, 1, ""))]);
    let (recs, _) = decode(&buf);
    assert!(recs[0].name_withheld());
    assert_eq!(recs[0].name_units(), Vec::<u16>::new());
}

#[test]
fn damage_yields_the_records_before_it_and_one_error() {
    let good = record(&spec(3, 1, "ok"));
    let mut broken = record(&spec(3, 2, "bad"));
    broken[0..4].copy_from_slice(&1_000_000u32.to_le_bytes());
    let buf = buffer(0, &[good, broken, record(&spec(3, 3, "after"))]);
    let (recs, error) = decode(&buf);
    assert_eq!(recs.len(), 1);
    assert!(matches!(error, Some(DecodeError::BadLength { .. })));
}

#[test]
fn an_unknown_version_stops_the_walk() {
    let mut odd = record(&spec(3, 2, "x"));
    odd[4..6].copy_from_slice(&9u16.to_le_bytes());
    let buf = buffer(0, &[record(&spec(2, 1, "a")), odd]);
    let (recs, error) = decode(&buf);
    assert_eq!(recs.len(), 1);
    assert!(matches!(
        error,
        Some(DecodeError::UnknownVersion { major: 9, .. })
    ));
}

#[test]
fn a_name_outside_the_record_is_an_error() {
    let mut rec = record(&spec(3, 2, "name"));
    rec[74..76].copy_from_slice(&200u16.to_le_bytes());
    let buf = buffer(0, &[rec]);
    let (recs, error) = decode(&buf);
    assert_eq!(recs.len(), 0);
    assert!(matches!(error, Some(DecodeError::BadName { .. })));
}

fn arb_spec() -> impl Strategy<Value = Spec> {
    (
        prop_oneof![Just(2_u16), Just(3_u16)],
        any::<u128>(),
        any::<u128>(),
        any::<i64>(),
        any::<i64>(),
        any::<u32>(),
        any::<u32>(),
        any::<u32>(),
        proptest::collection::vec(any::<u16>(), 0..40),
    )
        .prop_map(
            |(major, frn, parent, usn, timestamp, reason, source_info, attributes, name)| {
                // Version 2 numbers are 64-bit.
                let narrow = |v: u128| {
                    if major == 2 {
                        v & u128::from(u64::MAX)
                    } else {
                        v
                    }
                };
                Spec {
                    major,
                    frn: narrow(frn),
                    parent: narrow(parent),
                    usn,
                    timestamp,
                    reason,
                    source_info,
                    attributes,
                    name,
                }
            },
        )
}

proptest! {
    #[test]
    fn records_round_trip_with_v4_records_between(
        specs in proptest::collection::vec(arb_spec(), 0..12),
        v4_after in proptest::collection::vec(any::<bool>(), 12),
        next in any::<i64>(),
    ) {
        let mut parts = Vec::new();
        for (spec, &v4) in specs.iter().zip(&v4_after) {
            parts.push(record(spec));
            if v4 {
                parts.push(record_v4());
            }
        }
        let buf = buffer(next, &parts);
        let mut iter = records(&buf).unwrap();
        prop_assert_eq!(iter.next_usn(), next);
        let got: Vec<Spec> = iter
            .by_ref()
            .map(|r| {
                let r = r.unwrap();
                Spec {
                    major: r.major,
                    frn: r.frn,
                    parent: r.parent,
                    usn: r.usn,
                    timestamp: r.timestamp,
                    reason: r.reason,
                    source_info: r.source_info,
                    attributes: r.attributes,
                    name: r.name_units(),
                }
            })
            .collect();
        prop_assert_eq!(got, specs);
    }

    #[test]
    fn adversarial_buffers_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        if let Ok(iter) = records(&bytes) {
            for item in iter {
                let _ = item.map(|r| (r.name_units(), r.is_dir()));
            }
        }
    }

    #[test]
    fn a_mutated_valid_buffer_never_panics(
        specs in proptest::collection::vec(arb_spec(), 1..6),
        at in any::<prop::sample::Index>(),
        value in any::<u8>(),
    ) {
        let parts: Vec<Vec<u8>> = specs.iter().map(record).collect();
        let mut buf = buffer(1, &parts);
        let index = at.index(buf.len());
        buf[index] = value;
        if let Ok(iter) = records(&buf) {
            for item in iter {
                let _ = item.map(|r| r.name_units());
            }
        }
    }
}
