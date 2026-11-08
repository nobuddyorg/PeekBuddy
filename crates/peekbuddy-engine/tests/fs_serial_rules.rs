//! A rule with a command, end to end (#81): a file that matches the glob in
//! a watched tree makes the engine start the configured program, and a file
//! that does not match starts nothing.
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use peekbuddy_core::perf::MilestoneTable;
use peekbuddy_engine::ipc_server::{ping, pipe_name};
use peekbuddy_engine::{Names, Options, ProbeOptions, register_provider, run};
use peekbuddy_win::window::WindowHandle;

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(90);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_matching_file_starts_the_rules_command() {
    let root = std::env::temp_dir().join(format!("pb-rules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = Dir(root);
    let data = dir.0.join("data");
    let tree = dir.0.join("tree");
    let out = dir.0.join("ran.txt");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&tree).unwrap();
    let tree = std::fs::canonicalize(&tree).unwrap();
    let plain = tree
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| String::from(r"C:\Windows"));
    let cmd = format!(r"{system}\System32\cmd.exe");
    std::fs::write(
        data.join("settings.toml"),
        format!(
            "version = 1\n[general]\nfirst_run = false\n\
             [[watch]]\nid = \"w0\"\npath = '{plain}'\n\
             [[rule]]\nname = \"Triggers\"\nglob = \"*.trigger\"\n\
             kinds = [\"created\"]\ntoast = false\n\
             [rule.command]\nargv = ['{cmd}', '/c', 'echo', '%path%', '>', '{}']\n\
             confirmed = true\n",
            out.display()
        ),
    )
    .unwrap();

    let options = Options {
        headless: true,
        data_dir: Some(data.clone()),
        probe: Some(ProbeOptions {
            seconds: 8,
            out: None,
        }),
        ..Options::default()
    };
    let table = Arc::new(MilestoneTable::new());
    let engine = {
        let table = Arc::clone(&table);
        std::thread::spawn(move || run(&options, &table, register_provider()))
    };
    let class = Names::new(Some(&data)).window_class;
    wait_for("the engine window", || WindowHandle::find(&class).is_some());
    let pipe = pipe_name(Some(&data)).unwrap();
    let exe = std::env::current_exe().unwrap();
    wait_for("the engine pipe", || ping(&pipe, &exe).is_ok());

    // plain.txt does not match and must not run the command; go.trigger does. A
    // file written before the watch is armed is never a Created event, so
    // every try makes it new again.
    wait_for("the command to run", || {
        let _ = std::fs::write(tree.join("plain.txt"), b"x");
        let _ = std::fs::remove_file(tree.join("go.trigger"));
        std::fs::write(tree.join("go.trigger"), b"x").is_ok() && out.exists()
    });
    let said = std::fs::read_to_string(&out).unwrap();
    // Every run says the path it was started for; a plain.txt would show.
    // cmd quotes a path with a space as the argument had it.
    let said = said.trim().trim_matches('"');
    assert!(
        said.ends_with("go.trigger") && !said.starts_with(r"\\?\"),
        "{said}"
    );
    engine.join().unwrap().unwrap();
}
