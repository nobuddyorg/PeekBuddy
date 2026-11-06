//! `peekbuddy_engine::run` itself, in this process (#234): the engine starts
//! on a data directory with a watch, answers on its window and its pipe,
//! records what happens in the watched tree and what the previous run left
//! behind, tells the probe what it did and leaves its marks in the data
//! directory. The same start-up is tested end to end through the exe in the
//! `peekbuddy` crate; here the engine's own crate sees it, which is the crate
//! a mutant of `run` is tested by.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use peekbuddy_core::event::{Kind, WatchState};
use peekbuddy_core::ipc::{Command as IpcCommand, Message, PROTOCOL_VERSION};
use peekbuddy_core::perf::MilestoneTable;
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest};
use peekbuddy_engine::ipc_server::{ping, pipe_name};
use peekbuddy_engine::{Exit, Names, Options, ProbeOptions, register_provider, run};
use peekbuddy_store::read::Reader;
use peekbuddy_win::pipe::PipeClient;
use peekbuddy_win::window::WindowHandle;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pb-run-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    // A hosted arm runner is slow to start an engine and to rearm a watch.
    let until = Instant::now() + Duration::from_secs(90);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn rows(data: &Path) -> Vec<peekbuddy_core::query::Row> {
    let reader = Reader::new(&data.join("segments"));
    reader
        .page(
            &PageRequest {
                generation: 0,
                filter: Filter::default(),
                cursor: None,
                direction: Direction::Older,
                limit: 5_000,
            },
            &CancelFlag::new(),
        )
        .unwrap()
        .rows
}

#[test]
fn the_engine_runs_answers_records_and_reports() {
    let dir = Dir::new("engine");
    let data = dir.0.join("data");
    let tree = dir.0.join("tree");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&tree).unwrap();
    let tree = std::fs::canonicalize(&tree).unwrap();
    let plain = tree
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
    // The previous run "crashed": the marker names its log.
    std::fs::write(data.join("crash.marker"), "crash-20261102T000000Z.log").unwrap();

    let probe_out = dir.0.join("probe.json");
    let options = Options {
        headless: true,
        data_dir: Some(data.clone()),
        probe: Some(ProbeOptions {
            seconds: 6,
            out: Some(probe_out.clone()),
        }),
        ..Options::default()
    };
    let table = Arc::new(MilestoneTable::new());
    let engine = {
        let table = Arc::clone(&table);
        std::thread::spawn(move || run(&options, &table, register_provider()))
    };

    // It answers on its window and on its pipe.
    let class = Names::new(Some(&data)).window_class;
    wait_for("the engine window", || WindowHandle::find(&class).is_some());
    let pipe = pipe_name(Some(&data)).unwrap();
    let exe = std::env::current_exe().unwrap();
    wait_for("the engine pipe", || ping(&pipe, &exe).is_ok());

    // It records what happens in the watched tree.
    wait_for("the watch to be armed", || {
        std::fs::write(tree.join("seen.txt"), b"x").is_ok()
            && rows(&data).iter().any(|row| {
                row.kind == Kind::Created
                    && row
                        .path
                        .ends_with(&"seen.txt".encode_utf16().collect::<Vec<_>>())
            })
    });

    let exit = engine.join().unwrap().unwrap();
    let (abnormal_termination, settings) = match exit {
        Exit::Ran {
            abnormal_termination,
            settings,
        } => (abnormal_termination, settings),
        Exit::AlreadyRunning => (None, None),
    };
    assert_eq!(
        abnormal_termination.as_deref(),
        Some("crash-20261102T000000Z.log")
    );
    assert!(settings.is_some_and(|how| !how.is_empty()));
    assert!(!data.join("crash.marker").exists(), "the marker is taken");

    // The abnormal exit of the previous run is a row; so are the baseline of
    // the watch and its state.
    let logged = rows(&data);
    let kinds: Vec<Kind> = logged.iter().map(|row| row.kind).collect();
    assert!(kinds.contains(&Kind::EngineAbnormalExit), "{kinds:?}");
    assert!(kinds.contains(&Kind::BaselineEstablished), "{kinds:?}");
    assert!(
        kinds.contains(&Kind::WatchStateChanged(WatchState::Live)),
        "{kinds:?}"
    );
    assert!(
        logged.iter().any(|row| row.kind == Kind::EngineAbnormalExit
            && String::from_utf16_lossy(&row.name).contains("crash-20261102T000000Z.log")),
        "the row names the crash log"
    );

    // The data directory has what the engine makes.
    assert!(data.join("segments").join("catalog.db").exists());
    assert!(data.join("logs").is_dir());

    check_probe(&probe_out);
}

