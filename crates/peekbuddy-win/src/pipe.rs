//! The engine's local named pipe (#61).
//!
//! `\\.\pipe\PeekBuddy-<hash of the user SID>` is created with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE` (a pipe someone else created first makes
//! startup fail closed instead of connecting to a squatter),
//! `PIPE_REJECT_REMOTE_CLIENTS` and a protected DACL that names the current
//! user and SYSTEM only. The server checks the SID of every client, the client
//! checks that the server runs the expected exe. Everything is overlapped and
//! waits on events, so an idle pipe costs no wakeups and a stuck client can
//! never hold up shutdown.

use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use peekbuddy_core::ipc::{self, FrameDecoder, Message};
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, FALSE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, INFINITE, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, ResetEvent, SetEvent,
    WaitForMultipleObjects,
};

/// Pipe instances the server keeps (a viewer, a second-instance forwarder).
const INSTANCES: u32 = 2;
/// Buffer size of each direction.
const BUFFER: u32 = 64 * 1024;
/// How long a write to a stuck client may take before it is dropped.
const WRITE_TIMEOUT_MS: u32 = 5_000;

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// FNV-1a: a stable, non-cryptographic hash for names.
#[must_use]
pub fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn sid_string_of(token: HANDLE) -> std::io::Result<String> {
    // TOKEN_USER and its SID; 8-byte aligned.
    let mut buffer = [0u64; 64];
    let mut needed = 0u32;
    let size = u32::try_from(size_of_val(&buffer)).unwrap_or(u32::MAX);
    // SAFETY: `buffer` is writable for `size` bytes; `needed` is a valid out
    // pointer; `token` is a live token handle with TOKEN_QUERY.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &raw mut needed,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    // SAFETY: on success the buffer starts with a TOKEN_USER.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` points into `buffer`, which is alive; `text` is an out
    // pointer for a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &raw mut text) } == 0 {
        return Err(last_error());
    }
    let mut length = 0usize;
    loop {
        // SAFETY: the string is NUL-terminated, so the walk ends at the NUL.
        let at = unsafe { text.add(length) };
        // SAFETY: `at` is inside the string, see above.
        if unsafe { at.read() } == 0 {
            break;
        }
        length += 1;
    }
    // SAFETY: `length` units were just walked.
    let units = unsafe { std::slice::from_raw_parts(text, length) };
    let result = String::from_utf16_lossy(units);
    // SAFETY: the string came from ConvertSidToStringSidW.
    unsafe { LocalFree(text.cast()) };
    Ok(result)
}

fn open_token(process: HANDLE) -> std::io::Result<OwnedHandle> {
    let mut raw: HANDLE = std::ptr::null_mut();
    // SAFETY: `raw` is a valid out pointer.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut raw) } == 0 {
        return Err(last_error());
    }
    // SAFETY: OpenProcessToken succeeded, so `raw` is an owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// The SID of the current user as a string, such as `S-1-5-21-...`.
///
/// # Errors
///
/// The OS error when the token cannot be read.
pub fn current_user_sid() -> std::io::Result<String> {
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    let token = open_token(process)?;
    sid_string_of(token.as_raw_handle())
}

/// The name of this user's pipe, `\\.\pipe\PeekBuddy-<16 hex digits>`.
///
/// # Errors
///
/// The OS error when the user SID cannot be read.
pub fn user_pipe_name() -> std::io::Result<String> {
    Ok(format!(
        r"\\.\pipe\PeekBuddy-{:016x}",
        stable_hash(&current_user_sid()?)
    ))
}

fn process_user_sid(pid: u32) -> std::io::Result<String> {
    // SAFETY: plain integer arguments.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if raw.is_null() {
        return Err(last_error());
    }
    // SAFETY: OpenProcess succeeded, so `raw` is an owned handle.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let token = open_token(process.as_raw_handle())?;
    sid_string_of(token.as_raw_handle())
}

