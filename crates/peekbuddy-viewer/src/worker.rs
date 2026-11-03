//! The query worker (#66): every database read of the viewer happens here.
//!
//! The UI thread never touches SQLite. It sends [`Job`]s and reads
//! [`Reply`]s; the worker wakes it (`request_repaint`) only when it has
//! something to show, so an idle viewer paints nothing.
//!
//! # Generations
//!
//! A new filter, a new seek target or a new window bumps the *generation*.
//! Every job and every reply carries the generation it belongs to; the UI
//! drops replies of an older one. Page jobs also carry a [`CancelFlag`], which
//! the UI sets for the jobs it no longer wants, so a long scan stops at its
//! next step instead of running to the end.
//!
//! # Scans
//!
//! A page of a filtered view may take a scan that the reader cuts into steps
//! of a few milliseconds ([`peekbuddy_core::query::SCAN_BUDGET`]). The worker
//! asks again from the `resume` cursor until the page is full or the scan
//! ends, checking the cancel flag between steps and reporting progress at
//! most [`PROGRESS_EVERY`].
//!
//! # The live tail
//!
//! The engine does not push committed rows to the viewer yet, so the worker
//! follows the log itself: while a tail is watched it asks for the rows newer
//! than the newest it has delivered, every [`FAST`] while rows keep coming
//! and backing off to [`SLOW`] when nothing happens. A quiet log costs one
//! small keyset query per second and no repaint.

use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use peekbuddy_core::query::{CancelFlag, Direction, Filter, PAGE_SIZE, PageRequest};

use crate::rows::{GridRow, OffsetFn};
use crate::source::PageSource;
use crate::timeline::{Extent, Fetch, Fetched};

/// How often the tail polls while rows keep arriving.
pub const FAST: Duration = Duration::from_millis(250);
/// How often it polls when nothing happens.
pub const SLOW: Duration = Duration::from_secs(1);
/// The most often progress of a scan is reported.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// Work for the worker.
#[derive(Debug)]
pub enum Job {
    /// One page.
    Page {
        /// The generation it belongs to.
        generation: u64,
        /// What to fetch.
        fetch: Fetch,
        /// The filter.
        filter: Filter,
        /// Set to stop a scan.
        cancel: CancelFlag,
    },
    /// The extent of the log.
    Extent {
        /// The generation it belongs to.
        generation: u64,
    },
    /// How many rows the filter hides.
    Hidden {
        /// The generation it belongs to.
        generation: u64,
        /// The filter.
        filter: Filter,
        /// Set to stop the count.
        cancel: CancelFlag,
    },
    /// Follow the log from `cursor` on (`None`: from the start of an empty
    /// log). Replaces any tail watched before.
    WatchTail {
        /// The generation it belongs to.
        generation: u64,
        /// The filter.
        filter: Filter,
        /// The newest id delivered so far.
        cursor: Option<i64>,
    },
    /// Stop following.
    StopTail,
}

/// What the worker found.
#[derive(Debug)]
pub enum Reply {
    /// The answer to a [`Job::Page`].
    Page {
        /// The generation of the job.
        generation: u64,
        /// What was asked.
        fetch: Fetch,
        /// The rows, or why not.
        result: Result<Fetched<GridRow>, String>,
    },
    /// The answer to a [`Job::Extent`].
    Extent {
        /// The generation of the job.
        generation: u64,
        /// The extent, or why not.
        result: Result<Option<Extent>, String>,
    },
    /// The answer to a [`Job::Hidden`]; `None` when cancelled.
    Hidden {
        /// The generation of the job.
        generation: u64,
        /// The count.
        count: Option<u64>,
    },
    /// How far a scan has come.
    Progress {
        /// The generation of the job.
        generation: u64,
        /// Rows looked at so far.
        scanned: u64,
    },
    /// Rows the tail found.
    Tail {
        /// The generation of the watch.
        generation: u64,
        /// The rows, newest first.
        fetched: Fetched<GridRow>,
    },
}

