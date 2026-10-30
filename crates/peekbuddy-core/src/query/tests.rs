use super::*;

#[test]
fn an_empty_filter_is_empty() {
    assert!(Filter::default().is_empty());
    let all = Filter {
        kinds: Some(KindSet::ALL),
        ..Filter::default()
    };
    assert!(all.is_empty());
    let some = Filter {
        kinds: Some(KindSet::of(&[Kind::Created])),
        ..Filter::default()
    };
    assert!(!some.is_empty());
}

#[test]
fn like_literal_escapes_wildcards() {
    assert_eq!(like_literal(r"a%b_c\d"), r"%a\%b\_c\\d%");
    assert_eq!(like_literal(""), "%%");
}

#[test]
fn fts_phrase_doubles_quotes() {
    assert_eq!(fts_phrase("say \"hi\""), "\"say \"\"hi\"\"\"");
}

#[test]
fn indexable_counts_characters_not_bytes() {
    assert!(!is_indexable("äö"));
    assert!(is_indexable("äöü"));
}

#[test]
fn cancel_flag_is_shared_between_clones() {
    let flag = CancelFlag::new();
    let other = flag.clone();
    assert!(!other.is_cancelled());
    flag.cancel();
    assert!(other.is_cancelled());
}

#[test]
fn page_ids_follow_display_order() {
    assert_eq!(Page::default().newest_id(), None);
    let req = PageRequest::newest(3, Filter::default());
    assert_eq!((req.generation, req.limit), (3, PAGE_SIZE));
}

#[test]
fn kind_set_tags_are_ascending() {
    let set = KindSet::of(&[Kind::Deleted, Kind::Created]);
    assert_eq!(set.tags(), vec![0, 2]);
    assert!(KindSet::ALL.is_all());
    assert!(!set.is_all());
}
