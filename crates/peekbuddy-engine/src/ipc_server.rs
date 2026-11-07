//! The engine's side of the viewer protocol (#61).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use peekbuddy_core::ipc::{Command, Message, PROTOCOL_VERSION, WatchInfo};
use peekbuddy_core::settings::edit::{self, Refusal};
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
    /// This exe, spawned as the viewer.
    pub exe: PathBuf,
    /// The arguments that start the viewer ([`crate::instrument::viewer_args`]).
    pub viewer_args: Vec<std::ffi::OsString>,
    /// The settings file and the data directory it lives in.
    pub settings_path: PathBuf,
    /// Data directory text for validation.
    pub data_dir: String,
    /// Pause, resume and the watch list (#63, #68).
    pub pause: Arc<crate::pause_ctl::PauseCtl>,
    /// Markers (#82).
    pub markers: Arc<crate::marker_ctl::MarkerCtl>,
    /// Autostart at logon (#63); none for a test engine.
    pub autostart:
        Option<Arc<crate::autostart_ctl::AutostartCtl<peekbuddy_win::autostart::RunKeys>>>,
}

fn command_result(ok: bool, message: impl Into<String>) -> Message {
    Message::CommandResult {
        ok,
        message: message.into(),
    }
}

/// What the add-folder checks need to know about the machine: the data
/// directory and the exports folder inside it (F16).
fn edit_context(ctx: &Shared) -> Context {
    Context {
        data_dir: ctx.data_dir.clone(),
        export_dir: Some(
            Path::new(&ctx.data_dir)
                .join("exports")
                .to_string_lossy()
                .into_owned(),
        ),
    }
}

/// The form the file system resolves `path` to (long names, final path). A
/// folder that does not exist yet is resolved through its nearest existing
/// parent. `None` when no part of the path can be opened.
fn resolved(path: &str) -> Option<String> {
    let path = Path::new(path.trim());
    if let Some(units) = crate::capture_host::canonical_dirs(&[path])
        .into_iter()
        .next()
    {
        return String::from_utf16(&units).ok();
    }
    let base = resolved(&path.parent()?.to_string_lossy())?;
    Some(format!(
        "{}\\{}",
        base.trim_end_matches('\\'),
        path.file_name()?.to_string_lossy()
    ))
}

/// Refuses a folder that is the data or exports folder, or inside one, in
/// any spelling: the check of `PauseCtl::add_watch` compares the text as
/// given, so a short `RUNNER~1` name or a junction into the data folder would
/// pass it. Both sides are resolved here (F16).
fn refused_by_resolution(ctx: &Shared, path: &str) -> Option<Refusal> {
    let candidate = resolved(path.trim())?;
    let data = resolved(&ctx.data_dir).unwrap_or_else(|| ctx.data_dir.clone());
    let exports = Path::new(&ctx.data_dir).join("exports");
    let exports = resolved(&exports.to_string_lossy())
        .unwrap_or_else(|| exports.to_string_lossy().into_owned());
    edit::inside_own_folders(
        &candidate,
        &Context {
            data_dir: data,
            export_dir: Some(exports),
        },
    )
}

fn add_watch(ctx: &Shared, path: &str, confirmed: bool) -> Message {
    if let Some(refusal) = refused_by_resolution(ctx, path) {
        return command_result(false, refusal.to_string());
    }
    match ctx.pause.add_watch(path, confirmed, &edit_context(ctx)) {
        Ok(added) => {
            let mut message = format!(
                "added `{}`; it starts capturing with the next engine start",
                added.watch.id
            );
            for notice in &added.notices {
                message.push('\n');
                message.push_str(&notice.text());
            }
            command_result(true, message)
        }
        // The viewer asks the user and sends the same command again with
        // `confirmed` set.
        Err(crate::pause_ctl::EditError::Refused(Refusal::NeedsConfirmation(notice))) => {
            command_result(false, format!("confirm: {}", notice.text()))
        }
        Err(error) => command_result(false, error.to_string()),
    }
}

fn remove_watch(ctx: &Shared, id: &str) -> Message {
    match ctx.pause.remove_watch(id) {
        Ok(removed) => command_result(
            true,
            format!(
                "removed `{}`; the rows already recorded stay in the log",
                removed.watch.id
            ),
        ),
        Err(error) => command_result(false, error.to_string()),
    }
}

fn list_watches(ctx: &Shared) -> Vec<WatchInfo> {
    ctx.pause.list()
}

