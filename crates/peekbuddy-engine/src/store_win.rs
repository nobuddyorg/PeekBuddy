//! The event store inside the engine (#52): today's segment, the writer
//! pipeline, the roots of the settings, and the counters and probe numbers fed
//! from each commit.
//!
//! Day rotation and retention are #54; this opens the segment of the current
//! UTC day. Capture (#45) will push batches through [`Store::producer`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use peekbuddy_core::event::RootIdx;
use peekbuddy_core::perf::{CounterSnapshot, Counters};
use peekbuddy_core::settings::Settings;
use peekbuddy_store::pipeline::{CommitInfo, Config, Hooks, Pipeline, Producer};
use peekbuddy_store::writer::{RootInfo, VolInfo, Writer};
use peekbuddy_win::io::dir::DirHandle;
use peekbuddy_win::io::open::OpenOptions;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::io::volume::volume_identity;
use peekbuddy_win::perf::{etw, since_process_create_us};

use crate::datadir::DataDir;
use crate::probe::{BatchCommit, CommitLog};

/// How long quitting waits for the queue to reach the disk (F19).
pub const FLUSH_DEADLINE: Duration = Duration::from_secs(5);

/// The running store.
pub struct Store {
    pipeline: Pipeline,
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

/// `segments\YYYYMMDD.db` for the current UTC day.
#[must_use]
pub fn segment_path(data: &DataDir, now: SystemTime) -> PathBuf {
    let stamp = crate::crash::utc_stamp(now);
    data.segments.join(format!("{}.db", stamp.get(..8).unwrap_or("00000000")))
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
    /// Opens today's segment, registers the enabled watches and starts the
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
        let path = segment_path(data, SystemTime::now());
        let mut writer = Writer::open(&path).map_err(std::io::Error::other)?;
        let mut roots = 0;
        for (index, watch) in settings.watches.iter().enumerate().filter(|(_, w)| w.enabled) {
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
        config.hooks = Hooks {
            before_commit: None,
            on_commit: Some(Arc::new(move |info: &CommitInfo| {
                on_commit(&observer_counters, &observer_commits, info);
            })),
        };
        let pipeline = Pipeline::start(writer, config)?;
        Ok(Self { pipeline, path, roots })
    }

    /// The handle capture pushes batches through.
    #[must_use]
    pub fn producer(&self) -> Producer {
        self.pipeline.producer()
    }

    /// Batches lost because even the spill could not be written.
    #[must_use]
    pub fn lost_batches(&self) -> u64 {
        self.pipeline.stats().lost_batches
    }

    /// Waits for everything pushed to reach the database, at most
    /// [`FLUSH_DEADLINE`].
    pub fn flush(&self) -> bool {
        self.pipeline.flush(FLUSH_DEADLINE).is_ok()
    }

    /// Stops the writer and returns the connection for the final checkpoint.
    #[must_use]
    pub fn close(self) -> Option<Writer> {
        self.pipeline.close()
    }
}

fn on_commit(counters: &Counters, commits: &Mutex<CommitLog>, info: &CommitInfo) {
    use std::sync::atomic::Ordering;

    counters.rows_persisted.fetch_add(info.commit.rows, Ordering::Relaxed);
    counters.last_batch_size.store(info.commit.rows, Ordering::Relaxed);
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
