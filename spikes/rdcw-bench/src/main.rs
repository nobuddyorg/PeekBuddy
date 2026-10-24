//! Spike #32: ReadDirectoryChangesExW throughput and overflow behaviour.
//!
//! `rdcw-bench <experiment> [flags]`; one JSON line per cell on stdout.
//! Experiments:
//!   matrix    --dir D [--label L]       info classes x buffer sizes, overflow signal
//!   lifecycle --dir D                   root deleted/renamed/recreated, parent renamed
//!   detach    --dir D --flag-dir F      armed on a VHD; the orchestrator detaches it
//!   perf      --dir D --size S --rate R [--secs N --mode single|double --work-ns N --threads N]
//!   burst     --dir D --size S --ops N --stall-ms M
//!   pool      --dir D --size S --n N    kernel non-paged pool per armed handle
#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use peekbuddy_core::notify::{Class, parse};
use peekbuddy_win::io::dir::DirHandle;
use peekbuddy_win::io::iocp::{Batch, Iocp};
use peekbuddy_win::io::open::OpenOptions;
use peekbuddy_win::io::path::to_long_path;
use peekbuddy_win::io::slot::IoSlot;
use windows_sys::Win32::Storage::FileSystem::ReadDirectoryChangesExW;
use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::Threading::GetCurrentThread;
use windows_sys::Win32::System::WindowsProgramming::QueryThreadCycleTime;

const NOTIFY_ALL: u32 = 0x1FF;

struct Cfg {
    dir: PathBuf,
    label: String,
    size: usize,
    rate: u64,
    secs: u64,
    ops: u64,
    stall_ms: u64,
    mode: String,
    work_ns: u64,
    threads: usize,
    n: usize,
    flag_dir: PathBuf,
}

fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "'"))
}

fn emit(cfg: &Cfg, exp: &str, cell: &str, fields: &[(&str, String)]) {
    let mut line = format!(
        "{{\"spike\":\"rdcw\",\"experiment\":\"{exp}\",\"label\":{},\"cell\":{}",
        json_str(&cfg.label),
        json_str(cell)
    );
    for (k, v) in fields {
        line.push_str(&format!(",\"{k}\":{v}"));
    }
    println!("{line}}}");
}

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().collect()
}

fn open_dir(path: &Path) -> Result<DirHandle, String> {
    let long = to_long_path(&wide(path)).map_err(|e| format!("path: {e}"))?;
    DirHandle::open(&long, OpenOptions::default())
        .map_err(|e| format!("open {}: {e}", e.raw_os_error().unwrap_or(0)))
}

struct Watch {
    dir: DirHandle,
    slots: Vec<IoSlot>,
    class: i32,
}

fn class_of(info: i32) -> Class {
    match info {
        3 => Class::Full,
        2 => Class::Extended,
        _ => Class::Plain,
    }
}

impl Watch {
    fn new(
        path: &Path,
        size: usize,
        slots: usize,
        class: i32,
        iocp: &Iocp,
    ) -> Result<Self, String> {
        let dir = open_dir(path)?;
        iocp.associate(&dir, 1)
            .map_err(|e| format!("associate: {e}"))?;
        let mut v = Vec::new();
        for _ in 0..slots {
            v.push(IoSlot::new(size).map_err(|e| format!("slot: {e}"))?);
        }
        Ok(Self {
            dir,
            slots: v,
            class,
        })
    }

    /// Issues the read into slot `i`; the Win32 error on failure.
    fn arm(&mut self, i: usize, subtree: bool) -> Result<(), u32> {
        let slot = self.slots.get_mut(i).ok_or(1u32)?;
        let p = slot.begin().map_err(|_| 2u32)?;
        // SAFETY: the pointers come from `begin` and stay valid until the slot
        // is finished; the handle is open for overlapped directory listing.
        let ok = unsafe {
            ReadDirectoryChangesExW(
                self.dir.as_raw_handle(),
                p.buffer.cast::<c_void>(),
                p.len,
                i32::from(subtree),
                NOTIFY_ALL,
                std::ptr::null_mut(),
                p.overlapped,
                None,
                self.class,
            )
        };
        if ok == 0 {
            let e = std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(0)
                .cast_unsigned();
            let _ = slot.finish();
            return Err(e);
        }
        Ok(())
    }