fn process_image(pid: u32) -> std::io::Result<String> {
    // SAFETY: plain integer arguments.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if raw.is_null() {
        return Err(last_error());
    }
    // SAFETY: OpenProcess succeeded, so `raw` is an owned handle.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut buffer = [0u16; 1024];
    let mut size = 1024u32;
    // SAFETY: `buffer` holds `size` units; `size` is a valid in/out pointer.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle(),
            0,
            buffer.as_mut_ptr(),
            &raw mut size,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    let units = buffer.get(..size as usize).unwrap_or_default();
    Ok(String::from_utf16_lossy(units))
}

/// A security descriptor allowing the current user and SYSTEM only.
struct Dacl {
    descriptor: *mut core::ffi::c_void,
}

impl Dacl {
    fn for_current_user() -> std::io::Result<Self> {
        let sddl = wide(&format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", current_user_sid()?));
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; `descriptor` is an out pointer for
        // a LocalAlloc'd descriptor that `Drop` frees.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(Self { descriptor })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
            lpSecurityDescriptor: self.descriptor,
            bInheritHandle: 0,
        }
    }
}

impl Drop for Dacl {
    fn drop(&mut self) {
        // SAFETY: the descriptor came from the conversion above.
        unsafe { LocalFree(self.descriptor) };
    }
}

fn create_instance(name: &str, first: bool) -> std::io::Result<OwnedHandle> {
    let dacl = Dacl::for_current_user()?;
    let attributes = dacl.attributes();
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated; `attributes` points at a live
    // descriptor that is copied by the call.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            INSTANCES + 2,
            BUFFER,
            BUFFER,
            0,
            &raw const attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    // SAFETY: CreateNamedPipeW succeeded, so `handle` is an owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn new_event() -> std::io::Result<OwnedHandle> {
    // SAFETY: no attributes, manual-reset, initially unsignalled, unnamed.
    let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    if handle.is_null() {
        return Err(last_error());
    }
    // SAFETY: CreateEventW succeeded, so `handle` is an owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// Waits for `event` or `stop`. Returns `Some(true)` when `event` fired,
/// `Some(false)` when `stop` fired and `None` on timeout.
fn wait_two(event: &OwnedHandle, stop: &OwnedHandle, timeout_ms: u32) -> Option<bool> {
    let handles = [event.as_raw_handle(), stop.as_raw_handle()];
    // SAFETY: both handles are live; the count matches the array.
    let result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), FALSE, timeout_ms) };
    match result {
        r if r == WAIT_OBJECT_0 => Some(true),
        r if r == WAIT_OBJECT_0 + 1 => Some(false),
        r if r == WAIT_TIMEOUT => None,
        _ => Some(false),
    }
}

/// Replies queued by a handler for the connection it is serving.
#[derive(Debug, Default)]
pub struct Reply {
    bytes: Vec<u8>,
}

impl Reply {
    /// Queues `message` for the client. A message larger than a frame is
    /// dropped (and false returned).
    pub fn send(&mut self, message: &Message) -> bool {
        ipc::encode(message, &mut self.bytes)
    }

    /// The encoded replies queued so far.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Handles one message; may queue replies.
pub type Handler = dyn Fn(&Message, &mut Reply) + Send + Sync;

/// The pipe server: a few instances, each served by its own thread, one
/// client at a time.
pub struct PipeServer {
    stop: Arc<OwnedHandle>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for PipeServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipeServer")
            .field("threads", &self.threads.len())
            .finish_non_exhaustive()
    }
}

