use proptest::prelude::*;

use super::{MAX_WATCHES, Notice, Refusal, add_watch, check, new_id, remove_watch};
use crate::settings::{Context, Loaded, Settings, Watch, load};

fn ctx() -> Context {
    Context {
        data_dir: String::from(r"C:\Users\me\AppData\Local\PeekBuddy"),
        export_dir: Some(String::from(r"C:\Users\me\Documents\PeekBuddy exports")),
    }
}

fn settings_with(watches: &[(&str, &str, bool)]) -> Settings {
    Settings {
        watches: watches
            .iter()
            .map(|(id, path, recursive)| Watch {
                id: (*id).to_owned(),
                path: (*path).to_owned(),
                recursive: *recursive,
                ..Watch::default()
            })
            .collect(),
        ..Settings::default()
    }
}

#[test]
fn a_folder_is_added_normalised_with_a_stable_id() {
    let mut settings = Settings::default();
    let added = add_watch(&mut settings, "C:/Work/Reports/", false, &ctx()).unwrap();
    assert_eq!(added.watch.path, r"\\?\C:\Work\Reports");
    assert!(added.watch.recursive);
    assert!(added.watch.enabled);
    assert_eq!(added.notices, Vec::new());
    assert!(added.watch.id.starts_with("reports-"), "{}", added.watch.id);
    assert_eq!(settings.watches, vec![added.watch.clone()]);
    // The same folder gets the same id once it is gone.
    remove_watch(&mut settings, &added.watch.id).unwrap();
    let again = add_watch(&mut settings, r"c:\work\reports", false, &ctx()).unwrap();
    assert_eq!(again.watch.id, added.watch.id);
}

