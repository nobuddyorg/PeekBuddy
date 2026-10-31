//! Engine <-> viewer protocol (#61): frames and messages, without any Win32.
//!
//! A frame is `u32 LE length | u16 LE type | payload`, where `length` counts
//! the type and the payload (so at least 2) and a whole frame never exceeds
//! [`MAX_FRAME`] bytes. The layout is fixed little-endian and hand-written so
//! it can be fuzzed on Linux and reviewed in one place; the history of the
//! protocol is in `docs/reference/ipc.md`.

use std::fmt;

/// The protocol version spoken by this build.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest frame, length prefix included.
pub const MAX_FRAME: usize = 1 << 20;

/// Largest string or byte payload a field may carry.
const MAX_FIELD: usize = MAX_FRAME - 64;

/// Message type numbers. Gaps are reserved for the live tail (#52, #65) and
/// the segment retirement handshake (#54).
mod kind {
    pub const HELLO: u16 = 1;
    pub const VERSION_MISMATCH: u16 = 2;
    pub const SHOW_VIEWER: u16 = 3;
    pub const SHUTDOWN: u16 = 4;
    pub const ENGINE_STOPPING: u16 = 5;
    pub const SELFTEST: u16 = 6;
    pub const COUNTERS: u16 = 7;
    pub const WATCH_STATE: u16 = 8;
    pub const RESYNC: u16 = 9;
    pub const COMMAND: u16 = 10;
    pub const COMMAND_RESULT: u16 = 11;
}

/// Counters the engine reports (the same six as the ETW counter event).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Raw change records received.
    pub events_received: u64,
    /// Rows durably committed.
    pub rows_persisted: u64,
    /// Buffer overflows.
    pub overflows: u64,
    /// Gap rows written.
    pub gaps: u64,
    /// Highest ring depth seen.
    pub ring_depth_high_water: u64,
    /// Size of the last committed batch.
    pub last_batch_size: u64,
}

/// State of one watch, as the viewer shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStateCode {
    /// Armed and capturing.
    Live,
    /// Paused by the user.
    Paused,
    /// Capturing with an unresolved gap.
    Gap,
    /// The root is gone or unreachable.
    Lost,
}

impl WatchStateCode {
    const fn to_byte(self) -> u8 {
        match self {
            Self::Live => 0,
            Self::Paused => 1,
            Self::Gap => 2,
            Self::Lost => 3,
        }
    }

    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Live),
            1 => Some(Self::Paused),
            2 => Some(Self::Gap),
            3 => Some(Self::Lost),
            _ => None,
        }
    }
}

/// A command from the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Pause every watch.
    PauseAll,
    /// Resume every watch.
    ResumeAll,
    /// Pause or resume one watch by its stable id.
    SetWatchPaused {
        /// The watch id.
        id: String,
        /// The new state.
        paused: bool,
    },
    /// Add a watch on `path`.
    AddWatch {
        /// Absolute path.
        path: String,
    },
    /// Remove the watch `id`.
    RemoveWatch {
        /// The watch id.
        id: String,
    },
    /// Validate, apply and persist new settings (the text of `settings.toml`).
    ApplySettings {
        /// The whole file.
        toml: String,
    },
}

/// One protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// First message both ways.
    Hello {
        /// The sender's [`PROTOCOL_VERSION`].
        protocol_version: u32,
        /// The sender's build (package version and git SHA).
        build: String,
    },
    /// The engine's answer to a [`Message::Hello`] with another version: the
    /// viewer asks the user to restart PeekBuddy.
    VersionMismatch {
        /// The engine's protocol version.
        engine_version: u32,
    },
    /// Open or focus the viewer (the second instance forwards this).
    ShowViewer,
    /// Ask the engine to shut down.
    Shutdown,
    /// The engine is going away; clients should disconnect.
    EngineStopping,
    /// Echoed back unchanged; for tests.
    Selftest {
        /// Anything.
        payload: Vec<u8>,
    },
    /// Counter snapshot.
    Counters(Counters),
    /// A watch changed state.
    WatchState {
        /// Index in the settings.
        index: u32,
        /// The new state.
        state: WatchStateCode,
    },
    /// Live-tail frames were dropped; query the database from this row id.
    Resync {
        /// First row id the viewer may have missed.
        from_id: u64,
    },
    /// A command.
    Command(Command),
    /// The outcome of a command.
    CommandResult {
        /// Whether it was applied.
        ok: bool,
        /// Detail, such as the validation problems.
        message: String,
    },
}

