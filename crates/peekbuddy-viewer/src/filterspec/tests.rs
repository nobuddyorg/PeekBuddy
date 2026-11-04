use std::collections::BTreeSet;

use peekbuddy_core::event::{Flags, GapReason, Kind};
use proptest::prelude::*;

use super::{Chip, Field, FilterSpec, ParseError, PatternKind, Span};

/// 2026-10-26 12:00:00 UTC in FILETIME ticks.
const NOON: u64 = 134_374_896_000_000_000;

fn parse(text: &str) -> FilterSpec {
    FilterSpec::parse(text).unwrap()
}

fn error(text: &str) -> ParseError {
    FilterSpec::parse(text).unwrap_err()
}

#[test]
fn the_example_of_the_issue_reads_and_prints_back() {
    // A backslash is written twice inside quotes, as the printer does.
    let text = "name:\"report\" kind:created,deleted root:\"C:\\\\work\" \
                in:\"C:\\\\work\\\\src\" since:1h glob:\"*.tmp\" hidden:show";
    let spec = parse(text);
    assert_eq!(spec.name, "report");
    assert_eq!(spec.kinds, BTreeSet::from([Chip::Created, Chip::Deleted]));
    assert_eq!(spec.root.as_deref(), Some("C:\\work"));
    assert_eq!(spec.subtree.as_deref(), Some("C:\\work\\src"));
    assert_eq!(spec.span, Span::LastHour);
    assert_eq!(
        spec.pattern,
        Some((PatternKind::Glob, String::from("*.tmp")))
    );
    assert!(spec.show_hidden);
    assert_eq!(spec.to_text(), text);
}

#[test]
fn nothing_is_the_empty_text_and_the_default() {
    assert_eq!(FilterSpec::default().to_text(), "");
    assert!(parse("").is_default());
    assert!(parse("   \t ").is_default());
}

#[test]
fn quotes_and_backslashes_survive() {
    let spec = FilterSpec {
        name: String::from("say \"hi\" \\ there"),
        ..FilterSpec::default()
    };
    assert_eq!(spec.to_text(), "name:\"say \\\"hi\\\" \\\\ there\"");
    assert_eq!(parse(&spec.to_text()), spec);
}

#[test]
fn bare_words_are_accepted_for_simple_values() {
    let spec = parse("name:report kind:modified since:today");
    assert_eq!(spec.name, "report");
    assert_eq!(spec.span, Span::Today);
}

#[test]
fn a_custom_range_uses_utc_times() {
    let spec = FilterSpec {
        span: Span::Between {
            from: Some(NOON),
            to: Some(NOON + 3_600 * 10_000_000),
        },
        ..FilterSpec::default()
    };
    let text = spec.to_text();
    assert_eq!(
        text,
        "from:2026-10-26T12:00:00.0000000Z to:2026-10-26T13:00:00.0000000Z"
    );
    assert_eq!(parse(&text), spec);
    // Either end alone is fine.
    assert!(matches!(
        parse("to:2026-10-26T12:00:00.0000000Z").span,
        Span::Between {
            from: None,
            to: Some(_)
        }
    ));
}

#[test]
fn problems_are_positioned_and_in_words() {
    let cases: &[(&str, usize, &str)] = &[
        ("bogus:x", 0, "unknown key `bogus`"),
        ("name:\"open", 5, "never closed"),
        ("name:", 5, "value is missing"),
        ("hello", 0, "expected a key"),
        ("kind:created,sparkly", 5, "unknown kind `sparkly`"),
        ("name:a name:b", 7, "given twice"),
        ("since:yesterday", 6, "takes 5m, 1h or today"),
        ("from:soon", 5, "takes a UTC time"),
        ("glob:\"a\" re:\"b\"", 9, "not both"),
        ("hidden:no", 7, "takes show"),
        (
            "since:1h from:2026-10-26T12:00:00.0000000Z",
            0,
            "exclude each other",
        ),
        ("name:\"a\\x\"", 7, "must be followed by"),
    ];
    for (text, at, message) in cases {
        let found = error(text);
        assert!(found.message.contains(message), "{text}: {found}");
        assert_eq!(found.at, *at, "{text}: {found}");
    }
}

#[test]
fn an_empty_pattern_is_no_pattern() {
    assert_eq!(parse("glob:\"\"").pattern, None);
    assert_eq!(parse("re:\"\"").pattern, None);
}

#[test]
fn chips_become_kind_sets_by_class() {
    let spec = FilterSpec {
        kinds: BTreeSet::from([Chip::Created, Chip::Gaps]),
        ..FilterSpec::default()
    };
    let set = spec.to_filter(NOON, 0).unwrap().kinds.unwrap();
    assert!(set.contains(Kind::Created));
    assert!(set.contains(Kind::ReconciledCreated));
    assert!(
        set.contains(Kind::Overflow(GapReason::RingFull)),
        "any reason"
    );
    assert!(!set.contains(Kind::Deleted));
    assert!(!set.contains(Kind::EngineStarted));
    // No chip means every kind.
    assert!(
        FilterSpec::default()
            .to_filter(NOON, 0)
            .unwrap()
            .kinds
            .is_none()
    );
    // Every chip stands for something.
    for chip in Chip::ALL {
        assert!(!chip.kinds().is_empty(), "{chip:?}");
        assert_ne!(chip.label().len(), 0);
    }
}

