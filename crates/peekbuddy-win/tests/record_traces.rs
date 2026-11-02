//! Records real traces of real tools (#41, #270): the events the capture
//! backend delivers while `git`, `cargo`, `robocopy` and `tar` work in a
//! watched tree. The traces are the fixtures of the derive tests in
//! `peekbuddy-core/tests/derive_traces.rs`.
//!
//! The test does nothing unless `PEEKBUDDY_TRACE_OUT` names the directory to
//! write to; with it set, `cargo test -p peekbuddy-win --test record_traces`
//! regenerates `*.jsonl` there. Notepad, VS Code and Office are not recorded:
//! they are not on the hosted runners and their saves cannot be scripted
//! without a desktop session.
//!
//! One line per event after the header:
//! `{"src":"event","ts":<ticks since the first>,"kind":[tag,arg],
//! "file_id":"<hex>","path_u16":[...]}`, with the path relative to the root.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::fmt::Write as _;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use peekbuddy_core::capture::EventSink;
use peekbuddy_core::event::{Event, GapReason, RootIdx, VolIdx, WatchState};
use peekbuddy_win::rdcw::{Detail, LifecycleSink, RootSpec, Watcher};

const ROOT: RootIdx = RootIdx(0);

/// The events of a watch with their names.
type Recorded = Vec<(Event, Vec<u16>)>;

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Recorded>>);

impl EventSink for Events {
    fn push(&mut self, event: &Event, name: &[u16]) -> bool {
        self.0.lock().unwrap().push((*event, name.to_vec()));
        true
    }
}

struct Life;

impl LifecycleSink for Life {
    fn state(&mut self, _: RootIdx, _: WatchState, _: Detail) {}
    fn reconcile(&mut self, _: RootIdx, _: GapReason) {}
}

/// Runs `program` with `args` in `dir`; the output is discarded.
fn tool(dir: &Path, program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "NUL")
        .env("GIT_CONFIG_SYSTEM", "NUL")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn version(program: &str) -> String {
    if program == "robocopy" {
        // It has no --version and prints its help for one.
        return String::from("robocopy (Windows)");
    }
    Command::new(program)
        .arg("--version")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn git(dir: &Path, args: &[&str]) {
    let mut full = vec!["-c", "user.name=t", "-c", "user.email=t@example.com"];
    full.extend_from_slice(args);
    assert!(tool(dir, "git", &full), "git {args:?} failed");
}

/// A scenario: what to prepare before the watch starts, and what to run
/// while it watches.
struct Scenario {
    name: &'static str,
    tool: &'static str,
    prepare: fn(&Path),
    run: fn(&Path),
}

fn write_tree(root: &Path, dirs: usize, files: usize, tag: &str) {
    for dir in 0..dirs {
        let path = root.join(format!("dir{dir}"));
        std::fs::create_dir_all(&path).unwrap();
        for file in 0..files {
            std::fs::write(
                path.join(format!("file{file}.txt")),
                format!("{tag} {dir} {file}\n").repeat(40),
            )
            .unwrap();
        }
    }
}

fn git_scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "git-checkout",
            tool: "git",
            prepare: |root| {
                git(root, &["init", "-q", "-b", "main"]);
                write_tree(root, 5, 8, "main");
                git(root, &["add", "-A"]);
                git(root, &["commit", "-q", "-m", "main"]);
                git(root, &["checkout", "-q", "-b", "other"]);
                for dir in 0..3 {
                    std::fs::write(
                        root.join(format!("dir{dir}")).join("file0.txt"),
                        "changed\n".repeat(60),
                    )
                    .unwrap();
                }
                std::fs::remove_file(root.join("dir4").join("file7.txt")).unwrap();
                std::fs::create_dir_all(root.join("added")).unwrap();
                std::fs::write(root.join("added").join("new.txt"), "new\n").unwrap();
                git(root, &["add", "-A"]);
                git(root, &["commit", "-q", "-m", "other"]);
                git(root, &["checkout", "-q", "main"]);
            },
            run: |root| {
                git(root, &["checkout", "-q", "other"]);
                git(root, &["checkout", "-q", "main"]);
            },
        },
        Scenario {
            name: "git-commit",
            tool: "git",
            prepare: |root| {
                git(root, &["init", "-q", "-b", "main"]);
                write_tree(root, 3, 6, "base");
                git(root, &["add", "-A"]);
                git(root, &["commit", "-q", "-m", "base"]);
            },
            run: |root| {
                std::fs::write(root.join("dir0").join("file0.txt"), "edited\n").unwrap();
                std::fs::write(root.join("dir1").join("brand-new.txt"), "x\n").unwrap();
                git(root, &["add", "-A"]);
                git(root, &["commit", "-q", "-m", "edit"]);
                git(root, &["status", "-s"]);
            },
        },
    ]
}