/// Why bytes are not a valid frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The length prefix is below 2 or above [`MAX_FRAME`].
    BadLength(u32),
    /// An unknown message type.
    UnknownType(u16),
    /// The payload ended early or has bytes left over.
    BadPayload(&'static str),
    /// A string is not valid UTF-8.
    BadText,
    /// An enum byte outside its range.
    BadValue(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLength(length) => write!(f, "frame length {length} is out of range"),
            Self::UnknownType(kind) => write!(f, "unknown message type {kind}"),
            Self::BadPayload(why) => write!(f, "bad payload: {why}"),
            Self::BadText => write!(f, "text is not valid UTF-8"),
            Self::BadValue(what) => write!(f, "{what} has an invalid value"),
        }
    }
}

impl std::error::Error for DecodeError {}

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn bytes(&mut self, value: &[u8]) {
        // Fields are bounded by MAX_FIELD at encode time (see `encode`).
        self.u32(u32::try_from(value.len()).unwrap_or(u32::MAX));
        self.0.extend_from_slice(value);
    }
    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize, what: &'static str) -> Result<&'a [u8], DecodeError> {
        let (head, tail) = self
            .0
            .split_at_checked(count)
            .ok_or(DecodeError::BadPayload(what))?;
        self.0 = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        self.take(1, "u8")?
            .first()
            .copied()
            .ok_or(DecodeError::BadPayload("u8"))
    }
    fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], DecodeError> {
        self.take(N, what)?
            .try_into()
            .map_err(|_| DecodeError::BadPayload(what))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array("u32")?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array("u64")?))
    }
    fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let length = self.u32()? as usize;
        if length > MAX_FIELD {
            return Err(DecodeError::BadPayload("field too long"));
        }
        self.take(length, "field")
    }
    fn text(&mut self) -> Result<String, DecodeError> {
        std::str::from_utf8(self.bytes()?)
            .map(str::to_owned)
            .map_err(|_| DecodeError::BadText)
    }
    fn finish(self) -> Result<(), DecodeError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::BadPayload("trailing bytes"))
        }
    }
}

fn command_kind(command: &Command) -> u8 {
    match command {
        Command::PauseAll => 0,
        Command::ResumeAll => 1,
        Command::SetWatchPaused { .. } => 2,
        Command::AddWatch { .. } => 3,
        Command::RemoveWatch { .. } => 4,
        Command::ApplySettings { .. } => 5,
    }
}

/// Appends the frame for `message` to `out` (a reusable buffer: nothing is
/// allocated once it has grown). Returns false, and leaves `out` unchanged,
/// when the message would exceed [`MAX_FRAME`].
pub fn encode(message: &Message, out: &mut Vec<u8>) -> bool {
    let start = out.len();
    // Length and type are patched in once the payload size is known.
    out.extend_from_slice(&[0; 6]);
    let kind = {
        let mut w = Writer(out);
        match message {
            Message::Hello {
                protocol_version,
                build,
            } => {
                w.u32(*protocol_version);
                w.text(build);
                kind::HELLO
            }
            Message::VersionMismatch { engine_version } => {
                w.u32(*engine_version);
                kind::VERSION_MISMATCH
            }
            Message::ShowViewer => kind::SHOW_VIEWER,
            Message::Shutdown => kind::SHUTDOWN,
            Message::EngineStopping => kind::ENGINE_STOPPING,
            Message::Selftest { payload } => {
                w.bytes(payload);
                kind::SELFTEST
            }
            Message::Counters(c) => {
                for value in [
                    c.events_received,
                    c.rows_persisted,
                    c.overflows,
                    c.gaps,
                    c.ring_depth_high_water,
                    c.last_batch_size,
                ] {
                    w.u64(value);
                }
                kind::COUNTERS
            }
            Message::WatchState { index, state } => {
                w.u32(*index);
                w.u8(state.to_byte());
                kind::WATCH_STATE
            }
            Message::Resync { from_id } => {
                w.u64(*from_id);
                kind::RESYNC
            }
            Message::Command(command) => {
                w.u8(command_kind(command));
                match command {
                    Command::PauseAll | Command::ResumeAll => {}
                    Command::SetWatchPaused { id, paused } => {
                        w.text(id);
                        w.u8(u8::from(*paused));
                    }
                    Command::AddWatch { path } => w.text(path),
                    Command::RemoveWatch { id } => w.text(id),
                    Command::ApplySettings { toml } => w.text(toml),
                }
                kind::COMMAND
            }
            Message::CommandResult { ok, message } => {
                w.u8(u8::from(*ok));
                w.text(message);
                kind::COMMAND_RESULT
            }
        }
    };
    let payload_len = out.len() - start - 6;
    if 6 + payload_len > MAX_FRAME {
        out.truncate(start);
        return false;
    }
    let length = u32::try_from(2 + payload_len).unwrap_or(u32::MAX);
    if let Some(prefix) = out.get_mut(start..start + 4) {
        prefix.copy_from_slice(&length.to_le_bytes());
    }
    if let Some(type_field) = out.get_mut(start + 4..start + 6) {
        type_field.copy_from_slice(&kind.to_le_bytes());
    }
    true
}

