//! The event store inside the engine (#52): today's segment, the writer
//! pipeline, the roots of the settings, and the counters and probe numbers fed
//! from each commit.
//!
//! Day rotation and retention are #54; this opens the segment of the current
//! UTC day. Capture (#45) will push batches through [`Store::producer`].

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use peekbuddy_core::event::{
    Event, Flags, Kind, NameKind, NameRef, RootIdx, STORE_WAL_BLOCKED, STORE_WRITE_FAILING, Source,
    VolIdx,
};
use peekbuddy_core::perf::{CounterSnapshot, Counters};
use peekbuddy_core::settings::Settings;
use peekbuddy_store::pipeline::{CommitInfo, Config, Health, Hooks, Pipeline, Producer};
use peekbuddy_store::retention::{Caps, NoViewer};
use peekbuddy_store::segment::{DEFAULT_SIZE_CAP, DayKey, Rotation};
use peekbuddy_store::writer::{RootInfo, SegmentsConfig, VolInfo, Writer};
use peekbuddy_win::io::dir::DirHandle;
use peekbuddy_win::io::open::OpenOptions;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::io::volume::volume_identity;
use peekbuddy_win::perf::{etw, since_process_create_us};

use crate::datadir::DataDir;
use crate::probe::{BatchCommit, CommitLog};

/// How long quitting waits for the queue to reach the disk (F19).
pub const FLUSH_DEADLINE: Duration = Duration::from_secs(5);

/// The running store: the pipeline while it runs, the writer after it was
/// stopped for the final checkpoint.
pub struct Store {
    pipeline: Option<Pipeline>,
    writer: Option<Writer>,
    producer: Producer,
    /// The segment file.
    pub path: PathBuf,
    /// Roots that could be registered (the others were unreachable).
    pub roots: usize,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("roots", &self.roots)
            .finish_non_exhaustive()
    }
}

/// The retention caps of the settings.
fn caps_of(settings: &Settings) -> Caps {
    Caps {
        max_age_days: settings.store.max_age_days,
        max_total_bytes: settings
            .store
            .max_total_mb
            .map(|mb| mb.saturating_mul(1024 * 1024)),
        max_rows: settings.store.max_rows,
    }
}

/// Now as FILETIME ticks.
fn now_filetime() -> u64 {
    crate::pause_ctl::now_ticks().0
}

fn volume_of(path: &str) -> Option<(VolInfo, Vec<u16>)> {
    let wide: Vec<u16> = path.encode_utf16().collect();
    let long = to_long_path(&wide).ok()?;
    let dir = DirHandle::open(&long, OpenOptions::default()).ok()?;
    let identity = volume_identity(&dir, &long).ok()?;
    Some((
        VolInfo {
            guid: String::from_utf16_lossy(&identity.guid_path),
            serial: i64::from(identity.serial),
            letter: None,
            journal_id: None,
        },
        wide,
    ))
}

impl Store {
    /// Opens today's segment, registers the watches and starts the
    /// writer. Every commit updates `counters`, appends to `commits` and
    /// writes the ETW counter event (never from a timer, F6).
    ///
    /// # Errors
    ///
    /// The error from opening the segment or starting the threads. A watch
    /// that cannot be opened is skipped (the lifecycle code of #48 deals
    /// with lost roots), not an error.
    pub fn start(
        data: &DataDir,
        settings: &Settings,
        counters: &Arc<Counters>,
        commits: &Arc<Mutex<CommitLog>>,
    ) -> std::io::Result<Self> {
        let (mut writer, skipped) = Writer::open_segments(
            SegmentsConfig {
                rotation: Rotation {
                    dir: data.segments.clone(),
                    day_of: Arc::new(|ts| {
                        let (year, month, day) = peekbuddy_win::localtime::local_day(ts);
                        DayKey { year, month, day }
                    }),
                    size_cap: DEFAULT_SIZE_CAP,
                },
                caps: caps_of(settings),
                retire: Arc::new(NoViewer),
            },
            now_filetime(),
        )
        .map_err(std::io::Error::other)?;
        if !skipped.is_empty() {
            // A health event: the files stay where they are, untouched.
            let mut text = String::new();
            for file in &skipped {
                let _ = writeln!(text, "{} skipped: {}", file.file, file.reason);
            }
            let _ = std::fs::write(data.logs.join("segments.log"), text);
        }
        let path = data
            .segments
            .join(writer.segment_file().unwrap_or("events.db"));
        let mut roots = 0;
        // Paused watches are registered too: pausing and resuming write
        // state rows for them (#63).
        for (index, watch) in settings.watches.iter().enumerate() {
            let Some((vol, units)) = volume_of(&watch.path) else {
                continue;
            };
            let registered = writer.register_root(&RootInfo {
                idx: RootIdx(u16::try_from(index).unwrap_or(u16::MAX)),
                vol,
                path: units,
                recursive: watch.recursive,
            });
            roots += usize::from(registered.is_ok());
        }
        let (observer_counters, observer_commits) = (Arc::clone(counters), Arc::clone(commits));
        let mut config = Config::new(data.root.join("spill"));
        let health_rows: Arc<OnceLock<Producer>> = Arc::new(OnceLock::new());
        let for_health = Arc::clone(&health_rows);
        config.hooks = Hooks {
            before_commit: None,
            on_commit: Some(Arc::new(move |info: &CommitInfo| {
                on_commit(&observer_counters, &observer_commits, info);
            })),
            before_attempt: None,
            on_health: Some(Arc::new(move |health| {
                if let Some(producer) = for_health.get() {
                    let mut batch = producer.new_batch();
                    if batch
                        .push(health_event(health, crate::pause_ctl::now_ticks()), &[])
                        .is_ok()
                    {
                        let _ = producer.push(batch);
                    }
                }
            })),
        };
        let pipeline = Pipeline::start(writer, config)?;
        let producer = pipeline.producer();
        let _ = health_rows.set(producer.clone());
        Ok(Self {
            pipeline: Some(pipeline),
            writer: None,
            producer,
            path,
            roots,
        })
    }

