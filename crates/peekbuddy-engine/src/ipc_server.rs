//! The engine's side of the viewer protocol (#61).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use peekbuddy_core::ipc::{Command, Message, PROTOCOL_VERSION};
use peekbuddy_core::settings::{self, Context, Loaded};
use peekbuddy_win::pipe::{self, PipeClient, PipeServer, Reply};
use peekbuddy_win::window::WindowHandle;

/// The build string sent in `Hello`.
#[must_use]
pub fn build() -> String {
    format!("peekbuddy {}", env!("CARGO_PKG_VERSION"))
}

/// The pipe name: this user's pipe, or for an isolated test engine (an
/// explicit data directory) one derived from that directory.
///
/// # Errors
///
/// The OS error when the user SID cannot be read.
pub fn pipe_name(data_dir: Option<&Path>) -> std::io::Result<String> {
    match data_dir {
        None => pipe::user_pipe_name(),
        Some(dir) => Ok(format!(
            r"\\.\pipe\PeekBuddy-test-{:016x}",
            pipe::stable_hash(&dir.to_string_lossy().to_lowercase())
        )),
    }
}

fn hello() -> Message {
    Message::Hello {
        protocol_version: PROTOCOL_VERSION,
        build: build(),
    }
}

/// What the handler may touch. Everything here is safe to use from the pipe
/// threads.
#[derive(Clone)]
pub struct Shared {
    /// The engine's window, to post a close to.
    pub window: WindowHandle,
    /// This exe, spawned as `--viewer`.
    pub exe: PathBuf,
    /// The settings file and the data directory it lives in.
    pub settings_path: PathBuf,
    /// Data directory text for validation.
    pub data_dir: String,
}

fn command_result(ok: bool, message: impl Into<String>) -> Message {
    Message::CommandResult {
        ok,
        message: message.into(),
    }
}