fn decode_command(r: &mut Reader<'_>) -> Result<Command, DecodeError> {
    Ok(match r.u8()? {
        0 => Command::PauseAll,
        1 => Command::ResumeAll,
        2 => Command::SetWatchPaused {
            id: r.text()?,
            paused: match r.u8()? {
                0 => false,
                1 => true,
                _ => return Err(DecodeError::BadValue("paused")),
            },
        },
        3 => Command::AddWatch { path: r.text()? },
        4 => Command::RemoveWatch { id: r.text()? },
        5 => Command::ApplySettings { toml: r.text()? },
        _ => return Err(DecodeError::BadValue("command")),
    })
}

fn decode_payload(kind: u16, payload: &[u8]) -> Result<Message, DecodeError> {
    let mut r = Reader(payload);
    let message = match kind {
        kind::HELLO => Message::Hello {
            protocol_version: r.u32()?,
            build: r.text()?,
        },
        kind::VERSION_MISMATCH => Message::VersionMismatch {
            engine_version: r.u32()?,
        },
        kind::SHOW_VIEWER => Message::ShowViewer,
        kind::SHUTDOWN => Message::Shutdown,
        kind::ENGINE_STOPPING => Message::EngineStopping,
        kind::SELFTEST => Message::Selftest {
            payload: r.bytes()?.to_vec(),
        },
        kind::COUNTERS => Message::Counters(Counters {
            events_received: r.u64()?,
            rows_persisted: r.u64()?,
            overflows: r.u64()?,
            gaps: r.u64()?,
            ring_depth_high_water: r.u64()?,
            last_batch_size: r.u64()?,
        }),
        kind::WATCH_STATE => Message::WatchState {
            index: r.u32()?,
            state: WatchStateCode::from_byte(r.u8()?).ok_or(DecodeError::BadValue("state"))?,
        },
        kind::RESYNC => Message::Resync { from_id: r.u64()? },
        kind::COMMAND => Message::Command(decode_command(&mut r)?),
        kind::COMMAND_RESULT => Message::CommandResult {
            ok: match r.u8()? {
                0 => false,
                1 => true,
                _ => return Err(DecodeError::BadValue("ok")),
            },
            message: r.text()?,
        },
        other => return Err(DecodeError::UnknownType(other)),
    };
    r.finish()?;
    Ok(message)
}

/// Decodes the first frame of `buf`.
///
/// Returns `Ok(None)` when more bytes are needed, and the message with the
/// number of bytes it used otherwise. An oversize or undersize length prefix
/// is refused from the first four bytes, before any payload arrives.
///
/// # Errors
///
/// A [`DecodeError`]; the connection should be closed.
pub fn decode(buf: &[u8]) -> Result<Option<(Message, usize)>, DecodeError> {
    let Some(prefix) = buf.first_chunk::<4>() else {
        return Ok(None);
    };
    let length = u32::from_le_bytes(*prefix);
    let body = length as usize;
    if !(2..=MAX_FRAME - 4).contains(&body) {
        return Err(DecodeError::BadLength(length));
    }
    let Some(frame) = buf.get(4..4 + body) else {
        return Ok(None);
    };
    let (kind, payload) = frame.split_at(2);
    let kind = u16::from_le_bytes([
        kind.first().copied().unwrap_or(0),
        kind.get(1).copied().unwrap_or(0),
    ]);
    Ok(Some((decode_payload(kind, payload)?, 4 + body)))
}