    /// The handle capture pushes batches through.
    #[must_use]
    pub fn producer(&self) -> Producer {
        self.producer.clone()
    }

    /// Batches lost because even the spill could not be written.
    #[must_use]
    pub fn lost_batches(&self) -> u64 {
        self.pipeline
            .as_ref()
            .map_or(0, |pipeline| pipeline.stats().lost_batches)
    }

    /// Waits for everything pushed to reach the database, at most
    /// [`FLUSH_DEADLINE`]. Returns whether it did.
    #[must_use]
    pub fn flush(&self) -> bool {
        self.pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.flush(FLUSH_DEADLINE).is_ok())
    }

    /// Stops accepting batches and the writer thread; the connection is kept
    /// for [`Store::checkpoint`].
    pub fn close(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            self.writer = pipeline.close();
        }
        if let Some(writer) = self.writer.as_mut() {
            // Sealing threads of earlier rotations, and the purge rows they
            // queued after the last commit.
            writer.wait_background();
            let _ = writer.flush_system_rows();
        }
    }

    /// Final checkpoint: truncates the write-ahead log.
    pub fn checkpoint(&self) {
        if let Some(writer) = &self.writer {
            let _ = writer.checkpoint_truncate();
        }
    }

    /// Closes the database.
    pub fn release(&mut self) {
        self.writer = None;
    }
}

/// The row that tells the log the store became unwell or well again. The
/// writer thread pushes it like any other row, so while the writer cannot
/// commit it waits in the ring or the spill file and is written with the rest.
#[must_use]
pub fn health_event(health: Health, (ts, qpc): (u64, u64)) -> Event {
    let (kind, cause) = match health {
        Health::WalBlocked => (Kind::StoreDegraded, STORE_WAL_BLOCKED),
        Health::WalReleased => (Kind::StoreRecovered, STORE_WAL_BLOCKED),
        Health::Failing => (Kind::StoreDegraded, STORE_WRITE_FAILING),
        Health::Recovered => (Kind::StoreRecovered, STORE_WRITE_FAILING),
    };
    Event {
        ts,
        qpc,
        src_ts: 0,
        source: Source::System,
        kind,
        raw_mask: cause,
        source_info: 0,
        is_dir: false,
        size: 0,
        attrs: 0,
        reparse_tag: 0,
        file_id: 0,
        parent_file_id: 0,
        usn: 0,
        vol: VolIdx(0),
        root: RootIdx::NONE,
        name: NameRef::EMPTY,
        name_kind: NameKind::Unknown,
        flags: Flags::NONE,
    }
}

