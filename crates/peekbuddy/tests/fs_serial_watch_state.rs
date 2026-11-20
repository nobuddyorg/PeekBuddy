//! A lost root, seen from the viewer's side of the pipe (#68, F19): the real
//! engine watches a temp folder, the folder is deleted and made again, and a
//! pipe client that asks for the watch list sees `Lost` in the first list
//! that reports a change and the watch back to `Live` afterwards, with
//! `Restored` among the logged states. The viewer's mapping turns each list
//! into the label the watches screen shows. A root added through the pipe
//! while the engine runs is armed at once (#553). Runs in the `fs-serial`
//! group on real NTFS.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

#[path = "support/engine.rs"]
mod engine;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine::{Kill, all_rows, exe, quit, spawn, wait_for_window, wait_until_settled};
use peekbuddy_core::event::{Kind, WatchState};
use peekbuddy_core::ipc::{Message, PROTOCOL_VERSION, WatchInfo};
use peekbuddy_testkit::TestDir;
use peekbuddy_viewer::watches::present;
use peekbuddy_win::pipe::PipeClient;
use peekbuddy_win::window::WindowHandle;

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

fn list(client: &mut PipeClient) -> Vec<WatchInfo> {
    client.send(&Message::ListWatches).unwrap();
    let Some(Message::WatchList(watches)) = client.receive().unwrap() else {
        unreachable!("the list is answered with a list");
    };
    watches
}

/// How many rows have a path that ends with `name`.
fn rows_named(data: &Path, name: &str) -> usize {
    all_rows(data)
        .iter()
        .filter(|row| {
            String::from_utf16_lossy(&row.path)
                .to_lowercase()
                .ends_with(name)
        })
        .count()
}

/// Two folders under a fresh temp dir, the first watched from the start as
/// `w0`, and the engine running on them with a pipe client attached.
struct Running {
    _dir: TestDir,
    data: PathBuf,
    first: PathBuf,
    added: PathBuf,
    engine: Kill,
    window: WindowHandle,
    client: PipeClient,
}

fn run_with_one_watch(tag: &str) -> Running {
    let dir = TestDir::new(tag).unwrap();
    let data = dir.path().join("data");
    let first = dir.path().join("first");
    let added = dir.path().join("added");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&added).unwrap();
    let first = std::fs::canonicalize(&first).unwrap();
    let plain = first
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
    until(&mut client, "w0 Live", |watch, _| {
        watch.armed && watch.state == WatchState::Live
    });
    Running {
        _dir: dir,
        data,
        first,
        added,
        engine,
        window,
        client,
    }
}

/// Sends `AddWatch` for `path`; returns the engine's answer.
fn add(client: &mut PipeClient, path: &Path) -> String {
    use peekbuddy_core::ipc::Command as Cmd;

    client
        .send(&Message::Command(Cmd::AddWatch {
            path: path.display().to_string(),
            confirmed: false,
        }))
        .unwrap();
    let Some(Message::CommandResult { ok, message }) = client.receive().unwrap() else {
        unreachable!("a command is answered with a result");
    };
    assert!(ok, "{message}");
    message
}

