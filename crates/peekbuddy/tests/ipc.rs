//! The engine pipe against the real exe (#61): who may connect, what a second
//! launch does, and what happens when the name is taken.
#![cfg(windows)]
#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns may unwrap"
)]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use peekbuddy_engine::Names;
use peekbuddy_testkit::TestDir;
use peekbuddy_win::window::WindowHandle;

const EXE: &str = env!("CARGO_BIN_EXE_peekbuddy");

struct Engine(Child);

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(data: &Path) -> (Engine, WindowHandle) {
    let mut child = Command::new(EXE)
        .args(["--headless", "--data-dir"])
        .arg(data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let class = Names::new(Some(data)).window_class;
    let deadline = Instant::now() + Duration::from_secs(20);
    let window = loop {
        if let Some(window) = WindowHandle::find(&class) {
            break window;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "the engine exited before its window appeared"
        );
        assert!(Instant::now() < deadline, "no engine window");
        std::thread::sleep(Duration::from_millis(20));
    };
    (Engine(child), window)
}

fn ping_command(data: &Path) -> String {
    format!("\"{EXE}\" --ipc-ping --data-dir \"{}\"", data.display())
}

fn ping_as_this_user(data: &Path) -> i32 {
    Command::new(EXE)
        .args(["--ipc-ping", "--data-dir"])
        .arg(data)
        .status()
        .unwrap()
        .code()
        .unwrap()
}

#[test]
fn a_client_of_the_same_user_reaches_the_engine() {
    let dir = TestDir::new("ipc-same").unwrap();
    let (mut engine, window) = start(dir.path());
    assert_eq!(ping_as_this_user(dir.path()), 0);
    // An unelevated child of the same user is also welcome: the viewer is one.
    assert_eq!(
        peekbuddy_win::process::run_restricted(&ping_command(dir.path())).unwrap(),
        0
    );
    window.post_close().unwrap();
    assert!(engine.0.wait().unwrap().success());
}

#[test]
fn a_low_integrity_process_is_refused() {
    let dir = TestDir::new("ipc-low").unwrap();
    let (_engine, _window) = start(dir.path());
    // ERROR_ACCESS_DENIED
    assert_eq!(
        peekbuddy_win::process::run_low_integrity(&ping_command(dir.path())).unwrap(),
        5
    );
}

/// Creates a local user, pings as that user, deletes the user. Needs an
/// elevated token (hosted runners have one); skipped otherwise.
#[test]
fn a_second_local_user_is_refused() {
    if !peekbuddy_win::token::is_admin().unwrap() {
        return;
    }
    let user = format!("pbipc{}", std::process::id() % 100_000);
    // Same shape as the runner probe (#34): a fixed prefix and hex digits.
    let password = format!("Pb!{:010x}", std::process::id().wrapping_mul(2_654_435_761));
    let net = |args: &[&str]| {
        let output = Command::new("net.exe").args(args).output().unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned()
                + &String::from_utf8_lossy(&output.stderr),
        )
    };
    let (added, detail) = net(&["user", &user, &password, "/add"]);
    assert!(added, "net user /add failed: {detail}");
    let dir = TestDir::new("ipc-user").unwrap();
    let (_engine, _window) = start(dir.path());
    // The other account must be able to run the exe and read the data dir.
    let result = peekbuddy_win::process::run_as_user(&user, &password, &ping_command(dir.path()));
    let _ = net(&["user", &user, "/delete"]);
    // ERROR_ACCESS_DENIED from the pipe; an account that cannot even start
    // the exe (profile or policy) is reported as an error, not as success.
    assert_eq!(result.unwrap(), 5);
}

#[test]
fn a_second_launch_exits_zero_and_shows_the_viewer_through_the_pipe() {
    let dir = TestDir::new("ipc-forward").unwrap();
    let (mut engine, window) = start(dir.path());
    let started = Instant::now();
    let status = Command::new(EXE)
        .args(["--headless", "--data-dir"])
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(engine.0.try_wait().unwrap().is_none());
    window.post_close().unwrap();
    assert!(engine.0.wait().unwrap().success());
}

#[test]
fn a_taken_pipe_name_stops_the_engine_with_a_logged_reason() {
    let dir = TestDir::new("ipc-squat").unwrap();
    let data = dir.path().join("data");
    // Someone else owns the name first.
    let name = peekbuddy_engine::ipc_server::pipe_name(Some(&data)).unwrap();
    let silent: std::sync::Arc<peekbuddy_win::pipe::Handler> = std::sync::Arc::new(
        |_: &peekbuddy_core::ipc::Message, _: &mut peekbuddy_win::pipe::Reply| {},
    );
    let squatter = peekbuddy_win::pipe::PipeServer::start(&name, &silent).unwrap();
    let status = Command::new(EXE)
        .args(["--headless", "--data-dir"])
        .arg(&data)
        .status()
        .unwrap();
    assert!(!status.success(), "the engine ran next to a squatter");
    let log = std::fs::read_to_string(data.join("logs").join("ipc-error.log")).unwrap();
    assert!(log.contains("could not create"), "{log}");
    drop(squatter);
}

#[test]
fn shutting_down_tells_a_connected_client() {
    use peekbuddy_core::ipc::Message;
    use peekbuddy_win::pipe::PipeClient;

    let dir = TestDir::new("ipc-stop").unwrap();
    let (mut engine, window) = start(dir.path());
    let name = peekbuddy_engine::ipc_server::pipe_name(Some(dir.path())).unwrap();
    let mut client =
        PipeClient::connect(&name, Some(Path::new(EXE)), Duration::from_secs(5)).unwrap();
    client
        .send(&Message::Hello {
            protocol_version: peekbuddy_core::ipc::PROTOCOL_VERSION,
            build: String::from("test"),
        })
        .unwrap();
    assert!(matches!(
        client.receive().unwrap(),
        Some(Message::Hello { .. })
    ));
    window.post_close().unwrap();
    assert_eq!(client.receive().unwrap(), Some(Message::EngineStopping));
    assert!(engine.0.wait().unwrap().success());
}

#[test]
fn a_wrong_protocol_version_is_answered_with_a_mismatch() {
    use peekbuddy_core::ipc::Message;
    use peekbuddy_win::pipe::PipeClient;

    let dir = TestDir::new("ipc-version").unwrap();
    let (_engine, _window) = start(dir.path());
    let name = peekbuddy_engine::ipc_server::pipe_name(Some(dir.path())).unwrap();
    let mut client = PipeClient::connect(&name, None, Duration::from_secs(5)).unwrap();
    client
        .send(&Message::Hello {
            protocol_version: 999,
            build: String::new(),
        })
        .unwrap();
    assert_eq!(
        client.receive().unwrap(),
        Some(Message::VersionMismatch {
            engine_version: peekbuddy_core::ipc::PROTOCOL_VERSION
        })
    );
}
