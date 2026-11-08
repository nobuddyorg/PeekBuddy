//! Desktop notifications (#81): switching them on creates the Start-menu
//! shortcut that carries the toast identity, the toast API accepts a toast
//! under that identity, and switching them off removes the shortcut. A
//! toast that is not displayed (a server image without a notification
//! surface) is not an error of the call, so display is not asserted.
#![cfg(windows)]

use peekbuddy_engine::rules_win::{AUMID, sync_shortcut};

#[test]
fn the_shortcut_is_created_the_toast_call_succeeds_and_the_shortcut_is_removed() {
    let programs =
        known_folders::get_known_folder_path(known_folders::KnownFolder::Programs).unwrap();
    let link = programs.join("PeekBuddy.lnk");
    if link.exists() {
        // A developer's own shortcut: not ours to remove.
        return;
    }
    sync_shortcut(true);
    assert_eq!(
        peekbuddy_win::aumid::shortcut_id(&link).unwrap().as_deref(),
        Some(AUMID)
    );
    let shown = tauri_winrt_notification::Toast::new(AUMID)
        .title("PeekBuddy test")
        .text1("A rule fired.")
        .show();
    sync_shortcut(false);
    assert!(!link.exists(), "switching off removes the shortcut");
    shown.unwrap();
}