/// Waits until the second watch of the list is `Live`, at most 10 s; returns
/// its id.
fn second_live_within_seconds(client: &mut PipeClient) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let watches = list(client);
        if let Some(watch) = watches.get(1)
            && watch.state == WatchState::Live
        {
            assert!(watch.armed, "{watches:?}");
            assert_eq!(present(watch).label, "Live");
            return watch.id.clone();
        }
        assert!(
            Instant::now() < deadline,
            "the added root is not Live within 10 s: {watches:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits until a row's path ends with `name`.
fn wait_for_row(data: &Path, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while rows_named(data, name) == 0 {
        assert!(Instant::now() < deadline, "no row for {name}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn rows_of(data: &Path, kind: Kind) -> usize {
    all_rows(data).iter().filter(|row| row.kind == kind).count()
}

/// A root added through the pipe while the engine runs (#68, #355, #553):
/// it is armed at once, listed `Live` within seconds, its baseline covers the
/// time before the handle was armed, and a file created in it is a row while
/// the first root keeps recording. The next start arms it from the settings
/// and it captures again.
#[test]
fn a_root_added_while_running_is_live_within_seconds_and_captures_after_a_restart() {
    let Running {
        _dir,
        data,
        first,
        added,
        mut engine,
        window,
        mut client,
    } = run_with_one_watch("watch-added");

    // Add the second root through the command the viewer sends.
    let message = add(&mut client, &added);
    assert!(message.contains("starts capturing now"), "{message}");
    let watches = list(&mut client);
    assert_eq!(watches.len(), 2, "{watches:?}");
    assert!(watches[0].armed && watches[0].id == "w0", "{watches:?}");
    assert!(watches[1].armed, "{watches:?}");
    assert_ne!(present(&watches[1]).label, "Starts after restart");

    // Live within seconds, without a restart, and baselined.
    let new_id = second_live_within_seconds(&mut client);
    let deadline = Instant::now() + Duration::from_secs(60);
    while rows_of(&data, Kind::BaselineEstablished) < 2 {
        assert!(Instant::now() < deadline, "no baseline of the added root");
        std::thread::sleep(Duration::from_millis(100));
    }

    // A file in the new root is a row, and the first root still records.
    std::fs::write(added.join("during.txt"), b"x").unwrap();
    std::fs::write(first.join("marker.txt"), b"x").unwrap();
    wait_for_row(&data, "during.txt");
    wait_for_row(&data, "marker.txt");
    let watches = list(&mut client);
    assert!(watches[1].armed && watches[1].id == new_id, "{watches:?}");
    drop(client);
    quit(window, &mut engine.0);

    // The next start arms both roots; a file created in the new one is a row.
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    let mut client = connect(&data);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let watches = list(&mut client);
        if watches.len() == 2
            && watches
                .iter()
                .all(|w| w.armed && w.state == WatchState::Live)
        {
            break;
        }
        assert!(Instant::now() < deadline, "not both live: {watches:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_until_settled(&data, Duration::from_secs(3));
    std::fs::write(added.join("after.txt"), b"x").unwrap();
    wait_for_row(&data, "after.txt");
    drop(client);
    quit(window, &mut engine.0);
}

/// The first watch of an engine that started without any (the first-run
/// flow of the viewer, #553) is armed at once and records.
#[test]
fn the_first_watch_of_an_engine_without_watches_is_live_within_seconds() {
    let dir = TestDir::new("watch-first").unwrap();
    let data = dir.path().join("data");
    let added = dir.path().join("first");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&added).unwrap();
    std::fs::write(
        data.join("settings.toml"),
        "version = 1\n[general]\nfirst_run = false\n",
    )
    .unwrap();
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    let mut client = connect(&data);
    assert_eq!(list(&mut client), []);
    let message = add(&mut client, &added);
    assert!(message.contains("starts capturing now"), "{message}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let watches = list(&mut client);
        if watches.first().is_some_and(|w| w.state == WatchState::Live) {
            assert!(watches[0].armed, "{watches:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the first watch is not Live within 10 s: {watches:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    std::fs::write(added.join("hello.txt"), b"x").unwrap();
    wait_for_row(&data, "hello.txt");
    drop(client);
    quit(window, &mut engine.0);
}

/// A root added while the engine runs and removed again (#553) takes the
/// path of a root armed at start: it stops recording at once with a state
/// row and leaves the list, while the first root keeps recording.
#[test]
fn a_root_added_while_running_and_removed_again_stops_recording_at_once() {
    use peekbuddy_core::ipc::Command as Cmd;

    let Running {
        _dir,
        data,
        first,
        added,
        mut engine,
        window,
        mut client,
    } = run_with_one_watch("watch-added-removed");
    add(&mut client, &added);
    let new_id = second_live_within_seconds(&mut client);
    std::fs::write(added.join("kept.txt"), b"x").unwrap();
    wait_for_row(&data, "kept.txt");

    client
        .send(&Message::Command(Cmd::RemoveWatch { id: new_id }))
        .unwrap();
    let Some(Message::CommandResult { ok, message }) = client.receive().unwrap() else {
        unreachable!("a command is answered with a result");
    };
    assert!(ok, "{message}");
    let watches = list(&mut client);
    assert_eq!(watches.len(), 1, "{watches:?}");
    assert_eq!(watches[0].id, "w0");

    // Nothing more under the removed root: once the marker in the first root
    // is a row, the engine has had its chance at the file.
    std::fs::write(added.join("gone.txt"), b"x").unwrap();
    std::fs::write(first.join("marker.txt"), b"x").unwrap();
    wait_for_row(&data, "marker.txt");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(rows_named(&data, "gone.txt"), 0);
    drop(client);
    quit(window, &mut engine.0);
    assert_eq!(
        rows_of(&data, Kind::WatchStateChanged(WatchState::SuspendedByUser)),
        1,
        "the removal is one state row"
    );
}

/// A watch whose folder is missing at start (#553): its `Lost` row costs no
/// other root a row, and once the folder is made the watch comes back and a
/// file in it is a row.
#[test]
fn a_root_missing_at_start_records_once_its_folder_is_made() {
    let dir = TestDir::new("watch-missing").unwrap();
    let data = dir.path().join("data");
    let first = dir.path().join("first");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&first).unwrap();
    let first = std::fs::canonicalize(&first).unwrap();
    let plain = |path: &Path| {
        path.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned()
    };
    let later = first.with_file_name("later");
    std::fs::write(first.join("before.txt"), b"x").unwrap();
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n\
             [[watch]]\nid = \"w0\"\npath = '{}'\n\
             [[watch]]\nid = \"w1\"\npath = '{}'\n",
            plain(&first),
            plain(&later)
        ),
    )
    .unwrap();
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    wait_until_settled(&data, Duration::from_secs(3));
    // The first root's baseline is in the log next to the second's loss.
    let deadline = Instant::now() + Duration::from_secs(60);
    while rows_of(&data, Kind::BaselineEstablished) == 0 {
        assert!(
            Instant::now() < deadline,
            "no baseline of the first root: {:?}",
            all_rows(&data)
                .iter()
                .map(|r| (r.root.clone(), r.kind, String::from_utf16_lossy(&r.path)))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::write(first.join("marker.txt"), b"x").unwrap();
    wait_for_row(&data, "marker.txt");

    std::fs::create_dir_all(&later).unwrap();
    let mut client = connect(&data);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let watches = list(&mut client);
        if watches.get(1).is_some_and(|w| w.state == WatchState::Live) {
            break;
        }
        assert!(Instant::now() < deadline, "w1 not Live: {watches:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::write(later.join("arrived.txt"), b"x").unwrap();
    wait_for_row(&data, "arrived.txt");
    drop(client);
    quit(window, &mut engine.0);
}

/// Adding and removing a watch while the engine runs (#553, F8/#27): after
/// the third cycle the engine holds as many handles as after the first. A
/// removed watch closes its directory watch, its parent watches and its
/// plug-and-play registration at once. The first cycle warms up whatever is
/// opened lazily.
#[test]
fn add_and_remove_cycles_leave_the_handle_count_where_the_first_left_it() {
    use peekbuddy_core::ipc::Command as Cmd;
    use peekbuddy_win::metrics::handle_count;
    use std::os::windows::io::AsHandle;

    let Running {
        _dir,
        data,
        first: _,
        added,
        mut engine,
        window,
        mut client,
    } = run_with_one_watch("watch-handles");
    let mut after_first = 0;
    for cycle in 0..3 {
        add(&mut client, &added);
        let id = second_live_within_seconds(&mut client);
        let name = format!("cycle{cycle}.txt");
        std::fs::write(added.join(&name), b"x").unwrap();
        wait_for_row(&data, &name);
        client
            .send(&Message::Command(Cmd::RemoveWatch { id }))
            .unwrap();
        let Some(Message::CommandResult { ok, message }) = client.receive().unwrap() else {
            unreachable!("a command is answered with a result");
        };
        assert!(ok, "{message}");
        assert_eq!(list(&mut client).len(), 1);
        // The handles go a moment after the answer: the host closes them on
        // its own thread.
        let settled = |expected: Option<u32>| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let now = handle_count(engine.0.as_handle()).unwrap();
                if expected.is_none_or(|e| now <= e) || Instant::now() >= deadline {
                    return now;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        if cycle == 0 {
            after_first = settled(None);
        } else if cycle == 2 {
            let now = settled(Some(after_first));
            assert!(
                now <= after_first,
                "{now} handles after the third cycle, {after_first} after the first"
            );
        }
    }
    drop(client);
    quit(window, &mut engine.0);
}

/// Removing the first watch moves the second to position 0 at the next start
/// (#553). Its first walk must not diff its folder against the snapshot of
/// the removed one: no reconciled row for files that did not change.
#[test]
fn a_watch_that_moves_up_after_a_removal_writes_no_false_reconcile_rows() {
    use peekbuddy_core::ipc::Command as Cmd;

    let dir = TestDir::new("watch-moves-up").unwrap();
    let data = dir.path().join("data");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    for path in [&data, &first, &second] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(first.join("only-in-first.txt"), b"x").unwrap();
    std::fs::write(second.join("only-in-second.txt"), b"x").unwrap();
    let plain = |path: &Path| {
        std::fs::canonicalize(path)
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\?\")
            .to_owned()
    };
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n\
             [[watch]]\nid = \"w0\"\npath = '{}'\n\
             [[watch]]\nid = \"w1\"\npath = '{}'\n",
            plain(&first),
            plain(&second)
        ),
    )
    .unwrap();
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    let deadline = Instant::now() + Duration::from_secs(60);
    while rows_of(&data, Kind::BaselineEstablished) < 2 {
        assert!(Instant::now() < deadline, "no baselines");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut client = connect(&data);
    client
        .send(&Message::Command(Cmd::RemoveWatch {
            id: String::from("w0"),
        }))
        .unwrap();
    let Some(Message::CommandResult { ok, message }) = client.receive().unwrap() else {
        unreachable!("a command is answered with a result");
    };
    assert!(ok, "{message}");
    drop(client);
    quit(window, &mut engine.0);

    // The second watch is position 0 now.
    let mut engine = spawn(&data);
    let window = wait_for_window(&data, &mut engine.0);
    let deadline = Instant::now() + Duration::from_secs(60);
    while rows_of(&data, Kind::BaselineEstablished) < 3 {
        assert!(
            Instant::now() < deadline,
            "the moved watch was not baselined again"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    wait_until_settled(&data, Duration::from_secs(3));
    let reconciled: Vec<String> = all_rows(&data)
        .iter()
        .filter(|row| {
            matches!(
                row.kind,
                Kind::ReconciledCreated | Kind::ReconciledDeleted | Kind::ReconciledModified
            )
        })
        .map(|row| String::from_utf16_lossy(&row.path))
        .collect();
    assert!(
        reconciled.is_empty(),
        "false reconcile rows: {reconciled:?}"
    );
    quit(window, &mut engine.0);
}
