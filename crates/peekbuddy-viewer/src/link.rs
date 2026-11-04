//! The viewer's connection to the engine (#65).
//!
//! One thread owns the pipe. It connects, says hello, and then serves
//! commands one at a time (the pipe is a synchronous handle: a read that
//! waits would block every write, so the thread never waits in a read when
//! it is idle). While idle it asks the connection whether the engine is
//! still there every [`Timing::tick`], which costs the viewer a wakeup and
//! the engine nothing.
//!
//! The UI never touches the pipe. It sends commands with [`Link::send`] and
//! reads what happened with [`Link::poll`]; every event wakes the UI through
//! the `wake` callback (`Context::request_repaint`), so an idle viewer paints
//! nothing. A state is reported once per change, never once per retry.
//!
//! # States
//!
//! `Connecting` until the first attempt ends; `Connected` after a matching
//! hello; `Mismatch` when the engine speaks another protocol version (an old
//! engine still running after an update), which is final until the user asks
//! to reconnect; `Gone` when there is no engine, with a retry every
//! [`Timing::retry`]. Browsing the log works in every state: the viewer
//! reads the segments on its own.

use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use peekbuddy_core::ipc::{Command, Message, PROTOCOL_VERSION};

/// How the link thread paces itself.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How often an idle connection is checked for a dead engine.
    pub tick: Duration,
    /// How often a missing engine is tried again.
    pub retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            tick: Duration::from_millis(500),
            retry: Duration::from_secs(1),
        }
    }
}

/// Where the link stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    /// The first connection attempt is running.
    Connecting,
    /// Hello was answered with the same protocol version.
    Connected {
        /// The engine's build string.
        build: String,
    },
    /// The engine speaks another protocol version.
    Mismatch {
        /// The engine's protocol version.
        engine_version: u32,
    },
    /// No engine answers.
    Gone,
}

impl LinkState {
    /// Whether commands can be sent.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// What the link thread reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// The state changed.
    State(LinkState),
    /// The answer to the command sent with `tag`.
    Result {
        /// The tag [`Link::send`] returned.
        tag: u64,
        /// Whether the engine applied it.
        ok: bool,
        /// The engine's explanation, or why there was no answer.
        message: String,
    },
    /// A message the engine sent on its own (counters, watch state).
    Pushed(Message),
}

/// An open connection to an engine.
pub trait Connection: Send {
    /// Sends one message.
    ///
    /// # Errors
    ///
    /// The I/O error; the engine is then treated as gone.
    fn send(&mut self, message: &Message) -> io::Result<()>;

    /// Reads the next message; `None` when the engine closed the pipe.
    ///
    /// # Errors
    ///
    /// The I/O error.
    fn receive(&mut self) -> io::Result<Option<Message>>;

    /// How many bytes wait to be read, without blocking.
    ///
    /// # Errors
    ///
    /// An error once the engine is gone.
    fn available(&self) -> io::Result<u32>;
}

/// Opens connections to the engine.
pub trait Connector: Send + 'static {
    /// The connection type.
    type Conn: Connection;

    /// Tries once to connect, without waiting for the engine to appear.
    ///
    /// # Errors
    ///
    /// The I/O error when there is no engine to connect to.
    fn connect(&mut self) -> io::Result<Self::Conn>;
}

enum Control {
    Request { tag: u64, command: Command },
    Reconnect,
}

/// The UI's end of the connection.
#[derive(Debug)]
pub struct Link {
    control: Sender<Control>,
    events: Receiver<LinkEvent>,
    next_tag: u64,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    /// Starts the link thread. `wake` runs after every event.
    ///
    /// # Errors
    ///
    /// The error from spawning the thread.
    pub fn start<C: Connector>(
        connector: C,
        timing: Timing,
        wake: Arc<dyn Fn() + Send + Sync>,
        build: String,
    ) -> io::Result<Self> {
        let (control, requests) = mpsc::channel();
        let (sender, events) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(String::from("engine-link"))
            .spawn(move || {
                let mut out = Emitter {
                    events: sender,
                    wake,
                    last: None,
                };
                run(connector, timing, &requests, &mut out, &build);
            })?;
        Ok(Self {
            control,
            events,
            next_tag: 1,
            thread: Some(thread),
        })
    }