/// Collects bytes from a byte-mode pipe and yields whole messages.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Creates an empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes read from the pipe.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, if one has arrived.
    ///
    /// # Errors
    ///
    /// A [`DecodeError`]; the decoder is useless afterwards and the
    /// connection must be closed.
    pub fn next_message(&mut self) -> Result<Option<Message>, DecodeError> {
        match decode(&self.buf)? {
            Some((message, used)) => {
                self.buf.drain(..used);
                Ok(Some(message))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
#[expect(clippy::indexing_slicing, reason = "tests index frames")]
mod tests {
    use proptest::prelude::*;

    use super::{
        Command, Counters, DecodeError, FrameDecoder, MAX_FIELD, MAX_FRAME, Message,
        PROTOCOL_VERSION, Reader, WatchStateCode, decode, encode,
    };

    fn text() -> impl Strategy<Value = String> {
        "\\PC{0,40}"
    }

    fn command() -> impl Strategy<Value = Command> {
        prop_oneof![
            Just(Command::PauseAll),
            Just(Command::ResumeAll),
            (text(), any::<bool>()).prop_map(|(id, paused)| Command::SetWatchPaused { id, paused }),
            text().prop_map(|path| Command::AddWatch { path }),
            text().prop_map(|id| Command::RemoveWatch { id }),
            text().prop_map(|toml| Command::ApplySettings { toml }),
        ]
    }

    fn message() -> impl Strategy<Value = Message> {
        prop_oneof![
            (any::<u32>(), text()).prop_map(|(protocol_version, build)| Message::Hello {
                protocol_version,
                build
            }),
            any::<u32>().prop_map(|engine_version| Message::VersionMismatch { engine_version }),
            Just(Message::ShowViewer),
            Just(Message::Shutdown),
            Just(Message::EngineStopping),
            proptest::collection::vec(any::<u8>(), 0..64)
                .prop_map(|payload| Message::Selftest { payload }),
            proptest::array::uniform6(any::<u64>()).prop_map(|v| Message::Counters(Counters {
                events_received: v[0],
                rows_persisted: v[1],
                overflows: v[2],
                gaps: v[3],
                ring_depth_high_water: v[4],
                last_batch_size: v[5],
            })),
            (
                any::<u32>(),
                prop_oneof![
                    Just(WatchStateCode::Live),
                    Just(WatchStateCode::Paused),
                    Just(WatchStateCode::Gap),
                    Just(WatchStateCode::Lost)
                ]
            )
                .prop_map(|(index, state)| Message::WatchState { index, state }),
            any::<u64>().prop_map(|from_id| Message::Resync { from_id }),
            command().prop_map(Message::Command),
            (any::<bool>(), text())
                .prop_map(|(ok, message)| Message::CommandResult { ok, message }),
        ]
    }

    fn frame(message: &Message) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(encode(message, &mut out));
        out
    }

    proptest! {
        #[test]
        fn every_message_round_trips(message in message()) {
            let bytes = frame(&message);
            let (decoded, used) = decode(&bytes).unwrap().unwrap();
            prop_assert_eq!(decoded, message);
            prop_assert_eq!(used, bytes.len());
        }

        #[test]
        fn a_truncated_frame_asks_for_more_and_never_errors(message in message(), cut in any::<prop::sample::Index>()) {
            let bytes = frame(&message);
            let keep = cut.index(bytes.len());
            prop_assert_eq!(decode(&bytes[..keep]).unwrap(), None);
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
            let _ = decode(&bytes);
            let mut decoder = FrameDecoder::new();
            decoder.push(&bytes);
            let _ = decoder.next_message();
        }

        #[test]
        fn frames_split_anywhere_decode_the_same(a in message(), b in message(), split in any::<prop::sample::Index>()) {
            let mut all = frame(&a);
            all.extend(frame(&b));
            let at = split.index(all.len() + 1);
            let mut decoder = FrameDecoder::new();
            decoder.push(&all[..at]);
            let mut got = Vec::new();
            while let Some(m) = decoder.next_message().unwrap() {
                got.push(m);
            }
            decoder.push(&all[at..]);
            while let Some(m) = decoder.next_message().unwrap() {
                got.push(m);
            }
            prop_assert_eq!(got, vec![a, b]);
        }
    }

    #[test]
    fn the_layout_is_pinned() {
        let mut out = Vec::new();
        assert!(encode(
            &Message::Hello {
                protocol_version: 1,
                build: String::from("ab")
            },
            &mut out
        ));
        // length 12 = type (2) + u32 + u32 length + "ab"; type 1.
        assert_eq!(out, [12, 0, 0, 0, 1, 0, 1, 0, 0, 0, 2, 0, 0, 0, b'a', b'b']);
        let mut out = Vec::new();
        assert!(encode(&Message::ShowViewer, &mut out));
        assert_eq!(out, [2, 0, 0, 0, 3, 0]);
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn oversize_and_undersize_lengths_are_refused_before_the_payload_arrives() {
        let too_big = u32::try_from(MAX_FRAME).unwrap().to_le_bytes();
        assert_eq!(
            decode(&too_big),
            Err(DecodeError::BadLength(u32::try_from(MAX_FRAME).unwrap()))
        );
        assert_eq!(decode(&[0, 0, 0, 0]), Err(DecodeError::BadLength(0)));
        assert_eq!(decode(&[1, 0, 0, 0, 9]), Err(DecodeError::BadLength(1)));
        assert_eq!(
            decode(&[0xff, 0xff, 0xff, 0xff]),
            Err(DecodeError::BadLength(u32::MAX))
        );
    }

    #[test]
    fn bad_payloads_are_errors() {
        // Unknown type.
        assert_eq!(
            decode(&[2, 0, 0, 0, 99, 0]),
            Err(DecodeError::UnknownType(99))
        );
        // Trailing byte after ShowViewer.
        assert!(matches!(
            decode(&[3, 0, 0, 0, 3, 0, 7]),
            Err(DecodeError::BadPayload(_))
        ));
        // Hello with a text longer than the frame.
        assert!(matches!(
            decode(&[10, 0, 0, 0, 1, 0, 1, 0, 0, 0, 200, 0, 0, 0]),
            Err(DecodeError::BadPayload(_))
        ));
        // Invalid UTF-8 text.
        let mut hello = vec![11, 0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0xff];
        hello[0] = 11;
        assert_eq!(decode(&hello[..15]), Err(DecodeError::BadText));
        // Out-of-range enum.
        assert_eq!(
            decode(&[7, 0, 0, 0, 8, 0, 0, 0, 0, 0, 9]),
            Err(DecodeError::BadValue("state"))
        );
    }

    #[test]
    fn a_message_that_does_not_fit_is_refused_and_leaves_the_buffer_alone() {
        let mut out = vec![1, 2, 3];
        let huge = Message::Selftest {
            payload: vec![0; MAX_FRAME],
        };
        assert!(!encode(&huge, &mut out));
        assert_eq!(out, [1, 2, 3]);
        let just_fits = Message::Selftest {
            payload: vec![0; MAX_FRAME - 4 - 2 - 4],
        };
        assert!(encode(&just_fits, &mut out));
    }

    #[test]
    fn a_reused_buffer_is_not_reallocated() {
        let mut out = Vec::with_capacity(256);
        let pointer = out.as_ptr();
        for _ in 0..100 {
            out.clear();
            assert!(encode(&Message::Resync { from_id: 7 }, &mut out));
        }
        assert_eq!(out.as_ptr(), pointer);
    }
    #[test]
    fn decode_errors_say_what_is_wrong() {
        for (error, text) in [
            (DecodeError::BadLength(5), "frame length 5 is out of range"),
            (DecodeError::UnknownType(77), "unknown message type 77"),
            (DecodeError::BadPayload("u8"), "bad payload: u8"),
            (DecodeError::BadText, "text is not valid UTF-8"),
            (DecodeError::BadValue("state"), "state has an invalid value"),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    #[test]
    fn the_frame_length_limit_is_exact() {
        // The largest body the prefix may announce: the frame is not complete
        // yet, so the decoder asks for more.
        let body = u32::try_from(MAX_FRAME - 4).unwrap();
        assert_eq!(decode(&body.to_le_bytes()), Ok(None));
        let too_big = body + 1;
        assert_eq!(
            decode(&too_big.to_le_bytes()),
            Err(DecodeError::BadLength(too_big))
        );
        // The smallest: a type and nothing else.
        assert_eq!(decode(&1_u32.to_le_bytes()), Err(DecodeError::BadLength(1)));
        assert!(matches!(
            decode(&[2, 0, 0, 0, 0xFF, 0xFF]),
            Err(DecodeError::UnknownType(_))
        ));
    }

    #[test]
    fn a_field_may_be_as_long_as_the_limit_and_no_longer() {
        // A literal, so that a change of the constant is seen: one MiB less 64.
        let limit = 1_048_512;
        assert_eq!(MAX_FIELD, limit);
        let mut at_limit = u32::try_from(limit).unwrap().to_le_bytes().to_vec();
        at_limit.resize(4 + limit, b'x');
        assert_eq!(Reader(&at_limit).bytes().map(<[u8]>::len), Ok(limit));
        let mut over = u32::try_from(limit + 1).unwrap().to_le_bytes().to_vec();
        over.resize(4 + limit + 1, b'x');
        assert_eq!(
            Reader(&over).bytes(),
            Err(DecodeError::BadPayload("field too long"))
        );
    }
}