fn tool_scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "cargo-build",
            tool: "cargo",
            prepare: |root| {
                std::fs::create_dir_all(root.join("src")).unwrap();
                std::fs::write(
                    root.join("Cargo.toml"),
                    "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                )
                .unwrap();
                std::fs::write(
                    root.join("src").join("main.rs"),
                    "fn main() { println!(\"hi\"); }\n",
                )
                .unwrap();
            },
            run: |root| {
                assert!(tool(root, "cargo", &["build", "--offline", "-q"]));
                std::fs::write(
                    root.join("src").join("main.rs"),
                    "fn main() { println!(\"changed\"); }\n",
                )
                .unwrap();
                assert!(tool(root, "cargo", &["build", "--offline", "-q"]));
            },
        },
        Scenario {
            name: "robocopy-mirror",
            tool: "robocopy",
            prepare: |root| {
                write_tree(&root.join("src"), 4, 10, "v1");
                std::fs::create_dir_all(root.join("dst")).unwrap();
                // robocopy reports differences with exit codes 1 to 7.
                let _ = tool(
                    root,
                    "robocopy",
                    &["src", "dst", "/MIR", "/NFL", "/NDL", "/NJH", "/NJS"],
                );
                write_tree(&root.join("src"), 2, 10, "v2");
                std::fs::remove_file(root.join("src").join("dir3").join("file9.txt")).unwrap();
            },
            run: |root| {
                let _ = tool(
                    root,
                    "robocopy",
                    &["src", "dst", "/MIR", "/NFL", "/NDL", "/NJH", "/NJS"],
                );
            },
        },
        Scenario {
            name: "tar-extract",
            tool: "tar",
            prepare: |root| {
                write_tree(&root.join("pack"), 4, 10, "archive");
                assert!(tool(root, "tar", &["-cf", "pack.tar", "pack"]));
                std::fs::remove_dir_all(root.join("pack")).unwrap();
            },
            run: |root| {
                assert!(tool(root, "tar", &["-xf", "pack.tar"]));
                // The same archive over the existing tree: replaced files.
                assert!(tool(root, "tar", &["-xf", "pack.tar"]));
            },
        },
    ]
}

fn wait_quiet(events: &Events) {
    let mut last = 0;
    let mut since = Instant::now();
    while since.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(100));
        let now = events.0.lock().unwrap().len();
        if now != last {
            last = now;
            since = Instant::now();
        }
    }
}

fn record(scenario: &Scenario, out: &Path) {
    let root =
        std::env::temp_dir().join(format!("pb-trace-{}-{}", scenario.name, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root: PathBuf = std::fs::canonicalize(&root).unwrap();
    (scenario.prepare)(&root);
    std::thread::sleep(Duration::from_millis(500));

    let events = Events::default();
    let watcher = Watcher::start(events.clone(), Life).unwrap();
    watcher
        .add_root(RootSpec {
            root: ROOT,
            vol: VolIdx(0),
            path: root.as_os_str().encode_wide().collect(),
            recursive: true,
            last_access: false,
            buffer_bytes: 1024 * 1024,
            exclude: Vec::new(),
            reject: Vec::new(),
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    events.0.lock().unwrap().clear();

    (scenario.run)(&root);
    wait_quiet(&events);
    let _ = watcher.stop();

    let recorded = events.0.lock().unwrap().clone();
    let first = recorded.first().map_or(0, |(event, _)| event.ts);
    let mut text = format!(
        "{{\"src\":\"header\",\"scenario\":\"{}\",\"tool\":{:?},\"events\":{}}}\n",
        scenario.name,
        version(scenario.tool),
        recorded.len()
    );
    for (event, name) in &recorded {
        let (tag, arg) = event.kind.to_wire();
        let units: Vec<String> = name.iter().map(u16::to_string).collect();
        writeln!(
            text,
            "{{\"src\":\"event\",\"ts\":{},\"kind\":[{tag},{arg}],\"file_id\":\"{:x}\",\"path_u16\":[{}]}}",
            event.ts.saturating_sub(first),
            event.file_id,
            units.join(",")
        )
        .unwrap();
    }
    std::fs::write(out.join(format!("{}.jsonl", scenario.name)), text).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(!recorded.is_empty(), "{}: no event", scenario.name);
}

#[test]
fn record_the_traces() {
    let Some(out) = std::env::var_os("PEEKBUDDY_TRACE_OUT") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    for scenario in git_scenarios().into_iter().chain(tool_scenarios()) {
        record(&scenario, &out);
    }
}