    /// Sends `command`; the answer arrives as a [`LinkEvent::Result`] with the
    /// returned tag. Without an engine the answer is a failure, immediately.
    pub fn send(&mut self, command: Command) -> u64 {
        let tag = self.next_tag;
        self.next_tag += 1;
        // A dead thread cannot answer; the UI shows the link state anyway.
        let _ = self.control.send(Control::Request { tag, command });
        tag
    }

    /// Tries to connect again now: after "Start engine", or after the user
    /// restarted PeekBuddy following a version mismatch.
    pub fn reconnect(&self) {
        let _ = self.control.send(Control::Reconnect);
    }

    /// The next event, if one is waiting.
    #[must_use]
    pub fn poll(&self) -> Option<LinkEvent> {
        self.events.try_recv().ok()
    }

    /// Waits up to `timeout` for the result of `tag`; other events that
    /// arrive meanwhile are returned too, in order. Used on close, where the
    /// settings must reach the engine before the process ends.
    #[must_use]
    pub fn wait_result(&self, tag: u64, timeout: Duration) -> Option<(bool, String)> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.events.recv_timeout(left) {
                Ok(LinkEvent::Result {
                    tag: got,
                    ok,
                    message,
                }) if got == tag => return Some((ok, message)),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    /// Ends the thread and waits for it (tests; the process ending is enough
    /// in production).
    pub fn shutdown(mut self) {
        let thread = self.thread.take();
        drop(self);
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

/// Sends events and wakes the UI; reports a state only when it changes.
struct Emitter {
    events: Sender<LinkEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    last: Option<LinkState>,
}

impl Emitter {
    fn emit(&self, event: LinkEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }

    fn state(&mut self, state: LinkState) {
        if self.last.as_ref() != Some(&state) {
            self.last = Some(state.clone());
            self.emit(LinkEvent::State(state));
        }
    }

    fn result(&self, tag: u64, ok: bool, message: impl Into<String>) {
        self.emit(LinkEvent::Result {
            tag,
            ok,
            message: message.into(),
        });
    }
}

const NO_ENGINE: &str = "PeekBuddy's engine is not running";

fn hello(build: &str) -> Message {
    Message::Hello {
        protocol_version: PROTOCOL_VERSION,
        build: build.to_owned(),
    }
}

enum Handshake {
    Ready(String),
    Mismatch(u32),
    Failed,
}

fn handshake<C: Connection>(conn: &mut C, build: &str) -> Handshake {
    if conn.send(&hello(build)).is_err() {
        return Handshake::Failed;
    }
    match conn.receive() {
        Ok(Some(Message::Hello {
            protocol_version,
            build,
        })) if protocol_version == PROTOCOL_VERSION => Handshake::Ready(build),
        Ok(Some(Message::VersionMismatch { engine_version })) => {
            Handshake::Mismatch(engine_version)
        }
        Ok(Some(Message::Hello {
            protocol_version, ..
        })) => Handshake::Mismatch(protocol_version),
        _ => Handshake::Failed,
    }
}

/// Why `serve` ended.
enum Served {
    /// The engine went away.
    Lost,
    /// The UI dropped the link.
    Dropped,
}

/// Sends one command and reads until its result. `false` when the engine
/// was lost on the way.
fn exchange<C: Connection>(conn: &mut C, tag: u64, command: Command, out: &mut Emitter) -> bool {
    if conn.send(&Message::Command(command)).is_err() {
        out.result(tag, false, "the engine did not take the command");
        return false;
    }
    loop {
        match conn.receive() {
            Ok(Some(Message::CommandResult { ok, message })) => {
                out.result(tag, ok, message);
                return true;
            }
            Ok(Some(Message::EngineStopping) | None) | Err(_) => {
                out.result(tag, false, "the engine stopped before it answered");
                return false;
            }
            Ok(Some(other)) => out.emit(LinkEvent::Pushed(other)),
        }
    }
}

fn serve<C: Connection>(
    conn: &mut C,
    timing: Timing,
    requests: &Receiver<Control>,
    out: &mut Emitter,
) -> Served {
    loop {
        match requests.recv_timeout(timing.tick) {
            Ok(Control::Request { tag, command }) => {
                if !exchange(conn, tag, command, out) {
                    return Served::Lost;
                }
            }
            Ok(Control::Reconnect) => {}
            Err(RecvTimeoutError::Timeout) => match conn.available() {
                Err(_) => return Served::Lost,
                Ok(0) => {}
                Ok(_) => match conn.receive() {
                    Ok(Some(Message::EngineStopping) | None) | Err(_) => return Served::Lost,
                    Ok(Some(other)) => out.emit(LinkEvent::Pushed(other)),
                },
            },
            Err(RecvTimeoutError::Disconnected) => return Served::Dropped,
        }
    }
}

/// Waits `wait` for the UI, answering requests with a failure. Returns
/// `false` when the UI dropped the link.
fn idle(requests: &Receiver<Control>, wait: Option<Duration>, out: &Emitter) -> bool {
    loop {
        let next = match wait {
            Some(wait) => requests.recv_timeout(wait),
            None => requests.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match next {
            Ok(Control::Request { tag, .. }) => out.result(tag, false, NO_ENGINE),
            Ok(Control::Reconnect) | Err(RecvTimeoutError::Timeout) => return true,
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn run<C: Connector>(
    mut connector: C,
    timing: Timing,
    requests: &Receiver<Control>,
    out: &mut Emitter,
    build: &str,
) {
    out.state(LinkState::Connecting);
    loop {
        let keep_going = match connector.connect() {
            Err(_) => {
                out.state(LinkState::Gone);
                idle(requests, Some(timing.retry), out)
            }
            Ok(mut conn) => match handshake(&mut conn, build) {
                Handshake::Ready(engine_build) => {
                    out.state(LinkState::Connected {
                        build: engine_build,
                    });
                    match serve(&mut conn, timing, requests, out) {
                        Served::Dropped => return,
                        Served::Lost => {
                            out.state(LinkState::Gone);
                            idle(requests, Some(timing.retry), out)
                        }
                    }
                }
                Handshake::Mismatch(engine_version) => {
                    out.state(LinkState::Mismatch { engine_version });
                    // Final until the user restarts PeekBuddy and asks again.
                    idle(requests, None, out)
                }
                Handshake::Failed => {
                    out.state(LinkState::Gone);
                    idle(requests, Some(timing.retry), out)
                }
            },
        };
        if !keep_going {
            return;
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! An engine that lives in memory, for the link and for the UI tests.

    use std::collections::VecDeque;
    use std::io;
    use std::sync::{Arc, Mutex, PoisonError};

    use peekbuddy_core::ipc::{Message, PROTOCOL_VERSION};

    use super::{Connection, Connector};

    /// What the fake engine does.
    #[derive(Debug)]
    pub struct State {
        /// Whether the engine runs.
        pub up: bool,
        /// The protocol version it answers hello with.
        pub version: u32,
        /// What the client will read next.
        pub inbox: VecDeque<Message>,
        /// Everything the client sent.
        pub sent: Vec<Message>,
        /// Connections made.
        pub connects: u32,
    }

    /// A handle to the fake engine.
    #[derive(Debug, Clone)]
    pub struct Engine(pub Arc<Mutex<State>>);

    impl Engine {
        pub fn new(up: bool) -> Self {
            Self(Arc::new(Mutex::new(State {
                up,
                version: PROTOCOL_VERSION,
                inbox: VecDeque::new(),
                sent: Vec::new(),
                connects: 0,
            })))
        }

        pub fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
            f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
        }
    }

    pub struct Conn(Engine);

    impl Connector for Engine {
        type Conn = Conn;

        fn connect(&mut self) -> io::Result<Conn> {
            self.with(|state| {
                if !state.up {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
                state.connects += 1;
                state.inbox.clear();
                Ok(())
            })?;
            Ok(Conn(self.clone()))
        }
    }

    impl Connection for Conn {
        fn send(&mut self, message: &Message) -> io::Result<()> {
            self.0.with(|state| {
                if !state.up {
                    return Err(io::Error::from(io::ErrorKind::BrokenPipe));
                }
                state.sent.push(message.clone());
                let reply = match message {
                    Message::Hello { .. } if state.version == PROTOCOL_VERSION => {
                        Some(Message::Hello {
                            protocol_version: PROTOCOL_VERSION,
                            build: String::from("fake 1"),
                        })
                    }
                    Message::Hello { .. } => Some(Message::VersionMismatch {
                        engine_version: state.version,
                    }),
                    Message::Command(_) => Some(Message::CommandResult {
                        ok: true,
                        message: String::from("done"),
                    }),
                    _ => None,
                };
                state.inbox.extend(reply);
                Ok(())
            })
        }

        fn receive(&mut self) -> io::Result<Option<Message>> {
            self.0.with(|state| {
                if !state.up {
                    return Ok(None);
                }
                Ok(state.inbox.pop_front())
            })
        }

        fn available(&self) -> io::Result<u32> {
            self.0.with(|state| {
                if state.up {
                    Ok(u32::try_from(state.inbox.len()).unwrap_or(u32::MAX))
                } else {
                    Err(io::Error::from(io::ErrorKind::BrokenPipe))
                }
            })
        }
    }
}

#[cfg(test)]
#[expect(clippy::panic, reason = "a test panics on an unexpected event")]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use peekbuddy_core::ipc::{Command, Message, PROTOCOL_VERSION};

    use super::fake::Engine;
    use super::{Link, LinkEvent, LinkState, Timing};

    const FAST: Timing = Timing {
        tick: Duration::from_millis(5),
        retry: Duration::from_millis(5),
    };

    fn start(engine: &Engine) -> (Link, Arc<AtomicU32>) {
        let wakes = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&wakes);
        let link = Link::start(
            engine.clone(),
            FAST,
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            String::from("viewer test"),
        )
        .unwrap();
        (link, wakes)
    }

    fn next(link: &Link) -> LinkEvent {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = link.poll() {
                return event;
            }
            assert!(Instant::now() < deadline, "no event within 5 s");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn state(link: &Link) -> LinkState {
        match next(link) {
            LinkEvent::State(state) => state,
            other => panic!("expected a state, got {other:?}"),
        }
    }

    #[test]
    fn a_running_engine_connects_after_connecting() {
        let engine = Engine::new(true);
        let (link, wakes) = start(&engine);
        assert_eq!(state(&link), LinkState::Connecting);
        assert_eq!(
            state(&link),
            LinkState::Connected {
                build: String::from("fake 1")
            }
        );
        assert_eq!(wakes.load(Ordering::SeqCst), 2, "each event woke the UI");
        engine.with(|e| {
            assert!(matches!(
                e.sent.first(),
                Some(Message::Hello { protocol_version, build })
                    if *protocol_version == PROTOCOL_VERSION && build == "viewer test"
            ));
        });
        link.shutdown();
    }

    #[test]
    fn a_missing_engine_is_reported_once_and_found_when_it_starts() {
        let engine = Engine::new(false);
        let (link, wakes) = start(&engine);
        assert_eq!(state(&link), LinkState::Connecting);
        assert_eq!(state(&link), LinkState::Gone);
        // Many retries pass without another event: an idle viewer paints
        // nothing.
        std::thread::sleep(Duration::from_millis(80));
        assert!(link.poll().is_none());
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        engine.with(|e| e.up = true);
        link.reconnect();
        assert!(state(&link).is_connected());
        link.shutdown();
    }

    #[test]
    fn another_protocol_version_is_final_until_the_user_asks_again() {
        let engine = Engine::new(true);
        engine.with(|e| e.version = PROTOCOL_VERSION + 1);
        let (link, _) = start(&engine);
        assert_eq!(state(&link), LinkState::Connecting);
        assert_eq!(
            state(&link),
            LinkState::Mismatch {
                engine_version: PROTOCOL_VERSION + 1
            }
        );
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(engine.with(|e| e.connects), 1, "no retry on a mismatch");
        // The engine was updated and restarted: reconnect finds it.
        engine.with(|e| e.version = PROTOCOL_VERSION);
        link.reconnect();
        assert!(state(&link).is_connected());
        link.shutdown();
    }

    #[test]
    fn commands_get_their_results_and_fail_fast_without_an_engine() {
        let engine = Engine::new(true);
        let (mut link, _) = start(&engine);
        assert_eq!(state(&link), LinkState::Connecting);
        assert!(state(&link).is_connected());
        let tag = link.send(Command::PauseAll);
        assert_eq!(
            next(&link),
            LinkEvent::Result {
                tag,
                ok: true,
                message: String::from("done")
            }
        );
        engine.with(|e| {
            assert!(matches!(
                e.sent.last(),
                Some(Message::Command(Command::PauseAll))
            ));
        });
        let first = link.send(Command::ResumeAll);
        let second = link.send(Command::ResumeAll);
        assert_ne!(first, second, "every command has its own tag");
        link.shutdown();

        let (mut link, _) = start(&Engine::new(false));
        assert_eq!(state(&link), LinkState::Connecting);
        assert_eq!(state(&link), LinkState::Gone);
        let tag = link.send(Command::PauseAll);
        match next(&link) {
            LinkEvent::Result {
                tag: got,
                ok: false,
                message,
            } => {
                assert_eq!(got, tag);
                assert!(message.contains("not running"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        link.shutdown();
    }

    #[test]
    fn a_dying_engine_is_noticed_while_idle_and_found_again() {
        let engine = Engine::new(true);
        let (link, _) = start(&engine);
        assert_eq!(state(&link), LinkState::Connecting);
        assert!(state(&link).is_connected());
        engine.with(|e| e.up = false);
        assert_eq!(state(&link), LinkState::Gone);
        engine.with(|e| e.up = true);
        assert!(
            state(&link).is_connected(),
            "the retry finds the new engine"
        );
        assert_eq!(engine.with(|e| e.connects), 2);
        link.shutdown();
    }

    #[test]
    fn engine_stopping_and_pushed_messages_are_handled_while_idle() {
        let engine = Engine::new(true);
        let (link, _) = start(&engine);
        assert_eq!(state(&link), LinkState::Connecting);
        assert!(state(&link).is_connected());
        engine.with(|e| e.inbox.push_back(Message::Resync { from_id: 7 }));
        assert_eq!(
            next(&link),
            LinkEvent::Pushed(Message::Resync { from_id: 7 })
        );
        engine.with(|e| e.inbox.push_back(Message::EngineStopping));
        assert_eq!(state(&link), LinkState::Gone);
        link.shutdown();
    }

    #[test]
    fn wait_result_finds_its_tag_among_other_events() {
        let engine = Engine::new(true);
        let (mut link, _) = start(&engine);
        let tag = link.send(Command::PauseAll);
        assert_eq!(
            link.wait_result(tag, Duration::from_secs(5)),
            Some((true, String::from("done")))
        );
        assert_eq!(link.wait_result(tag + 100, Duration::from_millis(20)), None);
        link.shutdown();
    }
}