fn check_probe(probe_out: &Path) {
    // The probe says what it saw.
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(probe_out).unwrap()).unwrap();
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    assert_eq!(report["arch"], arch);
    assert_eq!(report["requestedSeconds"], 6);
    let names: Vec<String> = report["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap().to_owned())
        .collect();
    for wanted in [
        "SettingsLoaded",
        "StoreOpened",
        "WatcherArmed",
        "EngineReady",
    ] {
        assert!(
            names.iter().any(|name| name == wanted),
            "{wanted} in {names:?}"
        );
    }
    // Ready is the last of the steps: after the watch was armed and after the
    // store was open.
    let first = |wanted: &str| {
        report["milestones"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == wanted)
            .map(|row| row["firstUs"].as_u64().unwrap())
            .unwrap()
    };
    assert!(first("EngineReady") >= first("WatcherArmed"));
    assert!(first("EngineReady") >= first("StoreOpened"));
    assert!(
        report["counters"]["rowsPersisted"].as_u64().unwrap() >= 1,
        "{report}"
    );
}

/// An engine running in this process on a data directory with one watch, and
/// what it needs to be stopped again.
struct Running {
    dir: Dir,
    data: PathBuf,
    tree: PathBuf,
    class: String,
    engine: Option<std::thread::JoinHandle<std::io::Result<Exit>>>,
}

impl Running {
    /// `watch` is made relative to the tree; the directories named by
    /// `create` exist before the start.
    fn start(tag: &str, watch: &str, create: &str) -> Self {
        let dir = Dir::new(tag);
        let data = dir.0.join("data");
        let tree = dir.0.join("tree");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(tree.join(create)).unwrap();
        let tree = std::fs::canonicalize(&tree).unwrap();
        let plain = tree
            .join(watch)
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
        let options = Options {
            headless: true,
            data_dir: Some(data.clone()),
            ..Options::default()
        };
        let table = Arc::new(MilestoneTable::new());
        let engine = std::thread::spawn(move || run(&options, &table, register_provider()));
        let class = Names::new(Some(&data)).window_class;
        wait_for("the engine window", || WindowHandle::find(&class).is_some());
        Self {
            dir,
            data,
            tree,
            class,
            engine: Some(engine),
        }
    }

    /// Stops the engine and starts it again on the same data directory;
    /// `while_down` runs in between, when nothing watches the tree: that time
    /// is a gap, which the next start reconciles.
    fn restart(&mut self, while_down: impl FnOnce(&Self)) {
        let window = WindowHandle::find(&self.class).unwrap();
        window.post_close().unwrap();
        let exit = self.engine.take().unwrap().join().unwrap().unwrap();
        assert!(matches!(exit, Exit::Ran { .. }), "{exit:?}");
        while_down(self);
        let options = Options {
            headless: true,
            data_dir: Some(self.data.clone()),
            ..Options::default()
        };
        let table = Arc::new(MilestoneTable::new());
        self.engine = Some(std::thread::spawn(move || {
            run(&options, &table, register_provider())
        }));
        let class = self.class.clone();
        wait_for("the engine window", || WindowHandle::find(&class).is_some());
    }

    fn has_state(&self, state: WatchState) -> bool {
        rows(&self.data)
            .iter()
            .any(|row| row.kind == Kind::WatchStateChanged(state))
    }

    fn state_rows(&self, state: WatchState) -> usize {
        rows(&self.data)
            .iter()
            .filter(|row| row.kind == Kind::WatchStateChanged(state))
            .count()
    }

    fn has_row(&self, kind: Kind, name: &str) -> bool {
        let wanted: Vec<u16> = name.encode_utf16().collect();
        rows(&self.data)
            .iter()
            .any(|row| row.kind == kind && row.path.ends_with(&wanted))
    }

    fn command(&self, command: IpcCommand) {
        let pipe = pipe_name(Some(&self.data)).unwrap();
        let exe = std::env::current_exe().unwrap();
        let mut client = PipeClient::connect(&pipe, Some(&exe), Duration::from_secs(10)).unwrap();
        client
            .send(&Message::Hello {
                protocol_version: PROTOCOL_VERSION,
                build: String::from("test"),
            })
            .unwrap();
        let _ = client.receive().unwrap();
        client.send(&Message::Command(command)).unwrap();
        let _ = client.receive().unwrap();
    }

