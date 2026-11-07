//! A lost root, seen from the viewer's side of the pipe (#68, F19): the real
//! engine watches a temp folder, the folder is deleted and made again, and a
//! pipe client that asks for the watch list sees `Lost` in the first list
//! that reports a change and the watch back to `Live` afterwards, with
//! `Restored` among the logged states. The viewer's mapping turns each list
//! into the label the watches screen shows. Runs in the `fs-serial` group on
//! real NTFS.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

#[path = "support/engine.rs"]
mod engine;

use std::path::Path;
use std::time::{Duration, Instant};

use engine::{all_rows, exe, quit, spawn, wait_for_window, wait_until_settled};
use peekbuddy_core::event::{Kind, WatchState};
use peekbuddy_core::ipc::{Message, PROTOCOL_VERSION, WatchInfo};
use peekbuddy_testkit::TestDir;
use peekbuddy_viewer::watches::present;
use peekbuddy_win::pipe::PipeClient;

/// The state of the watch `w0` as the engine reports it, with the label the
/// watches screen would show.
fn ask(client: &mut PipeClient) -> (WatchInfo, &'static str) {
    client.send(&Message::ListWatches).unwrap();
    let Some(Message::WatchList(mut watches)) = client.receive().unwrap() else {
        unreachable!("the list is answered with a list");
    };
    assert_eq!(watches.len(), 1, "{watches:?}");
    let watch = watches.remove(0);
    let label = present(&watch).label;
    (watch, label)
}

/// Asks every 50 ms until `done` accepts the answer; returns every state seen
/// on the way, oldest first, without repeats.
fn until(
    client: &mut PipeClient,
    what: &str,
    mut done: impl FnMut(&WatchInfo, &'static str) -> bool,
) -> Vec<(WatchState, &'static str)> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen: Vec<(WatchState, &'static str)> = Vec::new();
    loop {
        let (watch, label) = ask(client);
        if seen.last() != Some(&(watch.state, label)) {
            seen.push((watch.state, label));
        }
        if done(&watch, label) {
            return seen;
        }
        assert!(
            Instant::now() < deadline,
            "no {what} within 60 s; saw {seen:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn connect(data: &Path) -> PipeClient {
    let name = peekbuddy_engine::ipc_server::pipe_name(Some(data)).unwrap();
    let mut client =
        PipeClient::connect(&name, Some(exe().as_path()), Duration::from_secs(10)).unwrap();
    client
        .send(&Message::Hello {
            protocol_version: PROTOCOL_VERSION,
            build: String::from("test"),
        })
        .unwrap();
    assert!(matches!(
        client.receive().unwrap(),
        Some(Message::Hello { .. })
    ));
    client
}

#[test]
fn a_deleted_root_shows_lost_and_comes_back_live_with_a_restored_row() {
    let dir = TestDir::new("watch-state").unwrap();
    let data = dir.path().join("data");
    let root = dir.path().join("tree");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(root.join("old")).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let plain = root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n[[watch]]\nid = \"w0\"\npath = '{plain}'\n"
        ),
    )
    .unwrap();
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    wait_until_settled(&data, Duration::from_secs(3));
    let mut client = connect(&data);

    // Armed and recording.
    let seen = until(&mut client, "Live", |watch, _| {
        watch.armed && watch.state == WatchState::Live
    });
    assert_eq!(seen.last(), Some(&(WatchState::Live, "Live")), "{seen:?}");

    // The root goes away: the very list that reports a change says Lost, and
    // the screen would label it so.
    std::fs::remove_dir_all(&root).unwrap();
    let (first_change, label) = {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let (watch, label) = ask(&mut client);
            if watch.state != WatchState::Live {
                break (watch, label);
            }
            assert!(
                Instant::now() < deadline,
                "the deleted root was not noticed"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert_eq!(first_change.state, WatchState::Lost, "{first_change:?}");
    assert!(first_change.armed);
    assert_eq!(label, "Lost");

    // It stays lost while the folder is missing.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(ask(&mut client).0.state, WatchState::Lost);

    // The folder returns: the watch comes back (Restored while it scans) and
    // ends Live again.
    std::fs::create_dir_all(root.join("new")).unwrap();
    let seen = until(&mut client, "Live after Restored", |watch, _| {
        watch.state == WatchState::Live
    });
    assert!(
        seen.iter().all(|(state, _)| matches!(
            state,
            WatchState::Lost | WatchState::Restored | WatchState::Reconciling | WatchState::Live
        )),
        "{seen:?}"
    );
    for (state, label) in &seen {
        if *state == WatchState::Restored {
            assert_eq!(*label, "Restored");
        }
    }
    std::fs::write(root.join("new").join("f.txt"), b"x").unwrap();
    wait_until_settled(&data, Duration::from_secs(4));
    quit(window, &mut engine.0);

    // The log has the whole story, in order.
    let states: Vec<WatchState> = all_rows(&data)
        .iter()
        .filter_map(|row| match row.kind {
            Kind::WatchStateChanged(state) => Some(state),
            _ => None,
        })
        .collect();
    let lost = states.iter().position(|state| *state == WatchState::Lost);
    let restored = states
        .iter()
        .position(|state| *state == WatchState::Restored);
    assert!(lost.is_some(), "{states:?}");
    assert!(restored > lost, "{states:?}");
}