fn on_commit(counters: &Counters, commits: &Mutex<CommitLog>, info: &CommitInfo) {
    use std::sync::atomic::Ordering;

    counters
        .rows_persisted
        .fetch_add(info.commit.rows, Ordering::Relaxed);
    counters
        .last_batch_size
        .store(info.commit.rows, Ordering::Relaxed);
    counters.observe_ring_depth(u64::try_from(info.ring_high_water).unwrap_or(u64::MAX));
    if let Ok(mut log) = commits.lock() {
        log.push(BatchCommit {
            at_us: since_process_create_us(),
            rows: info.commit.rows,
        });
    }
    let snapshot: CounterSnapshot = counters.snapshot();
    etw::counters(&snapshot);
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::event::{Kind, STORE_WAL_BLOCKED, STORE_WRITE_FAILING, Source};
    use peekbuddy_store::pipeline::Health;

    use std::sync::{Arc, Mutex};

    use peekbuddy_core::perf::Counters;
    use peekbuddy_core::settings::{Settings, Store as StoreSettings, Watch};

    use super::{Store, caps_of, health_event, now_filetime, volume_of};
    use crate::datadir::DataDir;
    use crate::portable::Decision;
    use crate::probe::CommitLog;

    #[test]
    fn each_health_change_is_a_system_row_with_its_cause() {
        let table = [
            (Health::WalBlocked, Kind::StoreDegraded, STORE_WAL_BLOCKED),
            (Health::WalReleased, Kind::StoreRecovered, STORE_WAL_BLOCKED),
            (Health::Failing, Kind::StoreDegraded, STORE_WRITE_FAILING),
            (Health::Recovered, Kind::StoreRecovered, STORE_WRITE_FAILING),
        ];
        for (health, kind, cause) in table {
            let event = health_event(health, (7, 9));
            assert_eq!((event.kind, event.raw_mask), (kind, cause), "{health:?}");
            assert_eq!((event.ts, event.qpc), (7, 9));
            assert_eq!(event.source, Source::System);
        }
    }

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("pb-store-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_caps_follow_the_settings_and_megabytes_are_mebibytes() {
        let mut settings = Settings {
            store: StoreSettings {
                max_age_days: Some(11),
                max_total_mb: Some(3),
                max_rows: Some(500),
            },
            ..Settings::default()
        };
        let caps = caps_of(&settings);
        assert_eq!(caps.max_age_days, Some(11));
        assert_eq!(caps.max_total_bytes, Some(3 * 1024 * 1024));
        assert_eq!(caps.max_rows, Some(500));
        settings.store.max_total_mb = None;
        assert_eq!(caps_of(&settings).max_total_bytes, None);
    }

    #[test]
    fn now_is_a_filetime_of_this_decade() {
        // 2020-01-01 as FILETIME ticks.
        assert!(now_filetime() > 132_223_104_000_000_000);
    }

    #[test]
    fn a_volume_is_found_for_a_directory_and_not_for_a_missing_one() {
        let dir = Dir::new("volume");
        let text = dir.0.to_string_lossy().into_owned();
        let (vol, wide) = volume_of(&text).unwrap();
        assert_eq!(String::from_utf16_lossy(&wide), text);
        assert_ne!(vol.guid, "");
        assert!(volume_of(&format!(r"{text}\missing")).is_none());
    }

    #[test]
    fn a_store_runs_flushes_closes_and_releases_its_file() {
        let dir = Dir::new("lifecycle");
        let data = DataDir::open(&dir.0.join("data"), Decision::Portable).unwrap();
        let settings = Settings {
            watches: vec![
                Watch {
                    id: "w0".to_owned(),
                    path: dir.0.to_string_lossy().into_owned(),
                    ..Watch::default()
                },
                Watch {
                    id: "w1".to_owned(),
                    path: dir.0.join("missing").to_string_lossy().into_owned(),
                    ..Watch::default()
                },
            ],
            ..Settings::default()
        };
        let counters = Arc::new(Counters::new());
        let commits = Arc::new(Mutex::new(CommitLog::new()));

        let mut store = Store::start(&data, &settings, &counters, &commits).unwrap();

        assert_eq!(store.roots, 1);
        assert_eq!(store.lost_batches(), 0);
        let producer = store.producer();
        let mut batch = producer.new_batch();
        batch
            .push(health_event(Health::Failing, (now_filetime(), 1)), &[])
            .unwrap();
        let _ = producer.push(batch);
        assert!(store.flush());
        assert!(
            counters
                .rows_persisted
                .load(std::sync::atomic::Ordering::Relaxed)
                >= 1
        );
        store.close();
        assert!(!store.flush());
        store.checkpoint();
        let wal = std::path::PathBuf::from(format!("{}-wal", store.path.display()));
        assert_eq!(std::fs::metadata(&wal).map_or(0, |meta| meta.len()), 0);
        store.release();
        std::fs::remove_file(&store.path).unwrap();
    }
}