    fn slot_of(&self, addr: usize) -> Option<usize> {
        self.slots.iter().position(|s| s.overlapped_addr() == addr)
    }
}

fn dequeue_one(iocp: &Iocp, timeout: Duration) -> Option<peekbuddy_win::io::iocp::Completion> {
    let mut batch = Batch::new(4);
    match iocp.dequeue(&mut batch, Some(timeout)) {
        Ok(n) if n > 0 => batch.iter().next(),
        _ => None,
    }
}

fn describe(
    c: &peekbuddy_win::io::iocp::Completion,
    watch: &Watch,
    size_class: Class,
) -> (String, usize, usize, Option<String>) {
    let status = format!("{:?}", c.status);
    let bytes = c.bytes as usize;
    let (mut records, mut errs) = (0, 0);
    let mut first_err = None;
    if bytes > 0
        && c.is_success()
        && let Some(i) = watch.slot_of(c.overlapped)
        && let Some(data) = watch.slots.get(i).and_then(|s| s.data(bytes))
    {
        for item in parse(data, bytes, size_class) {
            match item {
                Ok(_) => records += 1,
                Err(e) => {
                    errs += 1;
                    first_err.get_or_insert(format!("{:?}@{}", e.kind, e.offset));
                }
            }
        }
    }
    (status, bytes, records, first_err.filter(|_| errs > 0))
}

fn make_files(dir: &Path, count: usize, tag: &str) {
    let sub = dir.join(format!("flood-{tag}"));
    let _ = std::fs::create_dir_all(&sub);
    for i in 0..count {
        let _ = std::fs::write(sub.join(format!("f{i}.dat")), b"0123456789abcdef");
    }
}