#[test]
fn what_is_added_survives_validation_and_a_round_trip() {
    let mut settings = Settings::default();
    for path in [
        r"C:\Work",
        r"D:\Ünïcode folder (2)",
        r"\\server\share\team",
        r"E:\",
    ] {
        add_watch(&mut settings, path, true, &ctx()).unwrap();
    }
    let text = crate::settings::to_toml(&settings).unwrap();
    let Loaded::Ok { settings: back, .. } = load(&text, &ctx()) else {
        panic!("the file written is not valid: {text}");
    };
    assert_eq!(back.watches.len(), 4);
    assert_eq!(
        back.watches
            .iter()
            .map(|watch| &watch.path)
            .collect::<Vec<_>>(),
        settings
            .watches
            .iter()
            .map(|watch| &watch.path)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_data_directory_and_everything_inside_it_is_refused() {
    let mut settings = Settings::default();
    for path in [
        r"C:\Users\me\AppData\Local\PeekBuddy",
        r"C:\Users\me\AppData\Local\PeekBuddy\segments",
        r"c:/users/ME/appdata/local/peekbuddy/db",
    ] {
        assert_eq!(
            add_watch(&mut settings, path, true, &ctx()),
            Err(Refusal::DataDir),
            "{path}"
        );
    }
    assert_eq!(settings.watches, Vec::new());
    // A sibling is fine.
    assert!(
        add_watch(
            &mut settings,
            r"C:\Users\me\AppData\Local\PeekBuddy2",
            false,
            &ctx()
        )
        .is_ok()
    );
}

#[test]
fn the_export_directory_and_everything_inside_it_is_refused() {
    let mut settings = Settings::default();
    for path in [
        r"C:\Users\me\Documents\PeekBuddy exports",
        r"C:\Users\me\Documents\PeekBuddy exports\2026",
    ] {
        assert_eq!(
            add_watch(&mut settings, path, true, &ctx()),
            Err(Refusal::ExportDir),
            "{path}"
        );
    }
    // Without an export directory configured nothing is refused for it.
    let none = Context {
        export_dir: None,
        ..ctx()
    };
    assert!(
        add_watch(
            &mut settings,
            r"C:\Users\me\Documents\PeekBuddy exports",
            false,
            &none
        )
        .is_ok()
    );
}

#[test]
fn refusals_say_what_is_wrong_in_words() {
    for (refusal, word) in [
        (Refusal::DataDir, "data folder"),
        (Refusal::ExportDir, "export folder"),
        (
            Refusal::Duplicate {
                id: String::from("x"),
            },
            "already watched",
        ),
        (Refusal::Unknown(String::from("x")), "no watch"),
        (Refusal::TooMany, "too many"),
        (Refusal::Invalid(String::from("the path is empty")), "empty"),
        (
            Refusal::NeedsConfirmation(Notice::WholeDrive),
            "whole drive",
        ),
    ] {
        assert!(refusal.to_string().contains(word), "{refusal:?}");
    }
}

#[test]
fn a_whole_drive_needs_an_explicit_confirmation() {
    let mut settings = Settings::default();
    for path in [r"C:\", "c:", "C:/", r"\\?\C:\"] {
        assert_eq!(
            add_watch(&mut settings, path, false, &ctx()),
            Err(Refusal::NeedsConfirmation(Notice::WholeDrive)),
            "{path}"
        );
    }
    assert_eq!(settings.watches, Vec::new(), "nothing changes unconfirmed");
    let added = add_watch(&mut settings, r"D:\", true, &ctx()).unwrap();
    assert_eq!(added.watch.path, r"\\?\D:\");
    assert_eq!(added.notices, vec![Notice::WholeDrive]);
    assert!(added.watch.id.starts_with("d-"), "{}", added.watch.id);
}

#[test]
fn a_network_folder_is_added_with_a_word_about_it() {
    let mut settings = Settings::default();
    let added = add_watch(&mut settings, r"\\nas\team\docs", false, &ctx()).unwrap();
    assert_eq!(added.watch.path, r"\\?\UNC\nas\team\docs");
    assert_eq!(added.notices, vec![Notice::Network]);
    assert!(added.notices[0].text().contains("64 KiB"));
}

#[test]
fn the_same_folder_twice_is_refused_whatever_its_spelling() {
    let mut settings = Settings::default();
    let first = add_watch(&mut settings, r"C:\Work", false, &ctx()).unwrap();
    for path in [r"C:\Work", r"c:\work\", "C:/WORK", r"\\?\C:\Work"] {
        assert_eq!(
            add_watch(&mut settings, path, true, &ctx()),
            Err(Refusal::Duplicate {
                id: first.watch.id.clone()
            }),
            "{path}"
        );
    }
    assert_eq!(settings.watches.len(), 1);
}

#[test]
fn a_covered_subtree_is_added_with_information_and_a_container_says_what_it_holds() {
    let mut settings = settings_with(&[
        ("outer", r"\\?\C:\Work", true),
        ("flat", r"\\?\C:\Flat", false),
        ("inner", r"\\?\C:\Other\Inner", true),
    ]);
    let covered = add_watch(&mut settings, r"C:\Work\sub", false, &ctx()).unwrap();
    assert_eq!(
        covered.notices,
        vec![Notice::CoveredBy {
            id: String::from("outer")
        }]
    );
    // A non-recursive watch does not cover its children.
    let not_covered = add_watch(&mut settings, r"C:\Flat\sub", false, &ctx()).unwrap();
    assert_eq!(not_covered.notices, Vec::new());
    let container = add_watch(&mut settings, r"C:\Other", false, &ctx()).unwrap();
    assert_eq!(
        container.notices,
        vec![Notice::Covers {
            ids: vec![String::from("inner")]
        }]
    );
    assert!(container.notices[0].text().contains("`inner`"));
}

#[test]
fn bad_paths_are_refused_with_the_reason_from_the_parser() {
    let mut settings = Settings::default();
    for (path, word) in [
        ("", "empty"),
        ("relative\\dir", "absolute"),
        (r"C:\a\..\b", ".."),
        (r"\\.\pipe\x", "device"),
        (r"\\server", "server and a share"),
    ] {
        let Err(Refusal::Invalid(message)) = add_watch(&mut settings, path, true, &ctx()) else {
            panic!("{path} was not refused as invalid");
        };
        assert!(message.contains(word), "{path}: {message}");
    }
    // Surrounding blanks, as a pasted path has, are ignored.
    assert!(add_watch(&mut settings, "  C:\\Pasted  \n", false, &ctx()).is_ok());
}

#[test]
fn checking_changes_nothing() {
    let settings = Settings::default();
    let (path, notices) = check(&settings, r"C:\Work", false, &ctx()).unwrap();
    assert_eq!(path, r"\\?\C:\Work");
    assert_eq!(notices, Vec::new());
    assert_eq!(settings.watches, Vec::new());
}

#[test]
fn removing_takes_out_exactly_one_watch_and_unknown_ids_fail() {
    let mut settings = settings_with(&[("a", r"\\?\C:\A", true), ("b", r"\\?\C:\B", true)]);
    let removed = remove_watch(&mut settings, "a").unwrap();
    assert_eq!(removed.watch.path, r"\\?\C:\A");
    assert_eq!(settings.watches.len(), 1);
    assert_eq!(
        remove_watch(&mut settings, "a"),
        Err(Refusal::Unknown(String::from("a")))
    );
    assert_eq!(settings.watches.len(), 1);
}

#[test]
fn a_watch_the_engine_derived_for_a_link_cannot_be_removed_by_id() {
    let mut settings = settings_with(&[("a", r"\\?\C:\A", true)]);
    settings.watches.push(Watch {
        id: String::from("a-link"),
        path: String::from(r"\\?\C:\Target"),
        derived_from: Some(String::from("a")),
        ..Watch::default()
    });
    assert!(remove_watch(&mut settings, "a-link").is_err());
    assert_eq!(settings.watches.len(), 2);
}

#[test]
fn an_id_clash_gets_a_counter_and_stays_valid() {
    let mut settings = Settings::default();
    let id = new_id(&settings, r"\\?\C:\Same");
    settings.watches.push(Watch {
        id: id.clone(),
        path: String::from(r"\\?\D:\Elsewhere"),
        ..Watch::default()
    });
    let second = new_id(&settings, r"\\?\C:\Same");
    assert_eq!(second, format!("{id}-2"));
}

#[test]
fn the_number_of_watches_is_capped() {
    let mut settings = Settings::default();
    for n in 0..MAX_WATCHES {
        add_watch(&mut settings, &format!(r"C:\w{n}"), false, &ctx()).unwrap();
    }
    assert_eq!(
        add_watch(&mut settings, r"C:\one-more", false, &ctx()),
        Err(Refusal::TooMany)
    );
}

proptest! {
    #[test]
    fn ids_are_always_valid_and_paths_never_panic(path in "\\PC{0,60}") {
        let mut settings = Settings::default();
        if let Ok(added) = add_watch(&mut settings, &path, true, &ctx()) {
            let id = &added.watch.id;
            prop_assert!((1..=64).contains(&id.len()), "{}", id);
            prop_assert!(
                id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'),
                "{}", id
            );
            // Whatever is accepted is accepted by the file validator too.
            let text = crate::settings::to_toml(&settings).unwrap();
            prop_assert!(matches!(load(&text, &ctx()), Loaded::Ok { .. }), "{}", text);
        }
    }
}

#[test]
fn a_path_in_the_resolved_spelling_is_checked_against_the_resolved_folders() {
    use super::inside_own_folders;

    let resolved = Context {
        data_dir: String::from(r"\\?\C:\Users\runneradmin\AppData\Local\Temp\pbt\data"),
        export_dir: Some(String::from(
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\pbt\data\exports",
        )),
    };
    // What the file system makes of `C:\Users\RUNNER~1\...\pbt\data\segments`.
    assert_eq!(
        inside_own_folders(
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\pbt\data\segments",
            &resolved
        ),
        Some(Refusal::DataDir)
    );
    assert_eq!(
        inside_own_folders(
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\pbt\data",
            &resolved
        ),
        Some(Refusal::DataDir)
    );
    assert_eq!(
        inside_own_folders(
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\pbt\database",
            &resolved
        ),
        None,
        "a sibling with the same prefix is not inside"
    );
    assert_eq!(inside_own_folders("not a path", &resolved), None);
}

#[test]
fn ids_are_pinned_because_the_same_folder_must_get_the_same_id_for_ever() {
    let settings = Settings::default();
    // The folder name in the id alphabet, then 32 bits of FNV-1a of the
    // lower-cased path: changing either changes every id ever handed out.
    assert_eq!(
        new_id(&settings, r"\\?\C:\Work\Reports"),
        "reports-cbc205f3"
    );
    assert_eq!(
        new_id(&settings, r"\\?\C:\WORK\reports"),
        "reports-cbc205f3"
    );
    assert_eq!(new_id(&settings, r"\\?\UNC\nas\team\docs"), "docs-8f17c1c8");
    assert_eq!(new_id(&settings, r"\\?\C:\"), "c-913f8f29");
}

#[test]
fn two_folders_with_the_same_name_get_different_ids() {
    let settings = Settings::default();
    let a = new_id(&settings, r"\\?\C:\Work\Reports");
    let b = new_id(&settings, r"\\?\D:\Other\Reports");
    assert_eq!(b, "reports-6a4d8bb1");
    assert_ne!(a, b);
}

#[test]
fn a_clash_walks_through_the_counters_up_to_the_cap() {
    // Every watch the file may hold sits on the first ids of one folder; the
    // next add still finds a free counter.
    let mut settings = Settings::default();
    let base = new_id(&settings, r"\\?\C:\Same");
    settings.watches.push(Watch {
        id: base.clone(),
        path: String::from(r"\\?\D:\x0"),
        ..Watch::default()
    });
    for n in 2..MAX_WATCHES {
        settings.watches.push(Watch {
            id: format!("{base}-{n}"),
            path: format!(r"\\?\D:\x{n}"),
            ..Watch::default()
        });
    }
    let next = new_id(&settings, r"\\?\C:\Same");
    assert_eq!(next, format!("{base}-{MAX_WATCHES}"));
    assert!(next.len() <= 64, "{next}");
}
