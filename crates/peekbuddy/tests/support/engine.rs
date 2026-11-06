//! Starting, stopping and reading the real engine in the end-to-end suites.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_core::event::Kind;
use peekbuddy_core::query::{CancelFlag, Direction, Filter, PageRequest, Row};
use peekbuddy_engine::Names;
use peekbuddy_store::read::Reader;
use peekbuddy_win::window::WindowHandle;

pub const EXE: &str = env!("CARGO_BIN_EXE_peekbuddy");

pub struct Kill(pub Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The exe to run: `PEEKBUDDY_PERF_EXE` when the perf harness measures another
/// build (a base exe for the A/B, a build with a deliberate regression), else
/// the one of this workspace.
///
/// The harness hands over a canonical `\\?\` path; the pipe check compares it
/// with the image name the OS reports for the server, which has no prefix.
pub fn exe() -> std::path::PathBuf {
    let path = std::env::var_os("PEEKBUDDY_PERF_EXE")
        .map_or_else(|| std::path::PathBuf::from(EXE), std::path::PathBuf::from);
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map_or_else(|| path.clone(), std::path::PathBuf::from)
}

pub fn spawn(data: &Path) -> Kill {
    Kill(
        Command::new(exe())
            .args(["--headless", "--data-dir"])
            .arg(data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

pub fn wait_for_window(data: &Path, child: &mut Child) -> WindowHandle {
    let class = Names::new(Some(data)).window_class;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(window) = WindowHandle::find(&class) {
            return window;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "the engine exited before its window appeared"
        );
        assert!(Instant::now() < deadline, "no engine window after 60 s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn quit(window: WindowHandle, child: &mut Child) {
    quit_within(window, child, Duration::from_secs(120));
}

/// Closing flushes the rows of a burst to a disk a hosted runner shares; the
/// perf harness lets the engine take as long as it needs and reports it.
pub fn quit_within(window: WindowHandle, child: &mut Child, limit: Duration) {
    window.post_close().unwrap();
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{status:?}");
            return;
        }
        assert!(Instant::now() < deadline, "the engine did not quit in time");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Every row of the log, oldest first.
pub fn all_rows(data: &Path) -> Vec<Row> {
    let reader = Reader::new(&data.join("segments"));
    let cancel = CancelFlag::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader
            .page(
                &PageRequest {
                    generation: 0,
                    filter: Filter::default(),
                    cursor,
                    direction: Direction::Newer,
                    limit: 5_000,
                },
                &cancel,
            )
            .unwrap();
        rows.extend(page.rows.iter().rev().cloned());
        cursor = page.newest_id();
        if !page.more || cursor.is_none() {
            return rows;
        }
    }
}

/// The newest row id, or 0 while there is no segment yet.
pub fn newest_id(data: &Path) -> i64 {
    if !data.join("segments").join("catalog.db").exists() {
        return 0;
    }
    let reader = Reader::new(&data.join("segments"));
    reader
        .page(
            &PageRequest::newest(0, Filter::default()),
            &CancelFlag::new(),
        )
        .ok()
        .and_then(|page| page.newest_id())
        .unwrap_or(0)
}

/// Waits until the log has not grown for `quiet` and every reconcile that
/// started has finished.
pub fn wait_until_settled(data: &Path, quiet: Duration) {
    let limit = Duration::from_secs(180);
    let mut deadline = Instant::now() + limit;
    let mut last = -1;
    let mut since = Instant::now();
    loop {
        let now = newest_id(data);
        if now != last {
            // A log that still grows is a spill being drained: the limit is
            // for a log that stands still without being closed.
            last = now;
            since = Instant::now();
            deadline = since + limit;
        } else if since.elapsed() >= quiet && reconciles_balanced(data) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the log never settled: {}",
            reconcile_state(data)
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The reconcile rows of the log, for the message of a log that never settles.
fn reconcile_state(data: &Path) -> String {
    let rows = all_rows(data);
    let walks: Vec<String> = rows
        .iter()
        .filter(|r| matches!(r.kind, Kind::ReconcileStarted | Kind::ReconcileFinished))
        .map(|r| format!("{}:{:?}:{}", r.id, r.kind, r.raw_mask))
        .collect();
    let newest = rows.last().map(|r| r.id);
    format!("{} rows, newest {newest:?}, walks {walks:?}", rows.len())
}

pub fn reconciles_balanced(data: &Path) -> bool {
    let rows = all_rows(data);
    let started = rows
        .iter()
        .filter(|r| r.kind == Kind::ReconcileStarted)
        .count();
    let finished = rows
        .iter()
        .filter(|r| r.kind == Kind::ReconcileFinished)
        .count();
    started == finished
}