impl PipeServer {
    /// Creates the pipe `name` and starts serving.
    ///
    /// # Errors
    ///
    /// An error when the first instance cannot be created, for example
    /// because another process already owns the name (`ERROR_ACCESS_DENIED`):
    /// the caller must fail closed.
    pub fn start(name: &str, handler: &Arc<Handler>) -> std::io::Result<Self> {
        let first = create_instance(name, true)?;
        let mut instances = vec![first];
        for _ in 1..INSTANCES {
            instances.push(create_instance(name, false)?);
        }
        let stop = Arc::new(new_event()?);
        let mut threads = Vec::new();
        for instance in instances {
            let (stop, handler) = (Arc::clone(&stop), Arc::clone(handler));
            threads.push(
                std::thread::Builder::new()
                    .name(String::from("pipe"))
                    .spawn(move || serve_instance(&instance, &stop, &*handler))?,
            );
        }
        Ok(Self { stop, threads })
    }

    /// Stops serving: connected clients get `EngineStopping`, pending I/O is
    /// cancelled and the threads are joined.
    pub fn stop(&mut self) {
        // SAFETY: the event is live.
        unsafe { SetEvent(self.stop.as_raw_handle()) };
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn overlapped_with(event: &OwnedHandle) -> OVERLAPPED {
    OVERLAPPED {
        hEvent: event.as_raw_handle(),
        ..OVERLAPPED::default()
    }
}

/// Runs an overlapped call to completion, or cancels it when `stop` fires.
/// Returns the byte count, or `None` when stopped, timed out or failed.
fn finish(
    pipe: &OwnedHandle,
    started: bool,
    overlapped: &mut OVERLAPPED,
    event: &OwnedHandle,
    stop: &OwnedHandle,
    timeout_ms: u32,
) -> Option<u32> {
    if !started {
        // SAFETY: reading the thread's last error right after the call.
        if unsafe { GetLastError() } != ERROR_IO_PENDING {
            return None;
        }
        if wait_two(event, stop, timeout_ms) != Some(true) {
            // SAFETY: cancels the one request on this handle that uses
            // `overlapped`, which is still alive.
            unsafe { CancelIoEx(pipe.as_raw_handle(), overlapped) };
            let mut ignored = 0u32;
            // SAFETY: waits for the cancelled request to finish before the
            // buffers it uses are released.
            unsafe { GetOverlappedResult(pipe.as_raw_handle(), overlapped, &raw mut ignored, 1) };
            return None;
        }
    }
    let mut bytes = 0u32;
    // SAFETY: the request completed; `bytes` is a valid out pointer.
    let ok = unsafe { GetOverlappedResult(pipe.as_raw_handle(), overlapped, &raw mut bytes, 0) };
    (ok != 0).then_some(bytes)
}

fn write_all(
    pipe: &OwnedHandle,
    bytes: &[u8],
    event: &OwnedHandle,
    stop: &OwnedHandle,
    timeout_ms: u32,
) -> bool {
    let mut sent = 0usize;
    while sent < bytes.len() {
        let chunk = bytes.get(sent..).unwrap_or_default();
        let mut overlapped = overlapped_with(event);
        // SAFETY: the event is live.
        unsafe { ResetEvent(event.as_raw_handle()) };
        let length = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
        // SAFETY: `chunk` and `overlapped` outlive the request, which
        // `finish` always completes or cancels before returning.
        let started = unsafe {
            WriteFile(
                pipe.as_raw_handle(),
                chunk.as_ptr().cast(),
                length,
                std::ptr::null_mut(),
                &raw mut overlapped,
            )
        } != 0;
        match finish(pipe, started, &mut overlapped, event, stop, timeout_ms) {
            Some(0) | None => return false,
            Some(written) => sent += written as usize,
        }
    }
    true
}

fn serve_instance(pipe: &OwnedHandle, stop: &OwnedHandle, handler: &Handler) {
    let (Ok(connect_event), Ok(io_event)) = (new_event(), new_event()) else {
        return;
    };
    let own_sid = current_user_sid().unwrap_or_default();
    loop {
        // Wait for a client.
        let mut overlapped = overlapped_with(&connect_event);
        // SAFETY: the event is live.
        unsafe { ResetEvent(connect_event.as_raw_handle()) };
        // SAFETY: `overlapped` outlives the request, completed or cancelled
        // by `finish`.
        let connected = unsafe { ConnectNamedPipe(pipe.as_raw_handle(), &raw mut overlapped) } != 0;
        // SAFETY: reading the last error right after the call.
        let already = !connected && unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected
            && !already
            && finish(pipe, false, &mut overlapped, &connect_event, stop, INFINITE).is_none()
        {
            return;
        }
        if client_is_allowed(pipe, &own_sid) && !session(pipe, stop, &io_event, handler) {
            // Closing the handle (not DisconnectNamedPipe) keeps the farewell
            // readable for the client.
            return;
        }
        // SAFETY: disconnecting our own pipe instance to accept the next client.
        unsafe { DisconnectNamedPipe(pipe.as_raw_handle()) };
    }
}

fn client_is_allowed(pipe: &OwnedHandle, own_sid: &str) -> bool {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer.
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &raw mut pid) } == 0 {
        return false;
    }
    process_user_sid(pid).is_ok_and(|sid| sid == own_sid)
}