fn apply_settings(ctx: &Shared, text: &str) -> Message {
    let validation = Context {
        data_dir: ctx.data_dir.clone(),
        export_dir: None,
    };
    match settings::load(text, &validation) {
        Loaded::Ok { settings, .. } => match ctx.pause.apply_settings(&settings) {
            Ok(()) => {
                let mut message = String::from("saved");
                if let Some(ctl) = &ctx.autostart {
                    message.push_str("; ");
                    message.push_str(&ctl.apply(settings.general.autostart).map_or_else(
                        |error| error,
                        |status| crate::autostart_ctl::describe(&status),
                    ));
                }
                command_result(true, message)
            }
            Err(error) => command_result(false, error),
        },
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
            crate::instrument::spawn_viewer(&ctx.exe, &ctx.viewer_args);
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
        Message::Command(Command::MarkNow { name }) => {
            reply.send(&match ctx.markers.mark(name) {
                Ok(name) => command_result(true, format!("marked `{name}`")),
                Err(message) => command_result(false, message),
            });
        }
        Message::Command(Command::RenameMarker { event_id, name }) => {
            reply.send(&match ctx.markers.rename(*event_id, name) {
                Ok(name) => command_result(true, format!("renamed to `{name}`")),
                Err(message) => command_result(false, message),
            });
        }
        Message::Command(Command::PauseAll) => {
            #[cfg(feature = "inject")]
            crate::inject::leak_handle();
            ctx.pause.pause_all();
            reply.send(&command_result(true, "paused"));
        }
        Message::Command(Command::ResumeAll) => {
            #[cfg(feature = "inject")]
            crate::inject::leak_handle();
            ctx.pause.resume_all();
            reply.send(&command_result(true, "resumed"));
        }
        Message::Command(Command::SetWatchPaused { id, paused }) => {
            reply.send(&match ctx.pause.set_watch(id, *paused) {
                Ok(()) => command_result(true, if *paused { "paused" } else { "resumed" }),
                Err(message) => command_result(false, message),
            });
        }
        Message::Command(Command::AddWatch { path, confirmed }) => {
            reply.send(&add_watch(ctx, path, *confirmed));
        }
        Message::Command(Command::RemoveWatch { id }) => {
            reply.send(&remove_watch(ctx, id));
        }
        Message::ListWatches => {
            reply.send(&Message::WatchList(list_watches(ctx)));
        }
        // Messages the engine sends, not receives; ignored if a client
        // sends them.
        Message::VersionMismatch { .. }
        | Message::EngineStopping
        | Message::Counters(_)
        | Message::WatchList(_)
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
            viewer_args: Vec::new(),
            settings_path: dir.join("settings.toml"),
            data_dir: dir.to_string_lossy().into_owned(),
            autostart: None,
            markers: markers(dir),
            pause: crate::pause_ctl::PauseCtl::new(
                peekbuddy_core::settings::Settings {
                    watches: vec![peekbuddy_core::settings::Watch {
                        id: String::from("w1"),
                        path: String::from("C:/watched"),
                        ..peekbuddy_core::settings::Watch::default()
                    }],
                    ..peekbuddy_core::settings::Settings::default()
                },
                dir.join("settings.toml"),
                true,
                Box::new(Rows),
                crate::pause_ctl::now_ticks,
            ),
        }
    }

    /// A controller on a real pipeline over a scratch segment, in a folder
    /// next to `dir` (the pipeline keeps its files open for the process).
    fn markers(dir: &std::path::Path) -> std::sync::Arc<crate::marker_ctl::MarkerCtl> {
        let store = dir.with_extension("store");
        std::fs::create_dir_all(&store).unwrap();
        let writer = peekbuddy_store::writer::Writer::open(&store.join("events.db")).unwrap();
        let config = peekbuddy_store::pipeline::Config::new(store.join("spill"));
        let pipeline = peekbuddy_store::pipeline::Pipeline::start(writer, config).unwrap();
        let producer = pipeline.producer();
        std::mem::forget(pipeline);
        std::sync::Arc::new(crate::marker_ctl::MarkerCtl::new(
            producer,
            store,
            crate::pause_ctl::now_ticks,
        ))
    }

    struct Rows;

    impl crate::pause_ctl::RowSink for Rows {
        fn row(&mut self, _: peekbuddy_core::event::Event) {}
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
    fn marking_answers_with_the_cleaned_name_and_renaming_a_stranger_fails() {
        let dir = dir("marker");
        let ctx = ctx(&dir);
        let marked = answer(
            &ctx,
            &Message::Command(Command::MarkNow {
                name: String::from("  before install "),
            }),
        );
        assert_eq!(
            marked,
            [Message::CommandResult {
                ok: true,
                message: String::from("marked `before install`")
            }]
        );
        let blank = answer(
            &ctx,
            &Message::Command(Command::MarkNow {
                name: String::new(),
            }),
        );
        assert_eq!(
            blank,
            [Message::CommandResult {
                ok: true,
                message: String::from("marked `Marker`")
            }]
        );
        let renamed = answer(
            &ctx,
            &Message::Command(Command::RenameMarker {
                event_id: 12345,
                name: String::from("x"),
            }),
        );
        assert!(matches!(
            renamed.as_slice(),
            [Message::CommandResult { ok: false, .. }]
        ));
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
        // The file holds the new theme and keeps the watch of the engine; the
        // viewer's text never changes the watches.
        let after = std::fs::read_to_string(&ctx.settings_path).unwrap();
        assert!(after.contains("theme = \"dark\""), "{after}");
        assert!(after.contains("w1"), "{after}");

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
        assert_eq!(std::fs::read_to_string(&ctx.settings_path).unwrap(), after);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pause_and_resume_commands_reach_the_pause_control() {
        let dir = dir("commands");
        let ctx = ctx(&dir);
        let ok = |message: Message| {
            assert!(
                matches!(
                    answer(&ctx, &message).as_slice(),
                    [Message::CommandResult { ok: true, .. }]
                ),
                "{message:?}"
            );
        };
        ok(Message::Command(Command::PauseAll));
        assert!(ctx.pause.pause().global());
        ok(Message::Command(Command::ResumeAll));
        assert!(!ctx.pause.pause().global());
        ok(Message::Command(Command::SetWatchPaused {
            id: String::from("w1"),
            paused: true,
        }));
        assert!(
            ctx.pause
                .pause()
                .watch_paused(peekbuddy_core::event::RootIdx(0))
        );
        let unknown = answer(
            &ctx,
            &Message::Command(Command::SetWatchPaused {
                id: String::from("nope"),
                paused: true,
            }),
        );
        assert!(matches!(
            unknown.as_slice(),
            [Message::CommandResult { ok: false, .. }]
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn result(answers: &[Message]) -> (bool, String) {
        match answers {
            [Message::CommandResult { ok, message }] => (*ok, message.clone()),
            other => panic!("expected one command result, got {other:?}"),
        }
    }

    fn list(ctx: &Shared) -> Vec<peekbuddy_core::ipc::WatchInfo> {
        match answer(ctx, &Message::ListWatches).as_slice() {
            [Message::WatchList(watches)] => watches.clone(),
            other => panic!("expected a watch list, got {other:?}"),
        }
    }

    #[test]
    fn a_watch_is_added_listed_and_removed_and_the_file_follows() {
        let dir = dir("watches");
        let ctx = ctx(&dir);
        let listed = list(&ctx);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "w1");
        assert!(listed[0].armed);

        let (ok, message) = result(&answer(
            &ctx,
            &Message::Command(Command::AddWatch {
                path: String::from("C:/Reports/"),
                confirmed: false,
            }),
        ));
        assert!(ok, "{message}");
        assert!(message.contains("next engine start"), "{message}");
        let listed = list(&ctx);
        assert_eq!(listed.len(), 2);
        assert!(!listed[1].armed, "a new watch waits for the next start");
        assert_eq!(listed[1].path, r"\\?\C:\Reports");
        let file = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(file.contains("Reports"), "{file}");

        // The same folder twice is refused and changes nothing.
        let (ok, message) = result(&answer(
            &ctx,
            &Message::Command(Command::AddWatch {
                path: String::from(r"c:\reports"),
                confirmed: true,
            }),
        ));
        assert!(!ok);
        assert!(message.contains("already watched"), "{message}");
        assert_eq!(list(&ctx).len(), 2);

        // Remove the pending one, then the armed one.
        let new_id = listed[1].id.clone();
        let (ok, _) = result(&answer(
            &ctx,
            &Message::Command(Command::RemoveWatch { id: new_id }),
        ));
        assert!(ok);
        assert_eq!(list(&ctx).len(), 1);
        let (ok, message) = result(&answer(
            &ctx,
            &Message::Command(Command::RemoveWatch {
                id: String::from("w1"),
            }),
        ));
        assert!(ok, "{message}");
        assert_eq!(list(&ctx), []);
        let file = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(!file.contains("w1") && !file.contains("Reports"), "{file}");
        let (ok, message) = result(&answer(
            &ctx,
            &Message::Command(Command::RemoveWatch {
                id: String::from("w1"),
            }),
        ));
        assert!(!ok);
        assert!(message.contains("no watch"), "{message}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_data_directory_and_what_is_inside_it_are_refused() {
        let dir = dir("selfexclude");
        let ctx = ctx(&dir);
        for path in [
            dir.to_string_lossy().into_owned(),
            dir.join("segments").to_string_lossy().into_owned(),
            dir.join("exports").to_string_lossy().into_owned(),
        ] {
            let (ok, message) = result(&answer(
                &ctx,
                &Message::Command(Command::AddWatch {
                    path: path.clone(),
                    confirmed: true,
                }),
            ));
            assert!(!ok, "{path}");
            assert!(message.contains("data folder"), "{message}");
        }
        assert_eq!(list(&ctx).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_whole_drive_needs_a_second_command_with_the_confirmation() {
        let dir = dir("drive");
        let ctx = ctx(&dir);
        let add = |confirmed| {
            result(&answer(
                &ctx,
                &Message::Command(Command::AddWatch {
                    path: String::from(r"Z:\"),
                    confirmed,
                }),
            ))
        };
        let (ok, message) = add(false);
        assert!(!ok);
        assert!(message.starts_with("confirm: "), "{message}");
        assert!(message.contains("whole drive"), "{message}");
        assert_eq!(list(&ctx).len(), 1, "nothing changed");
        let (ok, message) = add(true);
        assert!(ok, "{message}");
        assert_eq!(list(&ctx).len(), 2);
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