#[test]
fn the_default_view_hides_cloud_churn_and_the_switch_shows_it() {
    let hidden = FilterSpec::default().to_filter(NOON, 0).unwrap();
    assert_eq!(hidden.hide, Flags::CLOUD_CHURN);
    let shown = FilterSpec {
        show_hidden: true,
        ..FilterSpec::default()
    }
    .to_filter(NOON, 0)
    .unwrap();
    assert_eq!(shown.hide, Flags::NONE);
}

#[test]
fn spans_become_time_ranges() {
    let at = |span| {
        FilterSpec {
            span,
            ..FilterSpec::default()
        }
        .to_filter(NOON, 7_200)
        .unwrap()
        .time
    };
    assert_eq!(at(Span::Any).from, None);
    assert_eq!(at(Span::Last5Minutes).from, Some(NOON - 300 * 10_000_000));
    assert_eq!(at(Span::LastHour).from, Some(NOON - 3_600 * 10_000_000));
    // 14:00 local (UTC+2): local midnight is 22:00 UTC the day before.
    assert_eq!(at(Span::Today).from, Some(134_374_392_000_000_000));
    let west = FilterSpec {
        span: Span::Today,
        ..FilterSpec::default()
    }
    .to_filter(NOON, -18_000)
    .unwrap();
    // 07:00 local (UTC-5): local midnight is 05:00 UTC.
    assert_eq!(west.time.from, Some(134_374_644_000_000_000));
}

#[test]
fn a_range_that_ends_before_it_starts_is_refused_at_the_span() {
    let spec = FilterSpec {
        span: Span::Between {
            from: Some(NOON),
            to: Some(NOON - 1),
        },
        ..FilterSpec::default()
    };
    let error = spec.to_filter(NOON, 0).unwrap_err();
    assert_eq!(error.field, Field::Span);
    assert!(error.message.contains("before"));
}

#[test]
fn an_invalid_regex_or_glob_is_refused_at_the_pattern_and_never_panics() {
    for (kind, text) in [
        (PatternKind::Regex, "(unclosed"),
        (PatternKind::Regex, "a{2,1}"),
        (PatternKind::Glob, "[abc"),
    ] {
        let spec = FilterSpec {
            pattern: Some((kind, text.to_owned())),
            ..FilterSpec::default()
        };
        let error = spec.to_filter(NOON, 0).unwrap_err();
        assert_eq!(error.field, Field::Pattern, "{text}");
        assert_ne!(error.message.len(), 0);
    }
    // A good one compiles and filters.
    let good = FilterSpec {
        pattern: Some((PatternKind::Glob, String::from("*.tmp"))),
        ..FilterSpec::default()
    };
    assert!(good.to_filter(NOON, 0).unwrap().pattern.is_some());
}

#[test]
fn paths_and_names_reach_the_filter_as_the_reader_wants_them() {
    let spec = FilterSpec {
        name: String::from("rep"),
        root: Some(String::from("C:\\work")),
        subtree: Some(String::from("C:\\work\\src")),
        ..FilterSpec::default()
    };
    let filter = spec.to_filter(NOON, 0).unwrap();
    assert_eq!(filter.name.as_deref(), Some("rep"));
    assert_eq!(
        filter.root,
        Some("C:\\work".encode_utf16().collect::<Vec<u16>>())
    );
    assert_eq!(
        filter.subtree,
        Some("C:\\work\\src".encode_utf16().collect::<Vec<u16>>())
    );
}

#[test]
fn an_empty_result_names_the_narrowest_filter() {
    assert_eq!(FilterSpec::default().narrowest(), None);
    let spec = parse("name:x kind:created since:1h");
    assert_eq!(spec.narrowest().as_deref(), Some("the name \"x\""));
    let spec = parse("kind:created since:1h");
    assert_eq!(spec.narrowest().as_deref(), Some("the kinds chosen"));
    let spec = parse("since:1h");
    assert_eq!(spec.narrowest().as_deref(), Some("the time span"));
    let spec = parse("glob:\"*.x\" name:y");
    assert_eq!(spec.narrowest().as_deref(), Some("the glob *.x"));
}

