//! Tests of the filter engine: rule tables, a naive reference matcher,
//! case rules and the drop counters.

use proptest::prelude::*;
use rstest::rstest;

use super::*;
use crate::event::tests::sample;

fn w(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn event(root: u16, kind: Kind, is_dir: bool, flags: Flags) -> Event {
    Event {
        root: RootIdx(root),
        kind,
        is_dir,
        flags,
        ..sample()
    }
}

/// Evaluates `path` against `rules` (compiled with `options`).
fn run(rules: &[Rule], options: &CompileOptions, path: &str, ev: &Event) -> Verdict {
    let set = FilterSet::compile_with(rules, options).unwrap();
    let units = w(path);
    set.eval(ev, WideStr::new(&units), &mut Scratch::new())
}

fn glob_hits(pattern: &str, path: &str, is_dir: bool) -> bool {
    let ev = event(0, Kind::Modified, is_dir, Flags::NONE);
    run(
        &[Rule::exclude(Pattern::Glob(pattern.into()))],
        &CompileOptions::default(),
        path,
        &ev,
    )
    .matched_rule
    .is_some()
}

#[rstest]
// anchoring
#[case("/a.txt", "a.txt", false, true)]
#[case("/a.txt", "d\\a.txt", false, false)]
#[case("a.txt", "d/a.txt", false, true)]
#[case("a.txt", "x/y/a.txt", false, true)]
#[case("d/a.txt", "d\\a.txt", false, true)]
#[case("d/a.txt", "x/d/a.txt", false, false)]
#[case("a.txt", "xa.txt", false, false)]
// `*` and `?` stop at separators
#[case("*.tmp", "a.tmp", false, true)]
#[case("*.tmp", "dir/a.tmp", false, true)]
#[case("*.tmp", "a.tmpx", false, false)]
#[case("d/*.tmp", "d/a.tmp", false, true)]
#[case("d/*.tmp", "d/sub/a.tmp", false, false)]
#[case("d/*", "d/x", false, true)]
#[case("a?c", "abc", false, true)]
#[case("a?c", "a/c", false, false)]
#[case("a?c", "ac", false, false)]
// `**`
#[case("**/x", "x", false, true)]
#[case("**/x", "a/b/x", false, true)]
#[case("**/x", "ax", false, false)]
#[case("a/**", "a", true, false)]
#[case("a/**", "a/b", false, true)]
#[case("a/**", "a/b/c", false, true)]
#[case("a/**/b", "a/b", false, true)]
#[case("a/**/b", "a/x/y/b", false, true)]
#[case("a/**/b", "a/xb", false, false)]
#[case("**", "anything/at/all", false, true)]
#[case("a**b", "axxb", false, true)]
#[case("a**b", "ax/xb", false, false)]
// directory only
#[case("build/", "build", true, true)]
#[case("build/", "build", false, false)]
#[case("build/", "x/build/y.o", false, true)]
#[case("build/", "build.o", false, false)]
#[case("build/", "src/build", true, true)]
#[case("/build/", "src/build", true, false)]
#[case("build\\", "build", true, true)]
// everything below a match
#[case("a", "a/b/c", false, true)]
#[case("a/b", "a/b/c", false, true)]
#[case("a/b", "a", true, false)]
// classes
#[case("[abc].txt", "b.txt", false, true)]
#[case("[abc].txt", "d.txt", false, false)]
#[case("[!abc].txt", "d.txt", false, true)]
#[case("[!abc].txt", "a.txt", false, false)]
#[case("[^abc].txt", "d.txt", false, true)]
#[case("[a-c].txt", "B.txt", false, true)]
#[case("[a-c].txt", "d.txt", false, false)]
#[case("[*]", "*", false, true)]
#[case("[*]", "a", false, false)]
#[case("[]]", "]", false, true)]
#[case("[a-]", "-", false, true)]
#[case("a[!x]b", "a/b", false, false)]
// separators
#[case("a\\b", "a/b", false, true)]
#[case("a/b", "a\\b", false, true)]
// case: the default is case-insensitive
#[case("README", "readme", false, true)]
#[case("*.TMP", "a.tmp", false, true)]
#[case("\u{E4}", "\u{C4}", false, true)]
fn glob_rules(#[case] pattern: &str, #[case] path: &str, #[case] is_dir: bool, #[case] hit: bool) {
    assert_eq!(glob_hits(pattern, path, is_dir), hit, "{pattern} on {path}");
}

#[test]
fn case_sensitive_roots_never_fold() {
    let sensitive = CompileOptions {
        default_case: CaseMode::Insensitive,
        overrides: vec![(RootIdx(1), CaseMode::Sensitive)],
    };
    let rules = [Rule::exclude(Pattern::Glob("README".into()))];
    let hit = |root: u16, path: &str| {
        let ev = event(root, Kind::Modified, false, Flags::NONE);
        run(&rules, &sensitive, path, &ev).matched_rule.is_some()
    };
    assert!(hit(0, "readme"));
    assert!(!hit(1, "readme"));
    assert!(hit(1, "README"));
    // Regexes and extensions follow the same rule.
    let rules = [
        Rule::exclude(Pattern::Regex("readme".into())),
        Rule::exclude(Pattern::Ext(vec!["TMP".into()])),
    ];
    let hit = |root: u16, path: &str| {
        let ev = event(root, Kind::Modified, false, Flags::NONE);
        run(&rules, &sensitive, path, &ev).matched_rule.is_some()
    };
    assert!(hit(0, "x/README"));
    assert!(!hit(1, "x/README"));
    assert!(hit(0, "a.tmp"));
    assert!(!hit(1, "a.tmp"));
    assert!(hit(1, "a.TMP"));
}

#[test]
fn last_matching_rule_wins() {
    let ev = event(0, Kind::Modified, false, Flags::NONE);
    let keep = Rule::include(Pattern::Glob("keep.log".into()));
    let hide = Rule::exclude(Pattern::Glob("*.log".into()));
    let o = CompileOptions::default();
    let v = run(&[hide.clone(), keep.clone()], &o, "keep.log", &ev);
    assert_eq!(
        v,
        Verdict {
            record: true,
            visible: true,
            matched_rule: Some(1)
        }
    );
    let v = run(&[keep, hide], &o, "keep.log", &ev);
    assert_eq!(
        v,
        Verdict {
            record: true,
            visible: false,
            matched_rule: Some(1)
        }
    );
    // `!` in the pattern is the same as an include rule.
    let rules = [
        Rule::exclude(Pattern::Glob("*.log".into())),
        Rule::exclude(Pattern::Glob("!keep.log".into())),
    ];
    assert!(run(&rules, &o, "keep.log", &ev).visible);
    assert!(!run(&rules, &o, "other.log", &ev).visible);
    // No match: recorded and visible.
    let v = run(&rules, &o, "a.txt", &ev);
    assert_eq!(
        v,
        Verdict {
            record: true,
            visible: true,
            matched_rule: None
        }
    );
}

#[test]
fn rules_apply_by_scope() {
    let rules = [
        Rule::exclude(Pattern::Glob("*.tmp".into())).in_watch(RootIdx(1)),
        Rule::exclude(Pattern::Glob("*.log".into())),
    ];
    let o = CompileOptions::default();
    let hidden = |root: u16, path: &str| {
        let ev = event(root, Kind::Modified, false, Flags::NONE);
        !run(&rules, &o, path, &ev).visible
    };
    assert!(hidden(1, "a.tmp"));
    assert!(!hidden(0, "a.tmp"));
    assert!(!hidden(2, "a.tmp"));
    assert!(hidden(0, "a.log"));
    assert!(hidden(2, "a.log"));
}

#[test]
fn kinds_flags_and_extensions() {
    let o = CompileOptions::default();
    let rules = [Rule::exclude(Pattern::Kinds(KindSet::of(&[
        Kind::Overflow(crate::event::GapReason::ParserError),
        Kind::AttributesChanged,
    ])))];
    let on = |kind| run(&rules, &o, "a", &event(0, kind, false, Flags::NONE)).visible;
    assert!(!on(Kind::AttributesChanged));
    // The payload does not matter: the class does.
    assert!(!on(Kind::Overflow(crate::event::GapReason::BufferOverflow)));
    assert!(on(Kind::Modified));

    let rules = [Rule::exclude(Pattern::Flags(
        Flags::CLOUD_CHURN.union(Flags::TEMP),
    ))];
    let on = |flags| run(&rules, &o, "a", &event(0, Kind::Modified, false, flags)).visible;
    assert!(!on(Flags::TEMP));
    assert!(!on(Flags::CLOUD_CHURN.union(Flags::SELF_CHURN)));
    assert!(on(Flags::SELF_CHURN));
    assert!(on(Flags::NONE));

    let rules = [Rule::exclude(Pattern::Ext(vec![
        "tmp".into(),
        ".bak".into(),
    ]))];
    let on = |path: &str| {
        run(
            &rules,
            &o,
            path,
            &event(0, Kind::Modified, false, Flags::NONE),
        )
        .visible
    };
    assert!(!on("a.tmp"));
    assert!(!on("d\\a.BAK"));
    assert!(on("a.tmpx"));
    assert!(on(".tmp"), "a leading dot is a name, not an extension");
    assert!(on("a."));
    assert!(on("d.tmp/a"));
    assert!(on("tmp"));
}

#[test]
fn the_resolved_name_is_tested_too() {
    let rules = [Rule::exclude(Pattern::Glob("Long Name.txt".into()))];
    let set = FilterSet::compile(&rules).unwrap();
    let ev = event(0, Kind::Modified, false, Flags::NONE);
    let short = w("LONGNA~1.TXT");
    let long = w("dir\\Long Name.txt");
    let mut scratch = Scratch::new();
    assert!(set.eval(&ev, WideStr::new(&short), &mut scratch).visible);
    let v = set.eval_with_resolved(
        &ev,
        WideStr::new(&short),
        Some(WideStr::new(&long)),
        &mut scratch,
    );
    assert!(!v.visible);
}

#[test]
fn lone_surrogates_match_through_the_replacement_character() {
    let rules = [Rule::exclude(Pattern::Glob("a?b".into()))];
    let set = FilterSet::compile(&rules).unwrap();
    let ev = event(0, Kind::Modified, false, Flags::NONE);
    let name = [0x61, 0xD800, 0x62];
    assert!(
        !set.eval(&ev, WideStr::new(&name), &mut Scratch::new())
            .visible
    );
}

#[test]
fn over_long_names_are_not_tested() {
    let rules = [Rule::exclude(Pattern::Glob("**".into()))];
    let set = FilterSet::compile(&rules).unwrap();
    let ev = event(0, Kind::Modified, false, Flags::NONE);
    let huge = vec![0x61_u16; MAX_NAME_UNITS + 1];
    assert!(
        set.eval(&ev, WideStr::new(&huge), &mut Scratch::new())
            .visible
    );
    let max = vec![0x61_u16; MAX_NAME_UNITS];
    assert!(
        !set.eval(&ev, WideStr::new(&max), &mut Scratch::new())
            .visible
    );
}

#[test]
fn compile_errors_name_the_rule() {
    let bad = |pattern: Pattern| {
        FilterSet::compile(&[
            Rule::exclude(Pattern::Glob("ok".into())),
            Rule::exclude(pattern),
        ])
        .unwrap_err()
    };
    assert!(matches!(
        bad(Pattern::Glob("[".into())),
        FilterError::Glob {
            rule: 1,
            error: GlobError::UnclosedClass
        }
    ));
    assert!(matches!(
        bad(Pattern::Regex("(".into())),
        FilterError::Regex { rule: 1, .. }
    ));
    assert!(matches!(
        bad(Pattern::Ext(vec![])),
        FilterError::Ext { rule: 1 }
    ));
    assert!(matches!(
        bad(Pattern::Ext(vec!["a.b".into()])),
        FilterError::Ext { rule: 1 }
    ));
    assert!(matches!(
        bad(Pattern::Ext(vec![".".into()])),
        FilterError::Ext { rule: 1 }
    ));
    assert!(matches!(
        bad(Pattern::Kinds(KindSet::EMPTY)),
        FilterError::NoKinds { rule: 1 }
    ));
    assert!(matches!(
        bad(Pattern::Flags(Flags::NONE)),
        FilterError::NoFlags { rule: 1 }
    ));
    let e =
        FilterSet::compile(&[Rule::include(Pattern::Glob("a".into())).never_record()]).unwrap_err();
    assert_eq!(e, FilterError::IncludeNeverRecord { rule: 0 });
    let e = FilterSet::compile(&[Rule::include(Pattern::Glob("!a".into()))]).unwrap_err();
    assert_eq!(e, FilterError::NegatedInclude { rule: 0 });
    let e = FilterSet::compile(&[Rule::exclude(Pattern::Glob("!a".into())).never_record()])
        .unwrap_err();
    assert_eq!(e, FilterError::IncludeNeverRecord { rule: 0 });
    let shown = |e: FilterError| e.to_string();
    assert_eq!(shown(FilterError::TooManyRules), "more than 100000 rules");
    assert_eq!(
        shown(FilterError::Ext { rule: 4 }),
        "rule 4: invalid extension list"
    );
    assert_eq!(
        shown(FilterError::NoKinds { rule: 4 }),
        "rule 4: no event kinds"
    );
    assert_eq!(shown(FilterError::NoFlags { rule: 4 }), "rule 4: no flags");
    assert_eq!(
        shown(FilterError::IncludeNeverRecord { rule: 4 }),
        "rule 4: an include rule cannot drop events"
    );
    assert_eq!(
        shown(FilterError::NegatedInclude { rule: 4 }),
        "rule 4: `!` in an include rule"
    );
    assert_eq!(
        shown(FilterError::Glob {
            rule: 4,
            error: GlobError::Empty
        }),
        "rule 4: the pattern is empty"
    );
    assert_eq!(
        shown(FilterError::Regex {
            rule: 4,
            message: "bad".into()
        }),
        "rule 4: bad"
    );
    assert_eq!(
        shown(FilterError::TooComplex {
            scope: Scope::Global
        }),
        "the rules of Global are too complex to compile"
    );
}

#[test]
fn the_rule_limit_is_exact() {
    let rule = Rule::exclude(Pattern::Ext(vec!["a".into()]));
    let at_limit = vec![rule.clone(); MAX_RULES];
    assert_eq!(
        FilterSet::compile(&at_limit).unwrap().rule_count(),
        MAX_RULES
    );
    let over = vec![rule; MAX_RULES + 1];
    assert_eq!(
        FilterSet::compile(&over).unwrap_err(),
        FilterError::TooManyRules
    );
}

#[test]
fn size_limits_reject_big_regexes() {
    // The NFA of a bounded repetition of a Unicode class is over the limit.
    let huge = Rule::exclude(Pattern::Regex(r"\w{200}".into()));
    let e = FilterSet::compile(&[huge]).unwrap_err();
    assert!(
        matches!(&e, FilterError::Regex { rule: 0, message } if message.contains("exceeded limit")),
        "{e}"
    );
}

#[test]
fn kind_sets_ignore_duplicates() {
    let once = KindSet::of(&[Kind::Created]);
    assert_eq!(KindSet::of(&[Kind::Created, Kind::Created]), once);
    assert!(once.contains(Kind::Created));
    assert!(!once.contains(Kind::Deleted));
    assert!(KindSet::EMPTY.is_empty());
    assert!(!once.is_empty());
}

#[test]
fn memory_is_the_sum_of_the_dfas() {
    let glob = Rule::exclude(Pattern::Glob("*.tmp".into()));
    let regex = Rule::exclude(Pattern::Regex("a.b".into()));
    let mem = |rules: &[Rule], options: &CompileOptions| {
        FilterSet::compile_with(rules, options)
            .unwrap()
            .memory_usage()
    };
    let o = CompileOptions::default();
    let (g, r) = (
        mem(std::slice::from_ref(&glob), &o),
        mem(std::slice::from_ref(&regex), &o),
    );
    assert!(g > 0 && r > 0);
    // A glob DFA and a regex DFA are separate.
    assert_eq!(mem(&[glob.clone(), regex.clone()], &o), g + r);
    // A second scope has its own DFAs.
    let watch = glob.clone().in_watch(RootIdx(1));
    assert_eq!(mem(&[glob.clone(), watch], &o), g * 2);
    // Each case mode in use has its own set, and only modes in use count.
    let sensitive_only = CompileOptions {
        default_case: CaseMode::Sensitive,
        overrides: Vec::new(),
    };
    let both = CompileOptions {
        default_case: CaseMode::Insensitive,
        overrides: vec![(RootIdx(1), CaseMode::Sensitive)],
    };
    let s = mem(std::slice::from_ref(&glob), &sensitive_only);
    // Only the modes in use are built: a default of one mode costs one set.
    assert!(s > 0);
    assert!(mem(std::slice::from_ref(&glob), &both) > g);
    assert_eq!(mem(&[glob], &both), g + s);
    assert_eq!(FilterSet::compile(&[]).unwrap().memory_usage(), 0);
}

#[test]
fn classes_reach_outside_the_basic_plane() {
    let hit = |pattern: &str, path: &str| glob_hits(pattern, path, false);
    assert!(hit("[\u{10000}-\u{10FFFF}]", "\u{1F600}"));
    assert!(!hit("[\u{10000}-\u{10FFFF}]", "a"));
    assert!(hit("[\u{FFFE}-\u{FFFF}]", "\u{FFFF}"));
    assert!(!hit("[\u{FFFE}-\u{FFFF}]", "\u{1F600}"));
    assert!(hit("[\u{FFFF}-\u{10001}]", "\u{10000}"));
    assert!(hit("[\u{1F600}]", "\u{1F600}"));
    assert!(!hit("[\u{1F600}]", "\u{1F601}"));
    assert!(hit("[a-a]", "a"));
    assert!(!hit("[a-a]", "b"));
    assert!(hit("[a-cb-d]", "d"));
    assert!(!hit("[a-cb-d]", "e"));
}

#[test]
fn extensions_are_taken_from_the_last_component() {
    let rules = [Rule::exclude(Pattern::Ext(vec!["tmp".into()]))];
    let o = CompileOptions::default();
    let hidden = |path: &str| {
        let ev = event(0, Kind::Modified, false, Flags::NONE);
        !run(&rules, &o, path, &ev).visible
    };
    assert!(hidden("a.tmp"));
    assert!(hidden("dir.x\\a.tmp"));
    assert!(hidden("dir/a.b.tmp"));
    assert!(!hidden("a.tmp\\b"));
    assert!(!hidden("a.tmp/"));
    assert!(!hidden("dir.tmp\\a"));
    // A leading dot is a name even when a directory comes first.
    assert!(!hidden("d\\.tmp"));
    assert!(!hidden("d/.tmp"));
}

#[test]
fn a_case_insensitive_root_in_a_case_sensitive_default() {
    let options = CompileOptions {
        default_case: CaseMode::Sensitive,
        overrides: vec![(RootIdx(1), CaseMode::Insensitive)],
    };
    let rules = [Rule::exclude(Pattern::Glob("README".into()))];
    let hit = |root: u16, path: &str| {
        let ev = event(root, Kind::Modified, false, Flags::NONE);
        run(&rules, &options, path, &ev).matched_rule.is_some()
    };
    assert!(!hit(0, "readme"));
    assert!(hit(0, "README"));
    assert!(hit(1, "readme"));
    assert!(hit(1, "README"));
}

#[test]
fn never_record_drops_are_counted_per_rule() {
    let rules = [
        Rule::exclude(Pattern::Glob("*.tmp".into())).never_record(),
        Rule::exclude(Pattern::Glob("*.log".into())),
        Rule::exclude(Pattern::Ext(vec!["swp".into()])).never_record(),
    ];
    let set = FilterSet::compile(&rules).unwrap();
    let mut scratch = Scratch::new();
    let ev = event(0, Kind::Modified, false, Flags::NONE);
    let names = [
        "a.tmp", "b.log", "c.txt", "d.swp", "e.tmp", "f.log", "g.TMP", "h",
    ];
    let (mut recorded, mut dropped) = (0_u64, 0_u64);
    for name in names {
        let units = w(name);
        let v = set.eval(&ev, WideStr::new(&units), &mut scratch);
        if v.record {
            recorded += 1;
        } else {
            dropped += 1;
        }
    }
    let stats = set.stats();
    assert_eq!(stats.seen, 8);
    assert_eq!(stats.recorded, recorded);
    assert_eq!(stats.dropped, dropped);
    assert_eq!(stats.recorded + stats.dropped, stats.seen);
    assert_eq!(
        (
            set.dropped_by_rule(0),
            set.dropped_by_rule(1),
            set.dropped_by_rule(2)
        ),
        (3, 0, 1)
    );
    assert_eq!(set.dropped_by_rule(99), 0);
    assert_eq!(set.rule_count(), 3);
}

// ---- a naive reference matcher ------------------------------------------

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

fn fold(c: char, ci: bool) -> char {
    if !ci {
        return c;
    }
    let mut buf = [0_u16; 2];
    match c.encode_utf16(&mut buf) {
        [u] => char::from_u32(u32::from(fold_upper(*u))).unwrap_or(c),
        _ => c,
    }
}

/// Matches one glob segment against one component, char by char.
fn ref_segment(pat: &[char], text: &[char], ci: bool) -> bool {
    match pat.split_first() {
        None => text.is_empty(),
        Some(('*', rest)) => {
            (0..=text.len()).any(|k| ref_segment(rest, text.get(k..).unwrap_or(&[]), ci))
        }
        Some(('?', rest)) => text
            .split_first()
            .is_some_and(|(_, t)| ref_segment(rest, t, ci)),
        Some(('[', rest)) => {
            let Some((negated, members, after)) = ref_class(rest) else {
                return false;
            };
            let Some((&c, t)) = text.split_first() else {
                return false;
            };
            let folded = fold(c, ci);
            let inside = members.iter().any(|&(lo, hi)| {
                (lo..=hi)
                    .filter_map(char::from_u32)
                    .any(|x| fold(x, ci) == folded)
            });
            (inside != negated) && !is_sep(c) && ref_segment(after, t, ci)
        }
        Some((&p, rest)) => text
            .split_first()
            .is_some_and(|(&c, t)| fold(p, ci) == fold(c, ci) && ref_segment(rest, t, ci)),
    }
}

type Members = Vec<(u32, u32)>;

/// Parses the class after `[`: (negated, members, rest after `]`).
fn ref_class(chars: &[char]) -> Option<(bool, Members, &[char])> {
    let mut i = 0;
    let negated = matches!(chars.first(), Some('!' | '^'));
    if negated {
        i = 1;
    }
    let mut members = Vec::new();
    let mut count = 0;
    loop {
        let &c = chars.get(i)?;
        if c == ']' && count > 0 {
            return Some((negated, members, chars.get(i + 1..)?));
        }
        if chars.get(i + 1) == Some(&'-') && chars.get(i + 2).is_some_and(|&e| e != ']') {
            let end = *chars.get(i + 2)?;
            members.push((u32::from(c), u32::from(end)));
            i += 3;
        } else {
            members.push((u32::from(c), u32::from(c)));
            i += 1;
        }
        count += 1;
    }
}

fn ref_segments(segs: &[Vec<char>], comps: &[Vec<char>], ci: bool, whole: bool) -> bool {
    let Some((head, rest)) = segs.split_first() else {
        return comps.is_empty();
    };
    if head.as_slice() == ['*', '*'] {
        if segs.len() == 1 {
            return whole || !comps.is_empty();
        }
        return (0..=comps.len())
            .any(|i| ref_segments(rest, comps.get(i..).unwrap_or(&[]), ci, false));
    }
    comps
        .split_first()
        .is_some_and(|(c, cs)| ref_segment(head, c, ci) && ref_segments(rest, cs, ci, false))
}

fn ref_glob(pattern: &str, path: &str, is_dir: bool, ci: bool) -> bool {
    let mut chars: Vec<char> = pattern.chars().collect();
    if chars.first() == Some(&'!') {
        chars.remove(0);
    }
    let dir_only = chars.last().copied().is_some_and(is_sep);
    if dir_only {
        chars.pop();
    }
    let leading = chars.first().copied().is_some_and(is_sep);
    if leading {
        chars.remove(0);
    }
    let anchored = leading || chars.iter().copied().any(is_sep);
    let mut segs: Vec<Vec<char>> = chars.split(|&c| is_sep(c)).map(<[char]>::to_vec).collect();
    segs.dedup_by(|a, b| a.as_slice() == ['*', '*'] && b.as_slice() == ['*', '*']);
    let comps: Vec<Vec<char>> = path
        .split(['/', '\\'])
        .map(|c| c.chars().collect())
        .collect();
    let n = comps.len();
    let self_ok = |m: usize| m < n || !dir_only || is_dir;
    if !anchored {
        let Some(seg) = segs.first() else {
            return false;
        };
        if seg.as_slice() == ['*', '*'] {
            // Every component matches; a directory-only `**/` needs a
            // directory: an ancestor, or the path itself when it is one.
            return (0..n).any(|j| self_ok(j + 1));
        }
        return (0..n)
            .any(|j| comps.get(j).is_some_and(|c| ref_segment(seg, c, ci)) && self_ok(j + 1));
    }
    (1..=n).any(|m| ref_segments(&segs, comps.get(..m).unwrap_or(&[]), ci, true) && self_ok(m))
}

#[derive(Debug, Clone)]
enum RefPattern {
    Glob(String),
    Regex(&'static str),
    Ext(&'static str),
    Kinds(Vec<Kind>),
    Flags(Flags),
}

#[derive(Debug, Clone)]
struct RefRule {
    root: Option<u16>,
    pattern: RefPattern,
    include: bool,
    never_record: bool,
}

fn ref_regex(source: &str, path: &str, ci: bool) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (
        source.chars().map(|c| fold(c, ci)).collect(),
        path.chars().map(|c| fold(c, ci)).collect(),
    );
    match source {
        "ab" => t.windows(2).any(|w| w == p.as_slice()),
        "^a" => t.first() == p.get(1),
        "b$" => t.last() == p.first(),
        // `a.b`: a, any char, b.
        _ => t
            .windows(3)
            .any(|w| w.first() == p.first() && w.get(2) == p.get(2)),
    }
}

fn ref_ext(ext: &str, path: &str, ci: bool) -> bool {
    let leaf = path.rsplit(['/', '\\']).next().unwrap_or("");
    match leaf.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < leaf.len() => {
            let have: Vec<char> = leaf[dot + 1..].chars().map(|c| fold(c, ci)).collect();
            let want: Vec<char> = ext.chars().map(|c| fold(c, ci)).collect();
            have == want
        }
        _ => false,
    }
}

fn ref_verdict(rules: &[RefRule], root: u16, path: &str, ev: &Event, ci: bool) -> Verdict {
    let mut last = None;
    for (i, rule) in rules.iter().enumerate() {
        if rule.root.is_some_and(|r| r != root) {
            continue;
        }
        let hit = match &rule.pattern {
            RefPattern::Glob(g) => ref_glob(g, path, ev.is_dir, ci),
            RefPattern::Regex(r) => ref_regex(r, path, ci),
            RefPattern::Ext(e) => ref_ext(e, path, ci),
            RefPattern::Kinds(kinds) => kinds.iter().any(|k| k.to_wire().0 == ev.kind.to_wire().0),
            RefPattern::Flags(f) => ev.flags.bits() & f.bits() != 0,
        };
        if hit {
            last = Some(i);
        }
    }
    match last.and_then(|i| rules.get(i).map(|r| (i, r))) {
        None => Verdict {
            record: true,
            visible: true,
            matched_rule: None,
        },
        Some((i, r)) => {
            let negated = matches!(&r.pattern, RefPattern::Glob(g) if g.starts_with('!'));
            if r.include || negated {
                Verdict {
                    record: true,
                    visible: true,
                    matched_rule: Some(i),
                }
            } else {
                Verdict {
                    record: !r.never_record,
                    visible: false,
                    matched_rule: Some(i),
                }
            }
        }
    }
}

fn to_rule(r: &RefRule) -> Rule {
    let pattern = match &r.pattern {
        RefPattern::Glob(g) => Pattern::Glob(g.clone()),
        RefPattern::Regex(s) => Pattern::Regex((*s).into()),
        RefPattern::Ext(e) => Pattern::Ext(vec![(*e).into()]),
        RefPattern::Kinds(k) => Pattern::Kinds(KindSet::of(k)),
        RefPattern::Flags(f) => Pattern::Flags(*f),
    };
    let mut rule = if r.include {
        Rule::include(pattern)
    } else {
        Rule::exclude(pattern)
    };
    if let Some(root) = r.root {
        rule = rule.in_watch(RootIdx(root));
    }
    if r.never_record {
        rule = rule.never_record();
    }
    rule
}

fn arb_segment() -> impl Strategy<Value = String> {
    let atom = prop_oneof![
        Just("a".to_owned()),
        Just("A".to_owned()),
        Just("b".to_owned()),
        Just(".".to_owned()),
        Just("_".to_owned()),
        Just("*".to_owned()),
        Just("?".to_owned()),
        Just("[ab]".to_owned()),
        Just("[!a]".to_owned()),
        Just("[a-b]".to_owned()),
        Just("[*.]".to_owned()),
    ];
    prop_oneof![
        8 => proptest::collection::vec(atom, 1..4).prop_map(|v| v.concat()),
        2 => Just("**".to_owned()),
    ]
}

fn arb_glob() -> impl Strategy<Value = String> {
    (
        any::<bool>(),
        any::<bool>(),
        proptest::collection::vec(arb_segment(), 1..4),
        prop_oneof![Just("/"), Just("\\")],
        any::<bool>(),
    )
        .prop_map(|(lead, dir, segs, sep, bang)| {
            let mut s = String::new();
            if bang {
                s.push('!');
            }
            if lead {
                s.push('/');
            }
            s.push_str(&segs.join(sep));
            if dir {
                s.push('/');
            }
            s
        })
}

fn arb_rule() -> impl Strategy<Value = RefRule> {
    let pattern = prop_oneof![
        6 => arb_glob().prop_map(RefPattern::Glob),
        1 => prop_oneof![Just("ab"), Just("^a"), Just("b$"), Just("a.b")].prop_map(RefPattern::Regex),
        1 => prop_oneof![Just("a"), Just("ab"), Just("B")].prop_map(RefPattern::Ext),
        1 => proptest::sample::subsequence(
            vec![Kind::Created, Kind::Modified, Kind::Deleted, Kind::RenamedTo],
            1..3,
        ).prop_map(RefPattern::Kinds),
        1 => prop_oneof![Just(Flags::TEMP), Just(Flags::CLOUD_CHURN)].prop_map(RefPattern::Flags),
    ];
    (
        proptest::option::of(0_u16..2),
        pattern,
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(root, pattern, include, never)| {
            let negated = matches!(&pattern, RefPattern::Glob(g) if g.starts_with('!'));
            // Valid combinations only: `!` needs an exclude rule, an include
            // rule never drops.
            let include = include && !negated;
            RefRule {
                root,
                pattern,
                include,
                never_record: never && !include && !negated,
            }
        })
}

fn arb_path() -> impl Strategy<Value = String> {
    let comp = proptest::collection::vec(
        prop_oneof![Just('a'), Just('A'), Just('b'), Just('.'), Just('_')],
        1..4,
    )
    .prop_map(|v| v.into_iter().collect::<String>());
    (
        proptest::collection::vec(comp, 1..4),
        proptest::collection::vec(prop_oneof![Just('/'), Just('\\')], 3),
    )
        .prop_map(|(comps, seps)| {
            let mut s = String::new();
            for (i, c) in comps.iter().enumerate() {
                if i > 0 {
                    s.push(seps.get(i - 1).copied().unwrap_or('/'));
                }
                s.push_str(c);
            }
            s
        })
}

proptest! {
    // A failing case is found fast; shrinking it a long time only slows a
    // mutation run down.
    #![proptest_config(ProptestConfig { cases: 2000, max_shrink_iters: 100, ..ProptestConfig::default() })]

    #[test]
    fn compiled_verdict_equals_the_reference(
        rules in proptest::collection::vec(arb_rule(), 0..6),
        path in arb_path(),
        root in 0_u16..2,
        is_dir in any::<bool>(),
        kind in prop_oneof![Just(Kind::Created), Just(Kind::Modified), Just(Kind::Deleted)],
        flags in prop_oneof![Just(Flags::NONE), Just(Flags::TEMP), Just(Flags::CLOUD_CHURN)],
        sensitive in any::<bool>(),
    ) {
        let ev = event(root, kind, is_dir, flags);
        let mode = if sensitive { CaseMode::Sensitive } else { CaseMode::Insensitive };
        let options = CompileOptions { default_case: mode, overrides: Vec::new() };
        let compiled: Vec<Rule> = rules.iter().map(to_rule).collect();
        let got = run(&compiled, &options, &path, &ev);
        let want = ref_verdict(&rules, root, &path, &ev, !sensitive);
        prop_assert_eq!(got, want, "rules {:?} path {}", rules, path);
    }
}

/// Literal names over letters whose fold is not ASCII-only.
fn arb_literal() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop_oneof![
            Just('a'),
            Just('A'),
            Just('\u{E4}'),
            Just('\u{C4}'),
            Just('\u{3C3}'),
            Just('\u{3C2}'),
            Just('\u{3A3}'),
            Just('\u{DF}'),
            Just('\u{131}'),
            Just('\u{69}'),
            Just('\u{49}'),
            Just('\u{1F600}'),
            Just('\u{10428}'),
            Just('\u{10400}'),
        ],
        1..5,
    )
    .prop_map(|v| v.into_iter().collect())
}

