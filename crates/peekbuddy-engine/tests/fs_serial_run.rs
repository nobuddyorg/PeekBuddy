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

use peekbuddy_core::event::Kind;
use peekbuddy_core::perf::MilestoneTable;
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest};
use peekbuddy_engine::ipc_server::{ping, pipe_name};
use peekbuddy_engine::{Exit, Names, Options, ProbeOptions, register_provider, run};
use peekbuddy_store::read::Reader;
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
    let until = Instant::now() + Duration::from_secs(30);
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
        kinds.contains(&Kind::WatchStateChanged(
            peekbuddy_core::event::WatchState::Live
        )),
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
    assert!(
        report["counters"]["rowsPersisted"].as_u64().unwrap() >= 1,
        "{report}"
    );
}
