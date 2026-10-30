//! The event store inside the engine (#52): today's segment, the writer
//! pipeline, the roots of the settings, and the counters and probe numbers fed
//! from each commit.
//!
//! Day rotation and retention are #54; this opens the segment of the current
//! UTC day. Capture (#45) will push batches through [`Store::producer`].

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use peekbuddy_core::event::RootIdx;
use peekbuddy_core::perf::{CounterSnapshot, Counters};
use peekbuddy_core::settings::Settings;
use peekbuddy_store::pipeline::{CommitInfo, Config, Hooks, Pipeline, Producer};
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
        config.hooks = Hooks {
            before_commit: None,
            on_commit: Some(Arc::new(move |info: &CommitInfo| {
                on_commit(&observer_counters, &observer_commits, info);
            })),
        };
        let pipeline = Pipeline::start(writer, config)?;
        let producer = pipeline.producer();
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