    /// Waits for `done`; a timeout says what the log held, so a failure on
    /// a runner shows what the engine did.
    fn wait(&self, what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(90);
        while !done() {
            assert!(
                Instant::now() < until,
                "timed out waiting for {what}; the log holds {:?}",
                rows(&self.data)
                    .iter()
                    .map(|row| (row.kind, String::from_utf16_lossy(&row.path)))
                    .collect::<Vec<_>>()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn stop(mut self) {
        let window = WindowHandle::find(&self.class).unwrap();
        window.post_close().unwrap();
        let exit = self.engine.take().unwrap().join().unwrap().unwrap();
        assert!(matches!(exit, Exit::Ran { .. }), "{exit:?}");
        drop(self.dir);
    }
}

fn wait_for_armed(engine: &Running, file: &str) {
    // A file written before the watch is armed is never a Created row, so
    // every try makes it new again.
    engine.wait("the watch to be armed", || {
        let path = engine.tree.join(file);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"x").is_ok() && engine.has_row(Kind::Created, file)
    });
}

#[test]
fn a_root_that_is_deleted_and_comes_back_is_watched_again() {
    let engine = Running::start("lost", "", "");
    wait_for_armed(&engine, "armed.txt");
    let lives = engine.state_rows(WatchState::Live);

    std::fs::remove_dir_all(&engine.tree).unwrap();
    engine.wait("the loss of the root", || {
        engine.has_state(WatchState::Lost)
    });
    std::fs::create_dir_all(&engine.tree).unwrap();
    engine.wait("the root to be watched again", || {
        engine.state_rows(WatchState::Live) > lives || engine.has_state(WatchState::Restored)
    });
    engine.wait("a file of the new root", || {
        std::fs::write(engine.tree.join("again.txt"), b"x").is_ok()
            && engine.has_row(Kind::Created, "again.txt")
    });
    engine.stop();
}

#[test]
fn a_root_two_levels_below_a_deleted_directory_is_watched_once_it_exists_again() {
    let engine = Running::start("nested", r"a\b", r"a\b");
    wait_for_armed(&engine, r"a\b\armed.txt");
    let lives = engine.state_rows(WatchState::Live);

    std::fs::remove_dir_all(engine.tree.join("a")).unwrap();
    engine.wait("the loss of the root", || {
        engine.has_state(WatchState::Lost)
    });
    std::fs::create_dir_all(engine.tree.join(r"a\b")).unwrap();
    engine.wait("the root to be watched again", || {
        engine.state_rows(WatchState::Live) > lives || engine.has_state(WatchState::Restored)
    });
    engine.wait("a file below the root", || {
        std::fs::write(engine.tree.join(r"a\b\file.txt"), b"x").is_ok()
            && engine.has_row(Kind::Created, "file.txt")
    });
    engine.stop();
}

#[test]
fn pausing_a_watch_is_a_row_and_nothing_is_recorded_until_it_resumes() {
    let engine = Running::start("pause", "", "");
    wait_for_armed(&engine, "before.txt");
    let lives = engine.state_rows(WatchState::Live);

    engine.command(IpcCommand::PauseAll);
    engine.wait("the pause", || {
        engine.has_state(WatchState::SuspendedByUser)
    });
    std::fs::write(engine.tree.join("while-paused.txt"), b"x").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(!engine.has_row(Kind::Created, "while-paused.txt"));

    engine.command(IpcCommand::ResumeAll);
    engine.wait("the watch to be live again", || {
        engine.state_rows(WatchState::Live) > lives
    });
    engine.wait("a file after the resume", || {
        std::fs::write(engine.tree.join("after.txt"), b"x").is_ok()
            && engine.has_row(Kind::Created, "after.txt")
    });
    engine.stop();
}

#[test]
fn an_export_into_the_data_dir_causes_no_event_even_when_a_watch_contains_it() {
    // The watch root is the folder that holds both the tree and the data
    // directory, the case F16 exists for.
    let engine = Running::start("selfx", "..", "");
    wait_for_armed(&engine, "armed.txt");

    let exports = engine.data.join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    for round in 0..20 {
        std::fs::write(
            exports.join(format!("export-{round}.csv.partial")),
            b"a,b\n",
        )
        .unwrap();
        std::fs::rename(
            exports.join(format!("export-{round}.csv.partial")),
            exports.join(format!("export-{round}.csv")),
        )
        .unwrap();
    }
    // A file of the tree, written after the exports, is seen: whatever the
    // engine was going to log about the exports is in the log by then.
    engine.wait("a file of the tree after the exports", || {
        std::fs::write(engine.tree.join("after.txt"), b"x").is_ok()
            && engine.has_row(Kind::Created, "after.txt")
    });
    let leaked: Vec<String> = rows(&engine.data)
        .iter()
        .map(|row| String::from_utf16_lossy(&row.path))
        .filter(|path| path.contains("export") || path.contains("segments"))
        .collect();
    assert!(
        leaked.is_empty(),
        "the log has rows of its own files: {leaked:?}"
    );
    engine.stop();
}

#[test]
fn an_entry_created_live_and_deleted_while_the_engine_was_down_is_reported_deleted() {
    // The live stream logs the file as created; no walk has seen it, so the
    // snapshot does not know it; it is deleted during the gap. The walk of the
    // next start finds it neither on disk nor in the snapshot: only the log
    // of the live stream says it existed (#290).
    let mut engine = Running::start("livedel", "", "");
    wait_for_armed(&engine, "armed.txt");
    std::fs::write(engine.tree.join("transient.txt"), b"x").unwrap();
    engine.wait("the live row of the transient file", || {
        engine.has_row(Kind::Created, "transient.txt")
    });
    engine.restart(|engine| std::fs::remove_file(engine.tree.join("transient.txt")).unwrap());
    engine.wait("the walk to report the file deleted", || {
        engine.has_row(Kind::ReconciledDeleted, "transient.txt")
    });
    engine.stop();
}