/// Serves one connected client. Returns false when the server is stopping.
fn session(pipe: &OwnedHandle, stop: &OwnedHandle, event: &OwnedHandle, handler: &Handler) -> bool {
    let mut decoder = FrameDecoder::new();
    let mut buffer = [0u8; 4096];
    loop {
        let mut overlapped = overlapped_with(event);
        // SAFETY: the event is live.
        unsafe { ResetEvent(event.as_raw_handle()) };
        // SAFETY: `buffer` and `overlapped` outlive the request, which
        // `finish` completes or cancels before returning.
        let started = unsafe {
            ReadFile(
                pipe.as_raw_handle(),
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len()).unwrap_or(0),
                std::ptr::null_mut(),
                &raw mut overlapped,
            )
        } != 0;
        // SAFETY: reading the last error right after the call.
        let broken = !started && unsafe { GetLastError() } == ERROR_BROKEN_PIPE;
        if broken {
            return true;
        }
        let Some(read) = finish(pipe, started, &mut overlapped, event, stop, INFINITE) else {
            // Stopped (the event fired) or the client went away. A closing
            // server tells the client, best effort.
            if wait_two(stop, stop, 0) == Some(true) {
                let mut bye = Vec::new();
                ipc::encode(&Message::EngineStopping, &mut bye);
                let io = new_event();
                if let Ok(io) = io {
                    let _ = write_all(pipe, &bye, &io, &io, 200);
                }
                return false;
            }
            return true;
        };
        if read == 0 {
            return true;
        }
        decoder.push(buffer.get(..read as usize).unwrap_or_default());
        let mut reply = Reply::default();
        loop {
            match decoder.next_message() {
                Ok(Some(message)) => handler(&message, &mut reply),
                Ok(None) => break,
                // A broken stream cannot be resynchronised: drop the client.
                Err(_) => return true,
            }
        }
        if !reply.bytes.is_empty() && !write_all(pipe, &reply.bytes, event, stop, WRITE_TIMEOUT_MS)
        {
            return true;
        }
    }
}

/// A connected client; blocking, one request at a time.
#[derive(Debug)]
pub struct PipeClient {
    file: std::fs::File,
    decoder: FrameDecoder,
}