fn arb_text() -> impl Strategy<Value = String> {
    // Quotes, backslashes, spaces and a few non-ASCII characters.
    proptest::collection::vec(
        prop_oneof![
            Just('"'),
            Just('\\'),
            Just(' '),
            Just(':'),
            Just('é'),
            Just('漢'),
            proptest::char::range('!', '~'),
        ],
        1..24,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn arb_spec() -> impl Strategy<Value = FilterSpec> {
    let chips = proptest::collection::btree_set(
        proptest::sample::select(Chip::ALL.to_vec()),
        0..=Chip::ALL.len(),
    );
    let span = prop_oneof![
        Just(Span::Any),
        Just(Span::Last5Minutes),
        Just(Span::LastHour),
        Just(Span::Today),
        (0u64..30_000_000_000_000_000, 0u64..30_000_000_000_000_000).prop_map(|(from, to)| {
            Span::Between {
                from: Some(from),
                to: Some(to),
            }
        }),
        (0u64..30_000_000_000_000_000).prop_map(|from| Span::Between {
            from: Some(from),
            to: None
        }),
        (0u64..30_000_000_000_000_000).prop_map(|to| Span::Between {
            from: None,
            to: Some(to)
        }),
    ];
    let pattern = proptest::option::of((
        prop_oneof![Just(PatternKind::Glob), Just(PatternKind::Regex)],
        arb_text(),
    ));
    (
        prop_oneof![Just(String::new()), arb_text()],
        chips,
        proptest::option::of(arb_text()),
        proptest::option::of(arb_text()),
        span,
        pattern,
        any::<bool>(),
    )
        .prop_map(
            |(name, kinds, root, subtree, span, pattern, show_hidden)| FilterSpec {
                name,
                kinds,
                root,
                subtree,
                span,
                pattern,
                show_hidden,
            },
        )
}

proptest! {
    #[test]
    fn printing_then_parsing_gives_the_same_spec(spec in arb_spec()) {
        let text = spec.to_text();
        let back = FilterSpec::parse(&text);
        prop_assert_eq!(back, Ok(spec), "{}", text);
    }

    #[test]
    fn the_text_form_is_stable(spec in arb_spec()) {
        let text = spec.to_text();
        let again = FilterSpec::parse(&text).unwrap().to_text();
        prop_assert_eq!(text, again);
    }

    #[test]
    fn arbitrary_text_never_panics_and_a_good_reading_prints_back(text in "\\PC{0,200}") {
        if let Ok(spec) = FilterSpec::parse(&text) {
            prop_assert_eq!(FilterSpec::parse(&spec.to_text()), Ok(spec));
        }
    }

    #[test]
    fn arbitrary_filter_text_with_the_right_alphabet_never_panics(
        text in "[a-z:\"\\\\ ,0-9TZ.-]{0,120}"
    ) {
        let _ = FilterSpec::parse(&text);
    }
}

#[test]
fn chips_have_the_words_of_the_bar_and_find_their_kinds() {
    let labels: Vec<&str> = Chip::ALL.iter().map(|chip| chip.label()).collect();
    assert_eq!(
        labels,
        [
            "Created",
            "Modified",
            "Deleted",
            "Renamed",
            "Attributes",
            "Gaps",
            "Engine"
        ]
    );
    assert_eq!(Chip::of(Kind::Created), Some(Chip::Created));
    assert_eq!(Chip::of(Kind::Modified), Some(Chip::Modified));
    assert_eq!(Chip::of(Kind::Deleted), Some(Chip::Deleted));
    assert_eq!(
        Chip::of(Kind::Overflow(GapReason::RingFull)),
        Some(Chip::Gaps)
    );
    assert_eq!(Chip::of(Kind::EngineStarted), Some(Chip::Engine));
}

#[test]
fn today_starts_at_local_midnight_even_when_the_offset_crosses_the_date_line() {
    let today = |now: u64, offset: i32| {
        FilterSpec {
            span: Span::Today,
            ..FilterSpec::default()
        }
        .to_filter(now, offset)
        .unwrap()
        .time
        .from
        .unwrap()
    };
    let hour = 3_600 * 10_000_000;
    // 23:00 UTC on the 26th is 01:00 on the 27th at UTC+2: local midnight of
    // the 27th is 22:00 UTC on the 26th.
    let midnight_utc = NOON - 12 * hour;
    assert_eq!(
        today(midnight_utc + 23 * hour, 7_200),
        midnight_utc + 22 * hour
    );
    // 01:00 UTC on the 26th is 20:00 on the 25th at UTC-5: local midnight of
    // the 25th is 05:00 UTC on the 25th.
    assert_eq!(
        today(midnight_utc + hour, -18_000),
        midnight_utc - 19 * hour
    );
}

#[test]
fn a_range_of_one_instant_is_fine_and_one_that_ends_first_is_not() {
    let range = |from, to| {
        FilterSpec {
            span: Span::Between {
                from: Some(from),
                to: Some(to),
            },
            ..FilterSpec::default()
        }
        .to_filter(NOON, 0)
    };
    assert!(range(NOON, NOON).is_ok(), "from equal to to is allowed");
    assert!(range(NOON, NOON - 1).is_err());
}
