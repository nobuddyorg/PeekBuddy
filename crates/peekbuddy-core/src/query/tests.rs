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

fn row(id: i64) -> Row {
    Row {
        id,
        ts: 0,
        kind: Kind::Created,
        source: Source::Rdcw,
        raw_mask: 0,
        is_dir: false,
        size: None,
        attrs: None,
        reparse_tag: 0,
        file_id: 0,
        name: Vec::new(),
        path: Vec::new(),
        old_path: Vec::new(),
        root: Vec::new(),
        flags: 0,
    }
}

#[test]
fn a_page_names_its_newest_and_oldest_rows() {
    let empty = Page::default();
    assert_eq!((empty.newest_id(), empty.oldest_id()), (None, None));
    let one = Page {
        rows: vec![row(7)],
        ..Page::default()
    };
    assert_eq!((one.newest_id(), one.oldest_id()), (Some(7), Some(7)));
    let three = Page {
        rows: vec![row(30), row(20), row(10)],
        ..Page::default()
    };
    assert_eq!((three.newest_id(), three.oldest_id()), (Some(30), Some(10)));
}

#[test]
fn the_default_view_hides_cloud_churn_and_nothing_else() {
    let view = Filter::default_view();
    assert_eq!(view.hide, Flags::CLOUD_CHURN);
    assert_ne!(view.hide, Flags::NONE);
    assert!(!view.is_empty());
    let unhidden = Filter {
        hide: Flags::NONE,
        ..view
    };
    assert!(unhidden.is_empty());
}