impl PipeClient {
    /// Opens the pipe `name`, waiting up to `timeout` for the server to
    /// exist and for a free instance. With `expected_exe`, the server process
    /// must be running that exe (the pipe could be a squatter's).
    ///
    /// # Errors
    ///
    /// The OS error (`ERROR_ACCESS_DENIED` when the DACL refuses us), or an
    /// error naming the unexpected server image.
    pub fn connect(
        name: &str,
        expected_exe: Option<&Path>,
        timeout: Duration,
    ) -> std::io::Result<Self> {
        let deadline = Instant::now() + timeout;
        let wide_name = wide(name);
        let file = loop {
            let attempt = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)
                .open(name);
            match attempt {
                Ok(file) => break file,
                Err(error) => {
                    let code = error
                        .raw_os_error()
                        .and_then(|code| u32::try_from(code).ok());
                    let retry = code == Some(ERROR_PIPE_BUSY) || code == Some(ERROR_FILE_NOT_FOUND);
                    if !retry || Instant::now() >= deadline {
                        return Err(error);
                    }
                    if code == Some(ERROR_PIPE_BUSY) {
                        // SAFETY: `wide_name` is NUL-terminated.
                        unsafe { WaitNamedPipeW(wide_name.as_ptr(), 100) };
                    } else {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
            }
        };
        if let Some(expected) = expected_exe {
            let mut pid = 0u32;
            // SAFETY: `pid` is a valid out pointer; the handle is live.
            if unsafe { GetNamedPipeServerProcessId(file.as_raw_handle(), &raw mut pid) } == 0 {
                return Err(last_error());
            }
            let image = process_image(pid)?;
            if !image.eq_ignore_ascii_case(&expected.to_string_lossy()) {
                return Err(std::io::Error::other(format!(
                    "the pipe is served by {image}, not by {}",
                    expected.display()
                )));
            }
        }
        Ok(Self {
            file,
            decoder: FrameDecoder::new(),
        })
    }

    /// Sends one message.
    ///
    /// # Errors
    ///
    /// The I/O error, or `InvalidInput` for a message that exceeds a frame.
    pub fn send(&mut self, message: &Message) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut bytes = Vec::new();
        if !ipc::encode(message, &mut bytes) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "message too large",
            ));
        }
        self.file.write_all(&bytes)
    }

    /// Reads the next message; `None` when the server closed the pipe.
    ///
    /// # Errors
    ///
    /// The I/O error, or `InvalidData` for a broken frame.
    pub fn receive(&mut self) -> std::io::Result<Option<Message>> {
        use std::io::Read as _;
        let mut buffer = [0u8; 4096];
        loop {
            match self.decoder.next_message() {
                Ok(Some(message)) => return Ok(Some(message)),
                Ok(None) => {}
                Err(error) => {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error));
                }
            }
            let read = match self.file.read(&mut buffer) {
                Ok(read) => read,
                Err(error)
                    if error.raw_os_error()
                        == Some(i32::try_from(ERROR_BROKEN_PIPE).unwrap_or(109)) =>
                {
                    0
                }
                Err(error) => return Err(error),
            };
            if read == 0 {
                return Ok(None);
            }
            self.decoder.push(buffer.get(..read).unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use peekbuddy_core::ipc::Message;

    use super::{PipeClient, PipeServer, Reply, current_user_sid, stable_hash, user_pipe_name};

    fn name(tag: &str) -> String {
        format!(r"\\.\pipe\PeekBuddy-test-{tag}-{}", std::process::id())
    }

    fn echo() -> Arc<super::Handler> {
        Arc::new(|message: &Message, reply: &mut Reply| match message {
            Message::Selftest { payload } => {
                reply.send(&Message::Selftest {
                    payload: payload.clone(),
                });
            }
            Message::Shutdown => {
                reply.send(&Message::EngineStopping);
            }
            _ => {}
        })
    }

    #[test]
    fn the_pipe_name_is_stable_and_per_user() {
        let first = user_pipe_name().unwrap();
        assert_eq!(first, user_pipe_name().unwrap());
        assert!(first.starts_with(r"\\.\pipe\PeekBuddy-"));
        assert_eq!(first.len(), r"\\.\pipe\PeekBuddy-".len() + 16);
        assert!(current_user_sid().unwrap().starts_with("S-1-5-"));
        assert_eq!(stable_hash("a"), stable_hash("a"));
        assert_ne!(stable_hash("a"), stable_hash("b"));
    }

    #[test]
    fn a_client_of_the_same_user_gets_its_message_echoed() {
        let name = name("echo");
        let mut server = PipeServer::start(&name, &echo()).unwrap();
        let mut client = PipeClient::connect(&name, None, Duration::from_secs(5)).unwrap();
        for payload in [vec![], vec![1, 2, 3], vec![9; 20_000]] {
            client
                .send(&Message::Selftest {
                    payload: payload.clone(),
                })
                .unwrap();
            assert_eq!(
                client.receive().unwrap(),
                Some(Message::Selftest { payload })
            );
        }
        drop(client);
        server.stop();
    }

    #[test]
    fn two_clients_are_served_at_once_and_a_third_waits_its_turn() {
        let name = name("two");
        let mut server = PipeServer::start(&name, &echo()).unwrap();
        let mut a = PipeClient::connect(&name, None, Duration::from_secs(5)).unwrap();
        let mut b = PipeClient::connect(&name, None, Duration::from_secs(5)).unwrap();
        a.send(&Message::Selftest { payload: vec![1] }).unwrap();
        b.send(&Message::Selftest { payload: vec![2] }).unwrap();
        assert_eq!(
            a.receive().unwrap(),
            Some(Message::Selftest { payload: vec![1] })
        );
        assert_eq!(
            b.receive().unwrap(),
            Some(Message::Selftest { payload: vec![2] })
        );
        // Both instances are busy; a third client connects after one leaves.
        let waiting = std::thread::spawn({
            let name = name.clone();
            move || PipeClient::connect(&name, None, Duration::from_secs(10)).map(|_| ())
        });
        std::thread::sleep(Duration::from_millis(200));
        drop(a);
        waiting.join().unwrap().unwrap();
        drop(b);
        server.stop();
    }

    #[test]
    fn a_pipe_that_exists_makes_the_server_fail_closed() {
        let name = name("squat");
        let squatter = PipeServer::start(&name, &echo()).unwrap();
        let error = PipeServer::start(&name, &echo()).unwrap_err();
        // FILE_FLAG_FIRST_PIPE_INSTANCE: access denied (or already exists).
        assert!(
            matches!(error.raw_os_error(), Some(5 | 231 | 183)),
            "{error:?}"
        );
        drop(squatter);
    }

    #[test]
    fn the_client_refuses_a_server_running_another_exe() {
        let name = name("image");
        let mut server = PipeServer::start(&name, &echo()).unwrap();
        let this = std::env::current_exe().unwrap();
        assert!(PipeClient::connect(&name, Some(&this), Duration::from_secs(5)).is_ok());
        let other = std::path::Path::new(r"C:\Windows\System32\cmd.exe");
        let error = PipeClient::connect(&name, Some(other), Duration::from_secs(5)).unwrap_err();
        assert!(error.to_string().contains("not by"), "{error}");
        server.stop();
    }

    #[test]
    fn stopping_tells_a_connected_client_and_never_hangs() {
        let name = name("stop");
        let mut server = PipeServer::start(&name, &echo()).unwrap();
        let mut client = PipeClient::connect(&name, None, Duration::from_secs(5)).unwrap();
        client
            .send(&Message::Selftest { payload: vec![1] })
            .unwrap();
        assert!(client.receive().unwrap().is_some());
        let started = std::time::Instant::now();
        server.stop();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(client.receive().unwrap(), Some(Message::EngineStopping));
        assert_eq!(client.receive().unwrap(), None);
    }

    #[test]
    fn a_garbage_stream_drops_the_client_but_not_the_server() {
        use std::io::Write as _;
        let name = name("garbage");
        let mut server = PipeServer::start(&name, &echo()).unwrap();
        let mut raw = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .unwrap();
        raw.write_all(&[0xff, 0xff, 0xff, 0xff, 1, 2, 3]).unwrap();
        drop(raw);
        let mut client = PipeClient::connect(&name, None, Duration::from_secs(5)).unwrap();
        client
            .send(&Message::Selftest { payload: vec![7] })
            .unwrap();
        assert_eq!(
            client.receive().unwrap(),
            Some(Message::Selftest { payload: vec![7] })
        );
        server.stop();
    }
}
