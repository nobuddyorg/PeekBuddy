//! The committed seed corpus of the `notify_parse` fuzz target (#44, #270):
//! raw buffers recorded from NTFS with all three information classes. Every
//! buffer must parse without an error into at least one record, whole.

#![expect(clippy::expect_used, reason = "tests expect")]

use std::path::PathBuf;

use peekbuddy_core::notify::{Class, parse};

fn corpus() -> Vec<(String, Class, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fuzz")
        .join("corpus")
        .join("notify_parse");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("the corpus directory") {
        let path = entry.expect("an entry").path();
        let name = path
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let bytes = std::fs::read(&path).expect("a readable seed");
        let Some((&head, buffer)) = bytes.split_first() else {
            continue;
        };
        let class = match head % 3 {
            0 => Class::Plain,
            1 => Class::Extended,
            _ => Class::Full,
        };
        out.push((name, class, buffer.to_vec()));
    }
    out
}

#[test]
fn every_recorded_buffer_parses_whole() {
    let seeds: Vec<_> = corpus()
        .into_iter()
        .filter(|(name, _, _)| {
            name.starts_with("plain_") || name.starts_with("extended_") || name.starts_with("full_")
        })
        .collect();
    for (name, class, buffer) in &seeds {
        let records: Vec<_> = parse(buffer, buffer.len(), *class).collect();
        assert!(!records.is_empty(), "{name}: no record");
        assert!(records.iter().all(Result::is_ok), "{name}: {records:?}");
    }
}

#[test]
fn the_corpus_has_real_buffers_of_all_three_classes() {
    let seeds = corpus();
    for (prefix, class) in [
        ("plain_", Class::Plain),
        ("extended_", Class::Extended),
        ("full_", Class::Full),
    ] {
        let of_class = seeds
            .iter()
            .filter(|(name, found, _)| name.starts_with(prefix) && *found == class)
            .count();
        assert!(of_class >= 10, "{prefix}: only {of_class} seeds");
    }
}

#[test]
fn the_richer_classes_carry_what_plain_cannot() {
    // The same operation, recorded in the extended class, is larger than in
    // the plain one: it adds ids, sizes, attributes and the reparse tag.
    let seeds = corpus();
    let size = |name: &str| {
        seeds
            .iter()
            .find(|(found, _, _)| found == name)
            .map(|(_, _, buffer)| buffer.len())
            .expect("a seed")
    };
    assert!(size("extended_create") > size("plain_create"));
}
