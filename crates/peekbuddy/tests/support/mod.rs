//! Helpers shared by the end-to-end suites that run the real engine on a tree
//! (#50, #57): start and stop it, read the log back, judge it with the churn
//! comparator.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_churn::compare::{Gap, Row as ChurnRow, compare};
use peekbuddy_churn::ops::RawKind;
use peekbuddy_core::event::{Kind, Source};
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

pub fn spawn(data: &Path) -> Kill {
    Kill(
        Command::new(EXE)
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
    window.post_close().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
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
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut last = -1;
    let mut since = Instant::now();
    loop {
        let now = newest_id(data);
        if now != last {
            last = now;
            since = Instant::now();
        } else if since.elapsed() >= quiet && reconciles_balanced(data) {
            return;
        }
        assert!(Instant::now() < deadline, "the log never settled");
        std::thread::sleep(Duration::from_millis(250));
    }
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

pub fn raw_kind(action: u32) -> Option<RawKind> {
    Some(match action {
        1 => RawKind::Added,
        2 => RawKind::Removed,
        3 => RawKind::Modified,
        4 => RawKind::RenamedOld,
        5 => RawKind::RenamedNew,
        _ => return None,
    })
}

/// `path` relative to `root` (both without the `\\?\` prefix), as UTF-16.
pub fn relative(path: &[u16], root: &[u16]) -> Option<Vec<u16>> {
    let head = path.get(..root.len())?;
    let same = String::from_utf16_lossy(head).eq_ignore_ascii_case(&String::from_utf16_lossy(root));
    let rest = path.get(root.len()..)?;
    if same && rest.first() == Some(&0x5C) {
        rest.get(1..).map(<[u16]>::to_vec)
    } else {
        None
    }
}

pub struct Run {
    pub data: PathBuf,
    pub root: PathBuf,
}

/// What the comparator saw.
pub struct Verdict {
    pub report: peekbuddy_churn::compare::Report,
    pub gaps: usize,
    pub unfinished_gaps: usize,
    pub summary: String,
}

pub fn judge(run: &Run, outcome: &peekbuddy_churn::exec::Outcome) -> Verdict {
    let root_units: Vec<u16> = run
        .root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .encode_utf16()
        .collect();
    let rows = all_rows(&run.data);

    let raw: Vec<ChurnRow> = rows
        .iter()
        .filter(|row| row.source == Source::Rdcw)
        .filter_map(|row| {
            Some(ChurnRow {
                path: relative(&row.path, &root_units)?,
                kind: raw_kind(row.raw_mask)?,
                time: row.ts,
            })
        })
        .collect();

    // A gap covers the whole root until the reconcile that follows it ends.
    let gap_rows = rows.iter().filter(|row| row.kind.is_gap()).count();
    let gaps: Vec<Gap> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.kind.is_gap())
        .filter_map(|(at, _)| {
            rows.iter()
                .skip(at)
                .find(|row| row.kind == Kind::ReconcileFinished)
                .map(|finish| Gap {
                    subtree: Vec::new(),
                    reconciled: finish.ts,
                })
        })
        .collect();

    let report = compare(&outcome.records, &raw, &gaps);
    let mut kinds: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for row in rows.iter().filter(|row| row.source != Source::Rdcw) {
        *kinds.entry(row.kind.name()).or_default() += 1;
    }
    let summary = format!(
        "ops {} matched {} covered by a gap {} undetected {} gap rows {gap_rows} rows {}; other rows {kinds:?}",
        outcome.records.len(),
        report.matched,
        report.covered_by_gap,
        report.undetected.len(),
        rows.len()
    );
    let detail: Vec<String> = report
        .undetected
        .iter()
        .take(12)
        .filter_map(|seq| outcome.records.iter().find(|r| r.seq == *seq))
        .map(|r| {
            let wanted: Vec<String> = r
                .expected_records()
                .unwrap_or_default()
                .iter()
                .map(|e| {
                    let seen = raw
                        .iter()
                        .any(|row| row.path == e.path && row.kind == e.kind);
                    format!(
                        "{:?} {} seen={seen}",
                        e.kind,
                        String::from_utf16_lossy(&e.path)
                    )
                })
                .collect();
            format!("#{} {:?} [{}]", r.seq, r.op, wanted.join("; "))
        })
        .collect();
    Verdict {
        report,
        gaps: gap_rows,
        unfinished_gaps: gap_rows - gaps.len(),
        summary: format!("{summary}\n{}", detail.join("\n")),
    }
}