/// Q3/Q4: which info classes and buffer sizes work, and how overflow is signalled.
fn matrix(cfg: &Cfg) {
    let root = cfg.dir.join(format!("matrix-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    for size in [64 * 1024_usize, 1024 * 1024] {
        for class in [3, 2, 1] {
            let cell = format!("size{}K-class{class}", size / 1024);
            let Ok(iocp) = Iocp::new(1) else { continue };
            let mut watch = match Watch::new(&root, size, 1, class, &iocp) {
                Ok(w) => w,
                Err(e) => {
                    emit(
                        cfg,
                        "matrix",
                        &cell,
                        &[("armed", "false".into()), ("error", json_str(&e))],
                    );
                    continue;
                }
            };
            if let Err(e) = watch.arm(0, true) {
                emit(
                    cfg,
                    "matrix",
                    &cell,
                    &[
                        ("armed", "false".into()),
                        ("error", json_str(&format!("win32 {e}"))),
                    ],
                );
                continue;
            }
            let _ = std::fs::write(root.join("probe.txt"), b"x");
            let first = dequeue_one(&iocp, Duration::from_secs(10));
            let (status, bytes, records, err) =
                first.as_ref().map_or(("timeout".into(), 0, 0, None), |c| {
                    describe(c, &watch, class_of(class))
                });
            let mut fields = vec![
                ("armed", "true".to_owned()),
                ("first_status", json_str(&status)),
                ("first_bytes", bytes.to_string()),
                ("first_records", records.to_string()),
                (
                    "first_parse_error",
                    err.map_or("null".into(), |e| json_str(&e)),
                ),
            ];
            // Overflow: re-arm, stall, flood past the buffer, then read.
            if let Some(c) = first {
                if let Some(i) = watch.slot_of(c.overlapped) {
                    let _ = watch.slots[i].finish();
                }
                if watch.arm(0, true).is_ok() {
                    let files = size / 40 + 2_000;
                    make_files(&root, files, &cell);
                    let second = dequeue_one(&iocp, Duration::from_secs(20));
                    let (status2, bytes2, records2, _) =
                        second.as_ref().map_or(("timeout".into(), 0, 0, None), |c| {
                            describe(c, &watch, class_of(class))
                        });
                    fields.push(("flood_files", files.to_string()));
                    fields.push(("flood_status", json_str(&status2)));
                    fields.push(("flood_bytes", bytes2.to_string()));
                    fields.push(("flood_records", records2.to_string()));
                    fields.push((
                        "overflow_signal",
                        json_str(if bytes2 == 0 && status2 == "Other(0)" {
                            "success_zero_bytes"
                        } else if status2.contains("NotifyEnumDir") {
                            "ERROR_NOTIFY_ENUM_DIR"
                        } else if bytes2 > 0 {
                            "records_no_overflow"
                        } else {
                            status2.as_str()
                        }),
                    ));
                }
            }
            emit(cfg, "matrix", &cell, &fields);
            let _ = std::fs::remove_dir_all(root.join(format!("flood-{cell}")));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Q5: what happens to the watch when the root or its parent changes.
fn lifecycle(cfg: &Cfg) {
    let base = cfg.dir.join(format!("life-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let size = 64 * 1024;
    let case =
        |name: &str, setup: &dyn Fn(&Path) -> (PathBuf, PathBuf), act: &dyn Fn(&Path, &Path)| {
            let dir = base.join(name);
            let _ = std::fs::create_dir_all(&dir);
            let (watched, victim) = setup(&dir);
            let Ok(iocp) = Iocp::new(1) else { return };
            let mut watch = match Watch::new(&watched, size, 1, 2, &iocp) {
                Ok(w) => w,
                Err(e) => {
                    emit(
                        cfg,
                        "lifecycle",
                        name,
                        &[("armed", "false".into()), ("error", json_str(&e))],
                    );
                    return;
                }
            };
            if let Err(e) = watch.arm(0, true) {
                emit(
                    cfg,
                    "lifecycle",
                    name,
                    &[
                        ("armed", "false".into()),
                        ("error", json_str(&format!("win32 {e}"))),
                    ],
                );
                return;
            }
            let _ = std::fs::write(watched.join("pre.txt"), b"x");
            let mut records_before = 0;
            if let Some(c) = dequeue_one(&iocp, Duration::from_secs(5)) {
                records_before = describe(&c, &watch, Class::Extended).2;
                if let Some(i) = watch.slot_of(c.overlapped) {
                    let _ = watch.slots[i].finish();
                }
                let _ = watch.arm(0, true);
            }
            act(&watched, &victim);
            let after = dequeue_one(&iocp, Duration::from_secs(10));
            let (status, bytes, records, _) =
                after.as_ref().map_or(("timeout".into(), 0, 0, None), |c| {
                    describe(c, &watch, Class::Extended)
                });
            if let Some(c) = after.as_ref()
                && let Some(i) = watch.slot_of(c.overlapped)
            {
                let _ = watch.slots[i].finish();
            }
            let rearm = watch.arm(0, true);
            emit(
                cfg,
                "lifecycle",
                name,
                &[
                    ("armed", "true".into()),
                    ("records_before", records_before.to_string()),
                    ("status_after", json_str(&status)),
                    ("bytes_after", bytes.to_string()),
                    ("records_after", records.to_string()),
                    (
                        "rearm",
                        json_str(&match rearm {
                            Ok(()) => "ok".into(),
                            Err(e) => format!("win32 {e}"),
                        }),
                    ),
                ],
            );
        };
    case(
        "root-deleted",
        &|d| (d.join("root"), d.join("root")),
        &|w, _| {
            let _ = std::fs::remove_dir_all(w);
        },
    );
    case(
        "root-renamed",
        &|d| (d.join("root"), d.join("root2")),
        &|w, v| {
            let _ = std::fs::rename(w, v);
        },
    );
    case(
        "parent-renamed",
        &|d| (d.join("parent").join("root"), d.join("parent2")),
        &|w, v| {
            if let Some(parent) = w.parent() {
                let _ = std::fs::rename(parent, v);
            }
        },
    );
    case(
        "root-deleted-and-recreated",
        &|d| (d.join("root"), d.join("root")),
        &|w, _| {
            let _ = std::fs::remove_dir_all(w);
            let _ = std::fs::create_dir_all(w);
        },
    );
    // Both setup paths must exist before arming.
    let _ = std::fs::remove_dir_all(&base);
}

/// Q5: the backing VHD is detached while a watch is armed.
fn detach(cfg: &Cfg) {
    let root = cfg.dir.join("detach-watch");
    let _ = std::fs::create_dir_all(&root);
    let Ok(iocp) = Iocp::new(1) else { return };
    let mut watch = match Watch::new(&root, 64 * 1024, 1, 2, &iocp) {
        Ok(w) => w,
        Err(e) => {
            emit(
                cfg,
                "detach",
                "armed",
                &[("armed", "false".into()), ("error", json_str(&e))],
            );
            return;
        }
    };
    if let Err(e) = watch.arm(0, true) {
        emit(
            cfg,
            "detach",
            "armed",
            &[
                ("armed", "false".into()),
                ("error", json_str(&format!("win32 {e}"))),
            ],
        );
        return;
    }
    let _ = std::fs::write(cfg.flag_dir.join("armed.flag"), b"1");
    let c = dequeue_one(&iocp, Duration::from_secs(60));
    let (status, bytes, records, _) = c.as_ref().map_or(("timeout".into(), 0, 0, None), |c| {
        describe(c, &watch, Class::Extended)
    });
    if let Some(c) = c.as_ref()
        && let Some(i) = watch.slot_of(c.overlapped)
    {
        let _ = watch.slots[i].finish();
    }
    let rearm = watch.arm(0, true);
    emit(
        cfg,
        "detach",
        "vhd-detached",
        &[
            ("armed", "true".into()),
            ("status", json_str(&status)),
            ("bytes", bytes.to_string()),
            ("records", records.to_string()),
            (
                "rearm",
                json_str(&match rearm {
                    Ok(()) => "ok".into(),
                    Err(e) => format!("win32 {e}"),
                }),
            ),
        ],
    );
}

fn cycles() -> u64 {
    let mut c = 0u64;
    // SAFETY: the pseudo handle needs no close; the out pointer is valid.
    unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut c) };
    c
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted
        .get(((sorted.len() as f64 - 1.0) * p).round() as usize)
        .copied()
        .unwrap_or(0)
}

/// Writer threads: create, modify, rename and delete small files in 64 directories.
fn spawn_writers(
    root: &Path,
    threads: usize,
    rate: u64,
    stop: Arc<AtomicBool>,
    issued: Arc<AtomicU64>,
    max_ops: u64,
) -> Vec<thread::JoinHandle<()>> {
    (0..threads)
        .map(|t| {
            let root = root.to_path_buf();
            let (stop, issued) = (Arc::clone(&stop), Arc::clone(&issued));
            thread::spawn(move || {
                let started = Instant::now();
                let per_thread = if rate == 0 {
                    0
                } else {
                    (rate / threads as u64).max(1)
                };
                let mut i = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    if max_ops > 0 && issued.load(Ordering::Relaxed) >= max_ops {
                        break;
                    }
                    if per_thread > 0 {
                        let due = started + Duration::from_nanos(i * 1_000_000_000 / per_thread);
                        while Instant::now() < due && !stop.load(Ordering::Relaxed) {
                            std::hint::spin_loop();
                        }
                    }
                    let d = root.join(format!("d{}", (i + t as u64) % 64));
                    let f = d.join(format!("t{t}-{i}.tmp"));
                    if std::fs::write(&f, b"0123456789abcdef0123456789abcdef").is_ok() {
                        if i % 4 == 0 {
                            let g = d.join(format!("t{t}-{i}.dat"));
                            if std::fs::rename(&f, &g).is_ok() && i % 8 == 0 {
                                let _ = std::fs::remove_file(&g);
                            }
                        }
                    }
                    issued.fetch_add(1, Ordering::Relaxed);
                    i += 1;
                }
            })
        })
        .collect()
}

struct PerfResult {
    ops: u64,
    elapsed: Duration,
    records: u64,
    bytes: u64,
    completions: u64,
    overflows: u64,
    rearm_us: Vec<u64>,
    cycles: u64,
}

/// One reader run: arm, dequeue, re-arm (before or after parsing), count overflows.
fn run_perf(
    cfg: &Cfg,
    size: usize,
    rate: u64,
    secs: u64,
    max_ops: u64,
    stall_ms: u64,
    double: bool,
) -> PerfResult {
    let root = cfg
        .dir
        .join(format!("perf-{}-{size}-{rate}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in 0..64 {
        let _ = std::fs::create_dir_all(root.join(format!("d{d}")));
    }
    let iocp = Iocp::new(1).expect("iocp");
    let slots = if double { 2 } else { 1 };
    let mut watch = Watch::new(&root, size, slots, 2, &iocp).expect("watch");
    watch.arm(0, true).expect("arm");
    let stop = Arc::new(AtomicBool::new(false));
    let issued = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let writers = spawn_writers(
        &root,
        cfg.threads,
        rate,
        Arc::clone(&stop),
        Arc::clone(&issued),
        max_ops,
    );
    let (mut records, mut bytes, mut completions, mut overflows) = (0u64, 0u64, 0u64, 0u64);
    let mut rearm_us = Vec::new();
    let mut stalled = false;
    let mut cycles_used = 0u64;
    let deadline = started + Duration::from_secs(secs);
    let mut batch = Batch::new(8);
    let mut idle_after_done = 0;
    loop {
        let now = Instant::now();
        if max_ops == 0 && now >= deadline {
            break;
        }
        let writers_done = max_ops > 0 && issued.load(Ordering::Relaxed) >= max_ops;
        let n = iocp
            .dequeue(&mut batch, Some(Duration::from_millis(100)))
            .unwrap_or(0);
        if n == 0 {
            if writers_done {
                idle_after_done += 1;
                if idle_after_done > 5 {
                    break;
                }
            }
            continue;
        }
        let c0 = cycles();
        for c in batch.iter() {
            let t_deq = Instant::now();
            if stall_ms > 0 && !stalled {
                stalled = true;
                thread::sleep(Duration::from_millis(stall_ms));
            }
            completions += 1;
            let Some(i) = watch.slot_of(c.overlapped) else {
                continue;
            };
            let overflow = !c.is_success() || c.bytes == 0;
            if overflow {
                overflows += 1;
            }
            let parse_slot = |watch: &Watch, records: &mut u64, bytes: &mut u64| {
                if !overflow
                    && let Some(data) = watch.slots.get(i).and_then(|s| s.data(c.bytes as usize))
                {
                    *bytes += u64::from(c.bytes);
                    for item in parse(data, c.bytes as usize, Class::Extended) {
                        if item.is_ok() {
                            *records += 1;
                            if cfg.work_ns > 0 {
                                let t = Instant::now();
                                while t.elapsed() < Duration::from_nanos(cfg.work_ns) {
                                    std::hint::spin_loop();
                                }
                            }
                        }
                    }
                }
            };
            if double {
                // Re-arm the spare first, then parse the completed buffer.
                let spare = 1 - i;
                let _ = watch.arm(spare, true);
                rearm_us.push(u64::try_from(t_deq.elapsed().as_micros()).unwrap_or(u64::MAX));
                parse_slot(&watch, &mut records, &mut bytes);
                if let Some(s) = watch.slots.get_mut(i) {
                    let _ = s.finish();
                }
            } else {
                parse_slot(&watch, &mut records, &mut bytes);
                if let Some(s) = watch.slots.get_mut(i) {
                    let _ = s.finish();
                }
                let _ = watch.arm(i, true);
                rearm_us.push(u64::try_from(t_deq.elapsed().as_micros()).unwrap_or(u64::MAX));
            }
        }
        cycles_used += cycles() - c0;
    }
    stop.store(true, Ordering::Relaxed);
    let elapsed = started.elapsed();
    for w in writers {
        let _ = w.join();
    }
    let ops = issued.load(Ordering::Relaxed);
    // Cancel the armed reads and drain before the directory handle drops.
    for i in 0..slots {
        let _ = watch.dir.cancel(&watch.slots[i]);
    }
    for _ in 0..slots {
        let _ = dequeue_one(&iocp, Duration::from_millis(500));
    }
    drop(watch);
    let _ = std::fs::remove_dir_all(&root);
    rearm_us.sort_unstable();
    PerfResult {
        ops,
        elapsed,
        records,
        bytes,
        completions,
        overflows,
        rearm_us,
        cycles: cycles_used,
    }
}

fn perf(cfg: &Cfg) {
    let double = cfg.mode == "double";
    let r = run_perf(cfg, cfg.size, cfg.rate, cfg.secs, 0, 0, double);
    let secs = r.elapsed.as_secs_f64();
    emit(
        cfg,
        "perf",
        &format!("size{}K-rate{}-{}", cfg.size / 1024, cfg.rate, cfg.mode),
        &[
            ("size_kib", (cfg.size / 1024).to_string()),
            ("target_ops_per_s", cfg.rate.to_string()),
            ("mode", json_str(&cfg.mode)),
            ("work_ns", cfg.work_ns.to_string()),
            (
                "achieved_ops_per_s",
                ((r.ops as f64 / secs) as u64).to_string(),
            ),
            (
                "records_per_s",
                ((r.records as f64 / secs) as u64).to_string(),
            ),
            ("completions", r.completions.to_string()),
            ("overflows", r.overflows.to_string()),
            (
                "bytes_per_record",
                if r.records > 0 {
                    format!("{:.1}", r.bytes as f64 / r.records as f64)
                } else {
                    "0".into()
                },
            ),
            (
                "records_per_completion",
                if r.completions > 0 {
                    format!("{:.0}", r.records as f64 / r.completions as f64)
                } else {
                    "0".into()
                },
            ),
            ("rearm_p50_us", percentile(&r.rearm_us, 0.5).to_string()),
            ("rearm_p99_us", percentile(&r.rearm_us, 0.99).to_string()),
            (
                "rearm_max_us",
                r.rearm_us.last().copied().unwrap_or(0).to_string(),
            ),
            (
                "cycles_per_1000_records",
                if r.records > 0 {
                    (r.cycles * 1000 / r.records).to_string()
                } else {
                    "0".into()
                },
            ),
        ],
    );
}

fn burst(cfg: &Cfg) {
    let r = run_perf(cfg, cfg.size, 0, 30, cfg.ops, cfg.stall_ms, false);
    emit(
        cfg,
        "burst",
        &format!(
            "size{}K-stall{}ms-ops{}",
            cfg.size / 1024,
            cfg.stall_ms,
            cfg.ops
        ),
        &[
            ("size_kib", (cfg.size / 1024).to_string()),
            ("stall_ms", cfg.stall_ms.to_string()),
            ("ops", r.ops.to_string()),
            ("records", r.records.to_string()),
            ("overflows", r.overflows.to_string()),
            ("overflowed", (r.overflows > 0).to_string()),
        ],
    );
}

fn nonpaged() -> (u64, u64) {
    // SAFETY: zero is a valid initial state; the size is set before the call.
    let mut info: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() };
    info.cb = u32::try_from(std::mem::size_of::<PERFORMANCE_INFORMATION>()).unwrap_or(0);
    // SAFETY: the struct and its size are valid for the call.
    unsafe { GetPerformanceInfo(&raw mut info, info.cb) };
    (info.KernelNonpaged as u64, info.PageSize as u64)
}

fn pool(cfg: &Cfg) {
    let root = cfg.dir.join(format!("pool-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    let iocp = Iocp::new(1).expect("iocp");
    thread::sleep(Duration::from_millis(500));
    let (before, page) = nonpaged();
    let mut watches = Vec::new();
    for i in 0..cfg.n {
        let sub = root.join(format!("w{i}"));
        let _ = std::fs::create_dir_all(&sub);
        if let Ok(mut w) = Watch::new(&sub, cfg.size, 1, 2, &iocp) {
            let _ = w.arm(0, true);
            watches.push(w);
        }
    }
    thread::sleep(Duration::from_millis(500));
    let (armed, _) = nonpaged();
    emit(
        cfg,
        "pool",
        &format!("size{}K-n{}", cfg.size / 1024, cfg.n),
        &[
            ("size_kib", (cfg.size / 1024).to_string()),
            ("handles", watches.len().to_string()),
            ("nonpaged_pages_before", before.to_string()),
            ("nonpaged_pages_armed", armed.to_string()),
            ("page_size", page.to_string()),
            (
                "delta_bytes",
                (armed.saturating_sub(before) * page).to_string(),
            ),
            (
                "delta_bytes_per_handle",
                if watches.is_empty() {
                    "0".into()
                } else {
                    (armed.saturating_sub(before) * page / watches.len() as u64).to_string()
                },
            ),
        ],
    );
    for w in &watches {
        let _ = w.dir.cancel(&w.slots[0]);
    }
    for _ in 0..watches.len() {
        let _ = dequeue_one(&iocp, Duration::from_millis(200));
    }
    drop(watches);
    let _ = std::fs::remove_dir_all(&root);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let exp = args.next().unwrap_or_default();
    let mut cfg = Cfg {
        dir: std::env::temp_dir(),
        label: std::env::var("BENCH_LABEL").unwrap_or_else(|_| "local".into()),
        size: 1024 * 1024,
        rate: 10_000,
        secs: 5,
        ops: 10_000,
        stall_ms: 0,
        mode: "single".into(),
        work_ns: 0,
        threads: 4,
        n: 10,
        flag_dir: std::env::temp_dir(),
    };
    while let Some(flag) = args.next() {
        let v = args.next().unwrap_or_default();
        match flag.as_str() {
            "--dir" => cfg.dir = PathBuf::from(v),
            "--label" => cfg.label = v,
            "--size" => cfg.size = v.parse().unwrap_or(cfg.size),
            "--rate" => cfg.rate = v.parse().unwrap_or(cfg.rate),
            "--secs" => cfg.secs = v.parse().unwrap_or(cfg.secs),
            "--ops" => cfg.ops = v.parse().unwrap_or(cfg.ops),
            "--stall-ms" => cfg.stall_ms = v.parse().unwrap_or(0),
            "--mode" => cfg.mode = v,
            "--work-ns" => cfg.work_ns = v.parse().unwrap_or(0),
            "--threads" => cfg.threads = v.parse().unwrap_or(cfg.threads),
            "--n" => cfg.n = v.parse().unwrap_or(cfg.n),
            "--flag-dir" => cfg.flag_dir = PathBuf::from(v),
            _ => {}
        }
    }
    match exp.as_str() {
        "matrix" => matrix(&cfg),
        "lifecycle" => lifecycle(&cfg),
        "detach" => detach(&cfg),
        "perf" => perf(&cfg),
        "burst" => burst(&cfg),
        "pool" => pool(&cfg),
        _ => println!("usage: rdcw-bench matrix|lifecycle|detach|perf|burst|pool ..."),
    }
}