fn fold_text(text: &str) -> String {
    text.chars().map(|c| fold(c, true)).collect()
}

proptest! {
    #[test]
    fn insensitive_equals_folding_both_sides(pattern in arb_literal(), name in arb_literal()) {
        let rules = [Rule::exclude(Pattern::Glob(format!("/{}", glob_escape(&pattern))))];
        let ev = event(0, Kind::Modified, false, Flags::NONE);
        let insensitive = run(&rules, &CompileOptions::default(), &name, &ev).matched_rule.is_some();
        let folded_rules = [Rule::exclude(Pattern::Glob(format!("/{}", glob_escape(&fold_text(&pattern)))))];
        let sensitive = CompileOptions { default_case: CaseMode::Sensitive, overrides: Vec::new() };
        let folded = run(&folded_rules, &sensitive, &fold_text(&name), &ev).matched_rule.is_some();
        prop_assert_eq!(insensitive, folded);
        prop_assert_eq!(insensitive, fold_text(&pattern) == fold_text(&name));
    }

    #[test]
    fn sensitive_roots_never_fold(pattern in arb_literal(), name in arb_literal()) {
        let rules = [Rule::exclude(Pattern::Glob(format!("/{}", glob_escape(&pattern))))];
        let ev = event(0, Kind::Modified, false, Flags::NONE);
        let sensitive = CompileOptions { default_case: CaseMode::Sensitive, overrides: Vec::new() };
        let hit = run(&rules, &sensitive, &name, &ev).matched_rule.is_some();
        prop_assert_eq!(hit, pattern == name);
    }
}