fn apply_settings(ctx: &Shared, text: &str) -> Message {
    let validation = Context {
        data_dir: ctx.data_dir.clone(),
        export_dir: None,
    };
    match settings::load(text, &validation) {
        Loaded::Ok { .. } => {
            match peekbuddy_win::atomic::write_atomic(&ctx.settings_path, text.as_bytes()) {
                Ok(()) => {
                    command_result(true, "saved; the watches change with the next start (#68)")
                }
                Err(error) => {
                    command_result(false, format!("could not write settings.toml: {error}"))
                }
            }
        }
        Loaded::TooNew { version, .. } => command_result(
            false,
            format!("version {version} is newer than this build reads"),
        ),
        Loaded::Invalid(problems) => command_result(
            false,
            problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}

/// Handles one message from a client.
pub fn handle(ctx: &Shared, message: &Message, reply: &mut Reply) {
    match message {
        Message::Hello {
            protocol_version, ..
        } => {
            if *protocol_version == PROTOCOL_VERSION {
                reply.send(&hello());
            } else {
                reply.send(&Message::VersionMismatch {
                    engine_version: PROTOCOL_VERSION,
                });
            }
        }
        Message::ShowViewer => {
            let _ = std::process::Command::new(&ctx.exe).arg("--viewer").spawn();
        }
        Message::Shutdown => {
            let _ = ctx.window.post_close();
        }
        Message::Selftest { payload } => {
            reply.send(&Message::Selftest {
                payload: payload.clone(),
            });
        }
        Message::Command(Command::ApplySettings { toml }) => {
            reply.send(&apply_settings(ctx, toml));
        }
        Message::Command(_) => {
            reply.send(&command_result(
                false,
                "pause, resume and watch changes arrive with #63 and #68",
            ));
        }
        // Messages the engine sends, not receives; ignored if a client
        // sends them.
        Message::VersionMismatch { .. }
        | Message::EngineStopping
        | Message::Counters(_)
        | Message::WatchState { .. }
        | Message::Resync { .. }
        | Message::CommandResult { .. } => {}
    }
}

/// Starts the server. Fails closed: a pipe that already exists (a second
/// engine of this user in another session, or a squatter) is an error.
///
/// # Errors
///
/// The OS error from creating the first pipe instance.
pub fn start(name: &str, ctx: Shared) -> std::io::Result<PipeServer> {
    let handler: Arc<pipe::Handler> =
        Arc::new(move |message: &Message, reply: &mut Reply| handle(&ctx, message, reply));
    PipeServer::start(name, &handler)
}

/// Tells a running engine to show the viewer: what a second launch does.
/// Failure is not an error worth reporting; the second instance exits 0
/// either way.
///
/// # Errors
///
/// The error from connecting or talking to the pipe.
pub fn forward_show_viewer(name: &str, exe: &Path) -> std::io::Result<()> {
    let mut client = PipeClient::connect(name, Some(exe), Duration::from_millis(500))?;
    client.send(&hello())?;
    match client.receive()? {
        Some(Message::Hello { .. }) => client.send(&Message::ShowViewer),
        // A version mismatch or a closed pipe: nothing more to do.
        _ => Ok(()),
    }
}

/// `peekbuddy --ipc-ping`: connects, says hello, expects hello back.
///
/// # Errors
///
/// The error from connecting (access denied for a client the DACL refuses)
/// or an unexpected answer.
pub fn ping(name: &str, exe: &Path) -> std::io::Result<()> {
    let mut client = PipeClient::connect(name, Some(exe), Duration::from_secs(3))?;
    client.send(&hello())?;
    match client.receive()? {
        Some(Message::Hello { .. }) => Ok(()),
        other => Err(std::io::Error::other(format!(
            "unexpected answer {other:?}"
        ))),
    }
}

#[cfg(test)]
#[expect(clippy::panic, reason = "a test panics on an unexpected answer")]
mod tests {
    use std::path::PathBuf;

    use peekbuddy_core::ipc::{Command, Message, PROTOCOL_VERSION};
    use peekbuddy_win::pipe::Reply;
    use peekbuddy_win::window::WindowHandle;

    use super::{Shared, handle, pipe_name};

    fn ctx(dir: &std::path::Path) -> Shared {
        Shared {
            window: WindowHandle::find("PeekBuddy.Test.NoSuchWindow").unwrap_or_else(|| {
                // A handle that does not exist: posting to it fails harmlessly.
                WindowHandle::from_raw(0)
            }),
            exe: PathBuf::from("peekbuddy.exe"),
            settings_path: dir.join("settings.toml"),
            data_dir: dir.to_string_lossy().into_owned(),
        }
    }

    fn answer(ctx: &Shared, message: &Message) -> Vec<Message> {
        let mut reply = Reply::default();
        handle(ctx, message, &mut reply);
        let mut decoder = peekbuddy_core::ipc::FrameDecoder::new();
        decoder.push(reply.bytes());
        let mut out = Vec::new();
        while let Some(message) = decoder.next_message().unwrap() {
            out.push(message);
        }
        out
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-ipc-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hello_is_answered_and_a_wrong_version_is_a_mismatch() {
        let dir = dir("hello");
        let ctx = ctx(&dir);
        let ok = answer(
            &ctx,
            &Message::Hello {
                protocol_version: PROTOCOL_VERSION,
                build: String::from("x"),
            },
        );
        assert!(matches!(
            ok.as_slice(),
            [Message::Hello {
                protocol_version: PROTOCOL_VERSION,
                ..
            }]
        ));
        let old = answer(
            &ctx,
            &Message::Hello {
                protocol_version: PROTOCOL_VERSION + 1,
                build: String::new(),
            },
        );
        assert_eq!(
            old,
            [Message::VersionMismatch {
                engine_version: PROTOCOL_VERSION
            }]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn selftest_is_echoed_and_engine_messages_from_a_client_are_ignored() {
        let dir = dir("echo");
        let ctx = ctx(&dir);
        assert_eq!(
            answer(
                &ctx,
                &Message::Selftest {
                    payload: vec![1, 2]
                }
            ),
            [Message::Selftest {
                payload: vec![1, 2]
            }]
        );
        assert_eq!(answer(&ctx, &Message::EngineStopping), []);
        assert_eq!(answer(&ctx, &Message::Resync { from_id: 3 }), []);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_are_validated_then_saved_or_refused_with_the_problems() {
        let dir = dir("settings");
        let ctx = ctx(&dir);
        let good = "version = 1\n[general]\ntheme = \"dark\"\n";
        let saved = answer(
            &ctx,
            &Message::Command(Command::ApplySettings {
                toml: good.to_owned(),
            }),
        );
        assert!(matches!(
            saved.as_slice(),
            [Message::CommandResult { ok: true, .. }]
        ));
        assert_eq!(std::fs::read_to_string(&ctx.settings_path).unwrap(), good);

        let bad = "version = 1\n[general]\nautostrat = true\n";
        let refused = answer(
            &ctx,
            &Message::Command(Command::ApplySettings {
                toml: bad.to_owned(),
            }),
        );
        match refused.as_slice() {
            [Message::CommandResult { ok: false, message }] => {
                assert!(message.contains("did you mean `autostart`"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        // The refused text did not replace the good file.
        assert_eq!(std::fs::read_to_string(&ctx.settings_path).unwrap(), good);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn commands_that_need_later_work_say_so() {
        let dir = dir("commands");
        let ctx = ctx(&dir);
        let out = answer(&ctx, &Message::Command(Command::PauseAll));
        assert!(matches!(
            out.as_slice(),
            [Message::CommandResult { ok: false, .. }]
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipe_names_are_per_user_or_per_test_directory() {
        let user = pipe_name(None).unwrap();
        assert!(user.starts_with(r"\\.\pipe\PeekBuddy-"));
        let a = pipe_name(Some(std::path::Path::new(r"C:\T\A"))).unwrap();
        assert_eq!(a, pipe_name(Some(std::path::Path::new(r"c:\t\a"))).unwrap());
        assert_ne!(a, pipe_name(Some(std::path::Path::new(r"C:\T\B"))).unwrap());
        assert!(a.starts_with(r"\\.\pipe\PeekBuddy-test-"));
    }
}