/// The UI's end of the worker.
#[derive(Debug)]
pub struct Worker {
    jobs: Sender<Job>,
    replies: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts the worker. `wake` runs after every reply.
    ///
    /// # Errors
    ///
    /// The error from spawning the thread.
    pub fn start(
        source: Arc<dyn PageSource>,
        offset: Arc<OffsetFn>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<Self> {
        let (jobs, inbox) = mpsc::channel();
        let (outbox, replies) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(String::from("query-worker"))
            .spawn(move || {
                Runner {
                    source,
                    offset,
                    wake,
                    outbox,
                    tail: None,
                    interval: FAST,
                }
                .run(&inbox);
            })?;
        Ok(Self {
            jobs,
            replies,
            thread: Some(thread),
        })
    }

    /// Queues a job.
    pub fn send(&self, job: Job) {
        // A worker that is gone cannot answer; the view shows its status.
        let _ = self.jobs.send(job);
    }

    /// The next reply, if one is waiting.
    #[must_use]
    pub fn poll(&self) -> Option<Reply> {
        self.replies.try_recv().ok()
    }

    /// Ends the thread and waits for it.
    pub fn shutdown(mut self) {
        let thread = self.thread.take();
        drop(self);
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

struct Watch {
    generation: u64,
    filter: Filter,
    cursor: Option<i64>,
}

struct Runner {
    source: Arc<dyn PageSource>,
    offset: Arc<OffsetFn>,
    wake: Arc<dyn Fn() + Send + Sync>,
    outbox: Sender<Reply>,
    tail: Option<Watch>,
    interval: Duration,
}

impl Runner {
    fn reply(&self, reply: Reply) {
        if self.outbox.send(reply).is_ok() {
            (self.wake)();
        }
    }

    fn run(mut self, inbox: &Receiver<Job>) {
        loop {
            let next = if self.tail.is_some() {
                inbox.recv_timeout(self.interval)
            } else {
                inbox.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match next {
                Ok(job) => self.job(job),
                Err(RecvTimeoutError::Timeout) => self.poll_tail(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn job(&mut self, job: Job) {
        match job {
            Job::Page {
                generation,
                fetch,
                filter,
                cancel,
            } => {
                if cancel.is_cancelled() {
                    return;
                }
                if let Some(result) = self.page(generation, fetch, &filter, &cancel) {
                    self.reply(Reply::Page {
                        generation,
                        fetch,
                        result,
                    });
                }
            }
            Job::Extent { generation } => {
                let result = self.source.extent();
                self.reply(Reply::Extent { generation, result });
            }
            Job::Hidden {
                generation,
                filter,
                cancel,
            } => {
                let count = self.source.hidden(&filter, &cancel).ok().flatten();
                self.reply(Reply::Hidden { generation, count });
            }
            Job::WatchTail {
                generation,
                filter,
                cursor,
            } => {
                self.interval = FAST;
                self.tail = Some(Watch {
                    generation,
                    filter,
                    cursor,
                });
            }
            Job::StopTail => self.tail = None,
        }
    }

    /// Reads one page, in steps. `None` when the job was cancelled.
    fn page(
        &self,
        generation: u64,
        fetch: Fetch,
        filter: &Filter,
        cancel: &CancelFlag,
    ) -> Option<Result<Fetched<GridRow>, String>> {
        let (mut cursor, direction) = match fetch {
            Fetch::Older { cursor } => (Some(cursor), Direction::Older),
            Fetch::Newer { cursor } => (Some(cursor), Direction::Newer),
            Fetch::Seek { cursor, .. } => (cursor, Direction::Older),
        };
        let mut rows: Vec<GridRow> = Vec::new();
        let mut scanned = 0_u64;
        let mut reported = Instant::now();
        loop {
            let request = PageRequest {
                generation,
                filter: filter.clone(),
                cursor,
                direction,
                limit: PAGE_SIZE.saturating_sub(rows.len()).max(1),
            };
            let page = match self.source.page(&request, cancel) {
                Ok(page) => page,
                Err(error) => return Some(Err(error)),
            };
            if page.cancelled || cancel.is_cancelled() {
                return None;
            }
            scanned = scanned.saturating_add(page.scanned);
            let block = page
                .rows
                .into_iter()
                .map(|row| GridRow::new(row, &*self.offset));
            match direction {
                // Older blocks follow what is held.
                Direction::Older => rows.extend(block),
                // Newer blocks come in front of it.
                Direction::Newer => {
                    let mut joined: Vec<GridRow> = block.collect();
                    joined.append(&mut rows);
                    rows = joined;
                }
            }
            match page.resume {
                Some(next) if rows.len() < PAGE_SIZE => {
                    cursor = Some(next);
                    if reported.elapsed() >= PROGRESS_EVERY {
                        reported = Instant::now();
                        self.reply(Reply::Progress {
                            generation,
                            scanned,
                        });
                    }
                }
                _ => {
                    return Some(Ok(Fetched {
                        rows,
                        more: page.more,
                    }));
                }
            }
        }
    }

    fn poll_tail(&mut self) {
        let Some(watch) = &self.tail else { return };
        let (generation, cursor) = (watch.generation, watch.cursor);
        let cancel = CancelFlag::new();
        let fetch = cursor.map_or(
            Fetch::Seek {
                index: 0,
                cursor: None,
            },
            |cursor| Fetch::Newer { cursor },
        );
        let Some(Ok(fetched)) = self.page(generation, fetch, &watch.filter.clone(), &cancel) else {
            // A read that failed is tried again at the slow pace.
            self.interval = SLOW;
            return;
        };
        if fetched.rows.is_empty() {
            self.interval = (self.interval * 2).min(SLOW);
            return;
        }
        let newest = fetched.rows.first().map(|row| row.id);
        let more = fetched.more;
        if let Some(watch) = &mut self.tail {
            watch.cursor = newest.or(watch.cursor);
        }
        self.interval = if more { Duration::ZERO } else { FAST };
        self.reply(Reply::Tail {
            generation,
            fetched,
        });
    }
}

#[cfg(test)]
#[expect(clippy::panic, reason = "a test panics on an unexpected reply")]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use peekbuddy_core::query::{CancelFlag, Filter, PAGE_SIZE};

    use super::{Job, Reply, Worker};
    use crate::source::fake::FakeLog;
    use crate::timeline::Fetch;

    fn start(log: &Arc<FakeLog>) -> (Worker, Arc<AtomicU32>) {
        let wakes = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&wakes);
        let worker = Worker::start(
            Arc::clone(log) as Arc<dyn crate::source::PageSource>,
            Arc::new(|_| 0),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        (worker, wakes)
    }

    fn next(worker: &Worker) -> Reply {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(reply) = worker.poll() {
                return reply;
            }
            assert!(Instant::now() < deadline, "no reply within 10 s");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn page(worker: &Worker, generation: u64, fetch: Fetch) {
        worker.send(Job::Page {
            generation,
            fetch,
            filter: Filter::default(),
            cancel: CancelFlag::new(),
        });
    }

    fn ids(reply: Reply) -> Vec<i64> {
        match reply {
            Reply::Page { result, .. } => result.unwrap().rows.iter().map(|r| r.id).collect(),
            Reply::Tail { fetched, .. } => fetched.rows.iter().map(|r| r.id).collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn no_query_runs_on_the_calling_thread() {
        let log = FakeLog::new(1_000);
        let (worker, wakes) = start(&log);
        let me = std::thread::current().id();
        page(
            &worker,
            1,
            Fetch::Seek {
                index: 0,
                cursor: None,
            },
        );
        worker.send(Job::Extent { generation: 1 });
        worker.send(Job::Hidden {
            generation: 1,
            filter: Filter::default_view(),
            cancel: CancelFlag::new(),
        });
        for _ in 0..3 {
            next(&worker);
        }
        let threads = log.threads();
        assert_ne!(threads.len(), 0);
        assert!(threads.iter().all(|thread| *thread != me), "{threads:?}");
        assert_eq!(wakes.load(Ordering::SeqCst), 3, "each reply woke the UI");
        worker.shutdown();
    }

    #[test]
    fn a_page_comes_back_in_display_order_with_its_fetch_and_generation() {
        let log = FakeLog::new(1_000);
        let (worker, _) = start(&log);
        let fetch = Fetch::Seek {
            index: 0,
            cursor: None,
        };
        page(&worker, 7, fetch);
        match next(&worker) {
            Reply::Page {
                generation,
                fetch: got,
                result,
            } => {
                assert_eq!((generation, got), (7, fetch));
                let fetched = result.unwrap();
                assert_eq!(fetched.rows.len(), PAGE_SIZE);
                assert_eq!(fetched.rows.first().map(|r| r.id), Some(1_000));
                assert_eq!(fetched.rows.last().map(|r| r.id), Some(801));
                assert!(fetched.more);
                // The text is ready: nothing is formatted on the UI thread.
                assert_eq!(fetched.rows[0].file_name(), "f1000.txt");
            }
            other => panic!("{other:?}"),
        }
        worker.send(Job::Extent { generation: 7 });
        match next(&worker) {
            Reply::Extent { result, .. } => {
                let extent = result.unwrap().unwrap();
                assert_eq!((extent.newest, extent.oldest), (1_000, 1));
            }
            other => panic!("{other:?}"),
        }
        worker.shutdown();
    }

    #[test]
    fn a_scan_in_steps_fills_the_page_and_keeps_the_order() {
        let log = FakeLog::new(1_000);
        log.step(30);
        let (worker, _) = start(&log);
        // Older: blocks of 30 follow one another.
        page(&worker, 1, Fetch::Older { cursor: 500 });
        let older = ids(next(&worker));
        assert_eq!(older.len(), PAGE_SIZE);
        assert_eq!(older.first(), Some(&499));
        assert!(older.windows(2).all(|pair| pair[0] - pair[1] == 1));
        // Newer: blocks of 30 come in front, nearest the cursor last.
        page(&worker, 1, Fetch::Newer { cursor: 100 });
        let newer = ids(next(&worker));
        assert_eq!(newer.len(), PAGE_SIZE);
        assert_eq!(newer.first(), Some(&300));
        assert_eq!(newer.last(), Some(&101));
        assert!(newer.windows(2).all(|pair| pair[0] - pair[1] == 1));
        assert!(log.calls() >= 2 * (PAGE_SIZE / 30), "{}", log.calls());
        worker.shutdown();
    }

    #[test]
    fn a_cancelled_job_is_not_run_and_not_answered() {
        let log = FakeLog::new(1_000);
        let (worker, _) = start(&log);
        let cancel = CancelFlag::new();
        cancel.cancel();
        worker.send(Job::Page {
            generation: 1,
            fetch: Fetch::Older { cursor: 500 },
            filter: Filter::default(),
            cancel,
        });
        // A later job proves the worker went on, and that nothing came before.
        worker.send(Job::Extent { generation: 2 });
        assert!(matches!(next(&worker), Reply::Extent { generation: 2, .. }));
        assert_eq!(log.calls(), 0);
        worker.shutdown();
    }

    #[test]
    fn the_tail_delivers_new_rows_and_wakes_the_ui_only_for_them() {
        let log = FakeLog::new(100);
        let (worker, wakes) = start(&log);
        worker.send(Job::WatchTail {
            generation: 3,
            filter: Filter::default(),
            cursor: Some(100),
        });
        // Nothing new: no reply, no wakeup, however long it polls.
        std::thread::sleep(Duration::from_millis(600));
        assert!(worker.poll().is_none());
        assert_eq!(wakes.load(Ordering::SeqCst), 0);

        log.grow(5);
        match next(&worker) {
            Reply::Tail {
                generation,
                fetched,
            } => {
                assert_eq!(generation, 3);
                let got: Vec<i64> = fetched.rows.iter().map(|r| r.id).collect();
                assert_eq!(got, vec![105, 104, 103, 102, 101]);
                assert!(!fetched.more);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        // The next poll starts after the rows it delivered.
        log.grow(2);
        assert_eq!(ids(next(&worker)), vec![107, 106]);
        worker.send(Job::StopTail);
        log.grow(1);
        std::thread::sleep(Duration::from_millis(700));
        assert!(worker.poll().is_none(), "a stopped tail does not poll");
        worker.shutdown();
    }

    #[test]
    fn a_tail_that_was_reset_starts_again_from_the_given_cursor() {
        let log = FakeLog::new(100);
        let (worker, _) = start(&log);
        worker.send(Job::WatchTail {
            generation: 1,
            filter: Filter::default(),
            cursor: Some(100),
        });
        log.grow(3);
        assert_eq!(ids(next(&worker)), vec![103, 102, 101]);
        // The UI refused the batch (a hole) and asks again from 100.
        worker.send(Job::WatchTail {
            generation: 1,
            filter: Filter::default(),
            cursor: Some(100),
        });
        assert_eq!(ids(next(&worker)), vec![103, 102, 101]);
        worker.shutdown();
    }

    #[test]
    fn an_empty_log_is_followed_from_its_start() {
        let log = FakeLog::new(0);
        let (worker, _) = start(&log);
        worker.send(Job::WatchTail {
            generation: 1,
            filter: Filter::default(),
            cursor: None,
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(worker.poll().is_none());
        log.grow(2);
        assert_eq!(ids(next(&worker)), vec![2, 1]);
        worker.shutdown();
    }

    #[test]
    fn a_burst_is_drained_without_waiting_for_the_next_tick() {
        let log = FakeLog::new(10);
        let (worker, _) = start(&log);
        worker.send(Job::WatchTail {
            generation: 1,
            filter: Filter::default(),
            cursor: Some(10),
        });
        log.grow(i64::try_from(3 * PAGE_SIZE).unwrap());
        let started = Instant::now();
        let mut total = 0;
        while total < 3 * PAGE_SIZE {
            total += ids(next(&worker)).len();
        }
        // Three full pages at the idle pace would take 750 ms.
        assert!(
            started.elapsed() < Duration::from_millis(700),
            "{:?}",
            started.elapsed()
        );
        worker.shutdown();
    }
}