/// Wraps glob metacharacters of a literal in `[x]` classes.
fn glob_escape(literal: &str) -> String {
    literal
        .chars()
        .map(|c| {
            if matches!(c, '*' | '?' | '[' | '!') {
                format!("[{c}]")
            } else {
                c.to_string()
            }
        })
        .collect()
}

proptest! {
    #[test]
    fn compile_never_panics_on_arbitrary_patterns(
        glob in "\\PC{0,24}",
        regex in "\\PC{0,24}",
        ext in "\\PC{0,8}",
    ) {
        let rules = [
            Rule::exclude(Pattern::Glob(glob)),
            Rule::exclude(Pattern::Regex(regex)),
            Rule::exclude(Pattern::Ext(vec![ext])),
        ];
        let _ = FilterSet::compile(&rules);
    }

    #[test]
    fn eval_never_panics_on_arbitrary_names(units in proptest::collection::vec(any::<u16>(), 0..40)) {
        let rules = [
            Rule::exclude(Pattern::Glob("**/*.tmp".into())),
            Rule::exclude(Pattern::Regex("a.b".into())),
            Rule::exclude(Pattern::Ext(vec!["x".into()])),
        ];
        let set = FilterSet::compile(&rules).unwrap();
        let ev = event(0, Kind::Modified, false, Flags::NONE);
        let _ = set.eval(&ev, WideStr::new(&units), &mut Scratch::new());
    }
}
