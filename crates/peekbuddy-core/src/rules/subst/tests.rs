#![expect(
    clippy::indexing_slicing,
    reason = "the tests index the few arguments they build"
)]

use proptest::prelude::*;

use super::*;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// The arguments `CommandLineToArgvW` (and the C runtime) read from a command
/// line made of arguments only, after the program name: the documented
/// rules for quotes and backslashes.
fn parse(line: &[u16]) -> Vec<Vec<u16>> {
    let mut args = Vec::new();
    let mut i = 0;
    while i < line.len() {
        while matches!(line.get(i), Some(&(SPACE | TAB))) {
            i += 1;
        }
        if i >= line.len() {
            break;
        }
        let mut arg = Vec::new();
        let mut in_quotes = false;
        while let Some(&unit) = line.get(i) {
            if !in_quotes && matches!(unit, SPACE | TAB) {
                break;
            }
            if unit == BACKSLASH {
                let mut n = 0;
                while line.get(i) == Some(&BACKSLASH) {
                    n += 1;
                    i += 1;
                }
                if line.get(i) == Some(&QUOTE) {
                    arg.extend(std::iter::repeat_n(BACKSLASH, n / 2));
                    if n % 2 == 1 {
                        arg.push(QUOTE);
                        i += 1;
                    }
                } else {
                    arg.extend(std::iter::repeat_n(BACKSLASH, n));
                }
                continue;
            }
            if unit == QUOTE {
                in_quotes = !in_quotes;
                i += 1;
                continue;
            }
            arg.push(unit);
            i += 1;
        }
        args.push(arg);
    }
    args
}

fn values<'a>(path: &'a [u16], root: &'a [u16]) -> Values<'a> {
    Values {
        path,
        kind: "created",
        root,
        ts: 133_000_000_000_000_000,
    }
}

#[test]
fn a_placeholder_is_one_whole_argument() {
    let path = units(r"C:\Watched\a b.txt");
    let root = units(r"C:\Watched");
    let args = expand(
        &argv(&[
            "C:\\tools\\h.exe",
            "%path%",
            "--kind",
            "%kind%",
            "%root%",
            "%ts%",
        ]),
        &values(&path, &root),
    );
    assert_eq!(
        args,
        [
            units(r"C:\tools\h.exe"),
            path,
            units("--kind"),
            units("created"),
            root,
            units("133000000000000000"),
        ]
    );
}

#[test]
fn a_placeholder_inside_other_text_is_left_alone() {
    let args = expand(
        &argv(&["x", "--file=%path%", "%PATH%", "%path% ", "%%path%%"]),
        &values(&units("p"), &units("r")),
    );
    assert_eq!(
        args[1..],
        [
            units("--file=%path%"),
            units("%PATH%"),
            units("%path% "),
            units("%%path%%")
        ]
    );
}

#[test]
fn a_nul_cannot_cut_the_command_line_short() {
    let path = vec![u16::from(b'a'), 0, u16::from(b'b')];
    let args = expand(&argv(&["x", "%path%", "a\0b"]), &values(&path, &[]));
    assert_eq!(args[1], vec![u16::from(b'a'), 0xFFFD, u16::from(b'b')]);
    assert_eq!(args[2], units("ab"));
}

#[test]
fn a_lone_surrogate_survives() {
    let path = vec![u16::from(b'a'), 0xD800, u16::from(b'b')];
    let args = expand(&argv(&["x", "%path%"]), &values(&path, &[]));
    assert_eq!(args[1], path);
    let line = command_line(&args);
    assert_eq!(parse(&line[2..]), vec![path]);
}

#[test]
fn hostile_names_stay_one_argument_each() {
    for name in [
        r"C:\a & del *.*",
        r#"C:\say "hi".txt"#,
        r"C:\100%.txt",
        "C:\\trail\\",
        "C:\\two  spaces\\",
        r#"C:\q\"x"#,
        "C:\\tab\there",
        "",
        "-rf",
        r"C:\a|b>c<d^e.txt",
        "C:\\back\\\\\"",
    ] {
        let path = units(name);
        let args = expand(
            &argv(&["C:\\p.exe", "%path%", "after"]),
            &values(&path, &units("C:\\")),
        );
        let line = command_line(&args[1..]);
        assert_eq!(parse(&line), vec![path, units("after")], "{name:?}");
    }
}

#[test]
fn placeholders_are_recognised_exactly() {
    for p in PLACEHOLDERS {
        assert!(is_placeholder(p));
    }
    assert!(!is_placeholder("%path%x"));
    assert!(!is_placeholder("path"));
    assert!(!is_placeholder("%PATH%"));
}

proptest! {
    /// Any arguments, quoted and parsed back, are the same arguments.
    #[test]
    fn quoting_round_trips(args in proptest::collection::vec(
        proptest::collection::vec(
            prop_oneof![
                Just(u16::from(b'a')), Just(SPACE), Just(TAB), Just(QUOTE),
                Just(BACKSLASH), Just(u16::from(b'%')), Just(u16::from(b'&')),
                0xD800_u16..0xE000, 1_u16..0x80,
            ],
            0..12,
        ),
        1..6,
    )) {
        let line = command_line(&args);
        prop_assert_eq!(parse(&line), args);
    }
}
