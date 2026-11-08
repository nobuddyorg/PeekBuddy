use super::{Command, Rule, check, kind_set};

fn ok_rule() -> Rule {
    Rule {
        name: String::from("Config changed"),
        glob: String::from("*.toml"),
        kinds: vec![String::from("modified")],
        ..Rule::default()
    }
}

fn keys(problems: &[(String, String)]) -> Vec<&str> {
    problems.iter().map(|(key, _)| key.as_str()).collect()
}

#[test]
fn a_plain_rule_is_fine_and_the_defaults_are_the_documented_ones() {
    assert_eq!(check(&ok_rule(), &[]), []);
    let rule = Rule::default();
    assert_eq!(
        (
            rule.enabled,
            rule.threshold,
            rule.window_s,
            rule.cooldown_s,
            rule.toast
        ),
        (true, 1, 10, 60, true)
    );
}

#[test]
fn every_problem_is_reported_with_its_key() {
    let rule = Rule {
        name: String::from("  "),
        watches: vec![String::from("ghost")],
        glob: String::from("[unclosed"),
        kinds: vec![String::from("sneezed")],
        threshold: 0,
        window_s: 0,
        cooldown_s: 999_999,
        toast: false,
        command: None,
        ..Rule::default()
    };
    assert_eq!(
        keys(&check(&rule, &["w1"])),
        [
            "name",
            "watches[0]",
            "glob",
            "kinds",
            "threshold",
            "window_s",
            "cooldown_s",
            "toast"
        ]
    );
}

#[test]
fn limits_are_inclusive() {
    let mut rule = ok_rule();
    rule.threshold = super::MAX_THRESHOLD;
    rule.window_s = super::MAX_WINDOW_S;
    rule.cooldown_s = super::MAX_COOLDOWN_S;
    assert_eq!(check(&rule, &[]), []);
    rule.threshold += 1;
    assert_eq!(keys(&check(&rule, &[])), ["threshold"]);
}

fn with_command(argv: &[&str], confirmed: bool) -> Rule {
    Rule {
        command: Some(Command {
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            confirmed,
            ..Command::default()
        }),
        ..ok_rule()
    }
}

#[test]
fn a_command_needs_the_users_explicit_yes() {
    assert_eq!(
        keys(&check(
            &with_command(&[r"C:\tools\h.exe", "%path%"], false),
            &[]
        )),
        ["command.confirmed"]
    );
    assert_eq!(
        check(&with_command(&[r"C:\tools\h.exe", "%path%"], true), &[]),
        []
    );
}

#[test]
fn the_program_is_a_full_path_and_never_from_the_event() {
    for program in ["h.exe", "%path%", r".\h.exe", "cmd", "/bin/sh"] {
        let found = check(&with_command(&[program, "x"], true), &[]);
        assert_eq!(keys(&found), ["command.argv[0]"], "{program}");
    }
    for program in [r"C:\tools\h.exe", "d:/tools/h.exe", r"\\server\share\h.exe"] {
        assert_eq!(check(&with_command(&[program], true), &[]), [], "{program}");
    }
    assert_eq!(
        keys(&check(&with_command(&[], true), &[])),
        ["command.argv"]
    );
}

#[test]
fn argument_and_time_limits_are_checked() {
    let found = check(&with_command(&[r"C:\p.exe", "a\0b"], true), &[]);
    assert_eq!(keys(&found), ["command.argv[1]"]);
    let mut rule = with_command(&[r"C:\p.exe"], true);
    if let Some(command) = rule.command.as_mut() {
        command.timeout_s = 0;
    }
    assert_eq!(keys(&check(&rule, &[])), ["command.timeout_s"]);
}

#[test]
fn a_rule_without_a_toast_needs_a_command() {
    let mut rule = ok_rule();
    rule.toast = false;
    assert_eq!(keys(&check(&rule, &[])), ["toast"]);
    rule.command = Some(Command {
        argv: vec![String::from(r"C:\p.exe")],
        confirmed: true,
        ..Command::default()
    });
    assert_eq!(check(&rule, &[]), []);
}

#[test]
fn kind_names_are_the_export_names_of_file_changes_only() {
    let all = kind_set(&[]).unwrap();
    for name in [
        "created",
        "modified",
        "deleted",
        "renamed_to",
        "reconciled_created",
    ] {
        let one = kind_set(&[name.to_owned()]).unwrap();
        assert!(!one.is_empty(), "{name}");
        assert!(!all.is_empty());
    }
    for name in [
        "overflow:ring_full",
        "engine_started",
        "marker",
        "watch_state:live",
        "",
    ] {
        assert!(kind_set(&[name.to_owned()]).is_err(), "{name:?}");
    }
}
