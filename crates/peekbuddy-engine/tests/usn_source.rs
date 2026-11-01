//! The USN source against the journal of the volume that holds the temp
//! directory (#212). Read-only: the journal is never created, deleted or
//! resized. A volume without a journal makes every test return early.
#![cfg(all(windows, feature = "usn"))]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, GapReason, Kind, RootIdx, Source, VolIdx};
use peekbuddy_core::usn::checkpoint::Checkpoint;
use peekbuddy_engine::pause_ctl::now_ticks;
use peekbuddy_engine::usn_source::{Checkpoints, OnGap, UsnRunner, UsnSpec};

type Rows = Arc<Mutex<Vec<(Event, String)>>>;

struct Collect(Rows);

impl EventSink for Collect {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        self.0
            .lock()
            .unwrap()
            .push((*event, String::from_utf16_lossy(name)));
        true
    }
}

#[derive(Clone, Default)]
struct Saved(Arc<Mutex<Option<Checkpoint>>>);

impl Checkpoints for Saved {
    fn load(&self) -> Option<Checkpoint> {
        *self.0.lock().unwrap()
    }

    fn save(&self, checkpoint: Checkpoint) {
        *self.0.lock().unwrap() = Some(checkpoint);
    }
}

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pb-usn-src-{tag}-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Started {
    runner: UsnRunner,
    rows: Rows,
    saved: Saved,
    gaps: Arc<Mutex<Vec<GapReason>>>,
}

/// Starts a runner on `dir`; `None` when the volume has no journal.
fn start(dir: &Dir, saved: &Saved) -> Option<Started> {
    let rows: Rows = Arc::default();
    let gaps: Arc<Mutex<Vec<GapReason>>> = Arc::default();
    let on_gap: OnGap = {
        let gaps = Arc::clone(&gaps);
        Box::new(move |reason| gaps.lock().unwrap().push(reason))
    };
    let spec = UsnSpec {
        vol: VolIdx(1),
        root: RootIdx(2),
        path: dir.path(),
    };
    match UsnRunner::start(
        &spec,
        Box::new(saved.clone()),
        Box::new(Collect(Arc::clone(&rows))),
        on_gap,
        now_ticks,
    ) {
        Ok(runner) => Some(Started {
            runner,
            rows,
            saved: saved.clone(),
            gaps,
        }),
        // ERROR_INVALID_FUNCTION (1) or ERROR_JOURNAL_NOT_ACTIVE (1179).
        Err(error) if matches!(error.raw_os_error(), Some(1 | 1179)) => None,
        Err(error) => {
            assert_eq!(error.to_string(), "", "start failed");
            None
        }
    }
}

