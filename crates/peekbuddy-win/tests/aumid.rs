//! The `AppUserModelID` shortcut (`aumid`, #81): the id written is the id read
//! back, a second write replaces the first, and a folder that is not there
//! is an error and not a panic.
#![cfg(all(windows, feature = "aumid"))]

use peekbuddy_win::aumid::{create_shortcut, shortcut_id};

#[expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]
fn temp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-aumid-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_id_written_is_the_id_read_back_and_a_second_write_replaces_it() {
    let dir = temp("roundtrip");
    let link = dir.join("PeekBuddy.lnk");
    let exe = std::env::current_exe().unwrap();
    create_shortcut(&link, &exe, "nobuddy.PeekBuddy.test").unwrap();
    assert!(link.exists());
    assert_eq!(
        shortcut_id(&link).unwrap().as_deref(),
        Some("nobuddy.PeekBuddy.test")
    );
    create_shortcut(&link, &exe, "nobuddy.PeekBuddy.other").unwrap();
    assert_eq!(
        shortcut_id(&link).unwrap().as_deref(),
        Some("nobuddy.PeekBuddy.other")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_missing_folder_or_file_is_an_error() {
    let dir = temp("missing");
    let exe = std::env::current_exe().unwrap();
    assert!(create_shortcut(&dir.join("no\\such\\dir\\x.lnk"), &exe, "id").is_err());
    assert!(shortcut_id(&dir.join("absent.lnk")).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