/// Waits until a row with `name` and `kind` is there.
fn wait_for(rows: &Rows, name: &str, kind: Kind) -> Event {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some((event, _)) = rows
            .lock()
            .unwrap()
            .iter()
            .find(|(event, n)| n == name && event.kind == kind)
        {
            return *event;
        }
        assert!(
            Instant::now() < deadline,
            "no {kind:?} row for {name}; rows: {:?}",
            rows.lock()
                .unwrap()
                .iter()
                .map(|(_, n)| n.clone())
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn gap_rows(rows: &Rows) -> Vec<GapReason> {
    rows.lock()
        .unwrap()
        .iter()
        .filter_map(|(event, _)| match event.kind {
            Kind::Overflow(reason) => Some(reason),
            _ => None,
        })
        .collect()
}

#[test]
fn files_created_under_the_root_arrive_with_names_relative_to_it() {
    let dir = Dir::new("live");
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    let saved = Saved::default();
    let Some(mut started) = start(&dir, &saved) else {
        return;
    };
    std::fs::write(dir.path().join("sub").join("seen.txt"), b"x").unwrap();
    std::fs::write(dir.path().join("top.txt"), b"x").unwrap();
    let nested = wait_for(&started.rows, r"sub\seen.txt", Kind::Created);
    let top = wait_for(&started.rows, "top.txt", Kind::Created);
    assert_eq!(nested.source, Source::Usn);
    assert_eq!((nested.vol, nested.root), (VolIdx(1), RootIdx(2)));
    assert_ne!(nested.usn, 0);
    assert_ne!(nested.file_id, 0);
    assert_ne!(top.file_id, nested.file_id);
    // The first start had no checkpoint: a fresh read, no gap.
    assert_eq!(gap_rows(&started.rows), []);
    assert!(started.runner.stop().is_none());
    assert!(started.saved.load().is_some(), "the position was saved");
    assert!(started.runner.counters().records >= 2);
}

#[test]
fn a_restart_with_an_intact_journal_continues_without_a_gap() {
    let dir = Dir::new("restart");
    let saved = Saved::default();
    let Some(mut first) = start(&dir, &saved) else {
        return;
    };
    std::fs::write(dir.path().join("before.txt"), b"x").unwrap();
    wait_for(&first.rows, "before.txt", Kind::Created);
    assert!(first.runner.stop().is_none());

    // Written while nobody listens.
    std::fs::write(dir.path().join("between.txt"), b"x").unwrap();

    let Some(mut second) = start(&dir, &saved) else {
        return;
    };
    wait_for(&second.rows, "between.txt", Kind::Created);
    assert_eq!(gap_rows(&second.rows), []);
    assert_eq!(*second.gaps.lock().unwrap(), []);
    // Nothing is delivered twice: the create of `before.txt` was before the
    // saved position. (Later records of the same file, a write or the close,
    // may follow it and are not duplicates.)
    let created_again = second
        .rows
        .lock()
        .unwrap()
        .iter()
        .filter(|(event, name)| name == "before.txt" && event.kind == Kind::Created)
        .count();
    assert_eq!(created_again, 0);
    assert!(second.runner.stop().is_none());
}

#[test]
fn a_changed_journal_id_gives_exactly_one_gap_row_and_a_reconcile_request() {
    let dir = Dir::new("idchange");
    let saved = Saved::default();
    let Some(mut first) = start(&dir, &saved) else {
        return;
    };
    assert!(first.runner.stop().is_none());
    let mut position = saved.load().unwrap();
    // The journal was recreated under the saved position's feet.
    position.journal_id ^= 1;
    saved.save(position);

    let Some(mut second) = start(&dir, &saved) else {
        return;
    };
    assert_eq!(gap_rows(&second.rows), [GapReason::JournalIdChanged]);
    assert_eq!(*second.gaps.lock().unwrap(), [GapReason::JournalIdChanged]);
    // The gap row is the root's own row: no name, source USN.
    let rows = second.rows.lock().unwrap().clone();
    let (gap, name) = rows.first().unwrap();
    assert_eq!(gap.source, Source::Usn);
    assert_eq!(name, "");
    // Reading goes on after the gap, from the journal's end.
    std::fs::write(dir.path().join("after.txt"), b"x").unwrap();
    wait_for(&second.rows, "after.txt", Kind::Created);
    assert_eq!(gap_rows(&second.rows).len(), 1);
    // The position now names the live journal again.
    assert_ne!(saved.load().unwrap().journal_id, position.journal_id);
    assert!(second.runner.stop().is_none());
}

#[test]
fn stopping_ends_a_pending_read_at_once() {
    let dir = Dir::new("stop");
    let saved = Saved::default();
    let Some(mut started) = start(&dir, &saved) else {
        return;
    };
    std::thread::sleep(Duration::from_millis(200));
    let begun = Instant::now();
    assert!(started.runner.stop().is_none());
    assert!(begun.elapsed() < Duration::from_secs(5));
    // A second stop is a no-op.
    assert!(started.runner.stop().is_none());
}

#[test]
fn a_renamed_file_logs_both_names() {
    let dir = Dir::new("rename");
    let saved = Saved::default();
    let Some(mut started) = start(&dir, &saved) else {
        return;
    };
    std::fs::write(dir.path().join("old.txt"), b"x").unwrap();
    wait_for(&started.rows, "old.txt", Kind::Created);
    std::fs::rename(dir.path().join("old.txt"), dir.path().join("new.txt")).unwrap();
    wait_for(&started.rows, "new.txt", Kind::RenamedTo);
    assert!(started.runner.stop().is_none());
}
