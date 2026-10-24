//! Spike #31: unprivileged USN journal reading.
//!
//! `usn-probe battery --label L --volume X: --ok-dir DIR --out FILE`
//!   runs every handle variant x FSCTL, the name-withholding workload and the
//!   latency and overlapped/cancel cells in the current token, appending one
//!   JSON line per attempt to FILE.
//! `usn-probe restricted <battery args>` re-runs `battery` in a restricted
//!   token child (Administrators deny-only, no privileges, medium integrity).
//! `usn-probe user NAME PASSWORD <battery args>` re-runs it as another user.
//!
//! Handshake for the name-withholding cell: the probe creates `ready.flag` in
//! the ok directory after noting its start USN; the orchestrator (admin)
//! changes files the probe cannot read and creates `done.flag`.
#![cfg(windows)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, GetTokenInformation,
    SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_QUERY, TokenElevation, TokenElevationType, TokenIntegrityLevel,
    WinBuiltinAdministratorsSid, TOKEN_MANDATORY_LABEL,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ID_DESCRIPTOR, GetFinalPathNameByHandleW, OpenFileById, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Ioctl::{
    FSCTL_ENUM_USN_DATA, FSCTL_GET_NTFS_VOLUME_DATA, FSCTL_QUERY_USN_JOURNAL,
    FSCTL_READ_FILE_USN_DATA, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateProcessAsUserW, CreateProcessWithLogonW, GetCurrentProcess,
    GetCurrentProcessId, INFINITE, LOGON_WITH_PROFILE, OpenProcessToken, PROCESS_INFORMATION,
    STARTUPINFOW, WaitForSingleObject,
};
use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;

const GENERIC_READ: u32 = 0x8000_0000;
const FILE_TRAVERSE: u32 = 0x20;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn json_escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            c if c.is_control() => vec![' '],
            c => vec![c],
        })
        .collect()
}

struct Out {
    path: PathBuf,
    label: String,
    volume: String,
    fs: String,
    token: String,
}

impl Out {
    fn line(&self, cell: &str, fields: &[(&str, String)]) {
        let mut line = format!(
            "{{\"spike\":\"usn\",\"label\":\"{}\",\"volume\":\"{}\",\"fs\":\"{}\",\"token\":\"{}\",\"cell\":\"{}\"",
            json_escape(&self.label),
            json_escape(&self.volume),
            json_escape(&self.fs),
            json_escape(&self.token),
            json_escape(cell)
        );
        for (k, v) in fields {
            line.push_str(&format!(",\"{k}\":{v}"));
        }
        line.push('}');
        line.push('\n');
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(line.as_bytes());
        }
        print!("{line}");
    }
}

fn s(v: &str) -> String {
    format!("\"{}\"", json_escape(v))
}

fn open(path: &str, access: u32, flags: u32) -> Result<HANDLE, u32> {
    // SAFETY: the path is a NUL-terminated wide string that outlives the call.
    let h = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        // SAFETY: no preconditions.
        Err(unsafe { GetLastError() })
    } else {
        Ok(h)
    }
}

fn ioctl(h: HANDLE, code: u32, input: &[u8], out: &mut [u8]) -> Result<u32, u32> {
    let mut returned = 0u32;
    // SAFETY: both buffers are live for the call and their lengths are passed.
    let ok = unsafe {
        DeviceIoControl(
            h,
            code,
            if input.is_empty() { null() } else { input.as_ptr().cast::<c_void>() },
            u32::try_from(input.len()).unwrap_or(0),
            out.as_mut_ptr().cast::<c_void>(),
            u32::try_from(out.len()).unwrap_or(0),
            &raw mut returned,
            null_mut(),
        )
    };
    if ok == 0 {
        // SAFETY: no preconditions.
        Err(unsafe { GetLastError() })
    } else {
        Ok(returned)
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2).and_then(|x| x.try_into().ok()).map_or(0, u16::from_le_bytes)
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).and_then(|x| x.try_into().ok()).map_or(0, u32::from_le_bytes)
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 8).and_then(|x| x.try_into().ok()).map_or(0, u64::from_le_bytes)
}

#[derive(Clone, Debug)]
struct Rec {
    frn: u128,
    parent: u128,
    usn: i64,
    reason: u32,
    name: String,
}

/// Parses USN records after the leading 8-byte NextUsn.
fn parse_records(buf: &[u8]) -> (i64, Vec<Rec>) {
    let next = u64_at(buf, 0).cast_signed();
    let mut at = 8usize;
    let mut out = Vec::new();
    while at + 8 <= buf.len() {
        let len = u32_at(buf, at) as usize;
        if len < 8 || at + len > buf.len() {
            break;
        }
        let rec = buf.get(at..at + len).unwrap_or(&[]);
        let major = u16_at(rec, 4);
        let (frn, parent, usn, reason, name_len, name_off) = if major == 3 {
            let frn = u128::from_le_bytes(rec.get(8..24).and_then(|x| x.try_into().ok()).unwrap_or([0; 16]));
            let parent = u128::from_le_bytes(rec.get(24..40).and_then(|x| x.try_into().ok()).unwrap_or([0; 16]));
            (frn, parent, u64_at(rec, 40).cast_signed(), u32_at(rec, 56), usize::from(u16_at(rec, 72)), usize::from(u16_at(rec, 74)))
        } else {
            (u128::from(u64_at(rec, 8)), u128::from(u64_at(rec, 16)), u64_at(rec, 24).cast_signed(), u32_at(rec, 40), usize::from(u16_at(rec, 56)), usize::from(u16_at(rec, 58)))
        };
        let units: Vec<u16> = rec
            .get(name_off..name_off + name_len)
            .unwrap_or(&[])
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        out.push(Rec { frn, parent, usn, reason, name: String::from_utf16_lossy(&units) });
        at += len.next_multiple_of(8);
    }
    (next, out)
}

fn read_input(start: i64, journal_id: u64, wait_bytes: u64, timeout: u64) -> Vec<u8> {
    let mut b = vec![0u8; 48];
    b[0..8].copy_from_slice(&start.to_le_bytes());
    b[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    b[16..24].copy_from_slice(&timeout.to_le_bytes());
    b[24..32].copy_from_slice(&wait_bytes.to_le_bytes());
    b[32..40].copy_from_slice(&journal_id.to_le_bytes());
    b[40..42].copy_from_slice(&2u16.to_le_bytes());
    b[42..44].copy_from_slice(&3u16.to_le_bytes());
    b
}

fn token_summary() -> String {
    let mut token: HANDLE = null_mut();
    // SAFETY: out pointer is valid; the pseudo handle needs no close.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) };
    if ok == 0 {
        return "token-unavailable".into();
    }
    let query = |class: i32| -> Vec<u8> {
        let mut buf = vec![0u8; 256];
        let mut len = 0u32;
        // SAFETY: buffer and length are valid for the call.
        let ok = unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), 256, &raw mut len) };
        if ok == 0 { Vec::new() } else { buf }
    };
    let elevated = u32_at(&query(TokenElevation), 0);
    let kind = u32_at(&query(TokenElevationType), 0);
    let il = query(TokenIntegrityLevel);
    // TOKEN_MANDATORY_LABEL: a pointer to the SID, whose last sub-authority is the RID.
    let rid = {
        let ptr = u64_at(&il, 0) as usize;
        if ptr == 0 {
            0
        } else {
            // SAFETY: the pointer comes from GetTokenInformation into `il`, which is live; a SID is at least 8 bytes plus sub-authorities.
            let sid = unsafe { std::slice::from_raw_parts(ptr as *const u8, 12) };
            let count = usize::from(sid.get(1).copied().unwrap_or(0));
            let at = 8 + 4 * count.saturating_sub(1);
            // SAFETY: as above, reading the last sub-authority of the SID.
            let tail = unsafe { std::slice::from_raw_parts((ptr + at) as *const u8, 4) };
            u32_at(tail, 0)
        }
    };
    // SAFETY: token was opened above.
    unsafe { CloseHandle(token) };
    let mut name = [0u16; 256];
    let mut n = 256u32;
    // SAFETY: buffer and length are valid.
    unsafe { GetUserNameW(name.as_mut_ptr(), &raw mut n) };
    let user = String::from_utf16_lossy(&name[..usize::try_from(n).unwrap_or(1).saturating_sub(1).min(255)]);
    format!("user={user};elevated={elevated};type={kind};integrity={rid:#x}")
}

fn fs_name(root: &str) -> String {
    let mut name = [0u16; 64];
    // SAFETY: the root path is NUL-terminated, the buffer length is passed.
    let ok = unsafe {
        GetVolumeInformationW(wide(root).as_ptr(), null_mut(), 0, null_mut(), null_mut(), null_mut(), name.as_mut_ptr(), 64)
    };
    if ok == 0 {
        return "unknown".into();
    }
    let end = name.iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf16_lossy(&name[..end])
}

fn volume_guid(root: &str) -> Option<String> {
    let mut buf = [0u16; 64];
    // SAFETY: NUL-terminated input, output buffer length passed.
    let ok = unsafe { GetVolumeNameForVolumeMountPointW(wide(root).as_ptr(), buf.as_mut_ptr(), 64) };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..end]))
}

struct Args {
    label: String,
    volume: String,
    ok_dir: PathBuf,
    out: PathBuf,
    raw: Vec<String>,
}

fn parse_args(raw: &[String]) -> Args {
    let mut a = Args { label: "local".into(), volume: "C:".into(), ok_dir: PathBuf::from("."), out: PathBuf::from("usn-probe.jsonl"), raw: raw.to_vec() };
    let mut it = raw.iter();
    while let Some(flag) = it.next() {
        let value = it.next().cloned().unwrap_or_default();
        match flag.as_str() {
            "--label" => a.label = value,
            "--volume" => a.volume = value,
            "--ok-dir" => a.ok_dir = PathBuf::from(value),
            "--out" => a.out = PathBuf::from(value),
            _ => {}
        }
    }
    a
}

fn error_text(code: u32) -> String {
    format!("{code}")
}

/// One handle variant. Returns the handle or reports the open error.
fn open_variant(out: &Out, variant: &str, volume: &str, guid: Option<&str>, overlapped: bool) -> Option<HANDLE> {
    let flags = FILE_FLAG_BACKUP_SEMANTICS | if overlapped { FILE_FLAG_OVERLAPPED } else { 0 };
    let (path, access) = match variant {
        "guid-dir" => (guid?.to_owned(), GENERIC_READ),
        "traverse" => (format!("\\\\.\\{volume}"), FILE_TRAVERSE),
        "backup-root" => (format!("{volume}\\"), GENERIC_READ),
        _ => (format!("\\\\.\\{volume}"), GENERIC_READ),
    };
    match open(&path, access, flags) {
        Ok(h) => Some(h),
        Err(e) => {
            out.line("open", &[("handle", s(variant)), ("ok", "false".into()), ("error", error_text(e))]);
            None
        }
    }
}

const VARIANTS: [&str; 4] = ["guid-dir", "traverse", "backup-root", "device-read"];

fn battery(a: &Args) {
    let root = format!("{}\\", a.volume.trim_end_matches('\\'));
    let out = Out {
        path: a.out.clone(),
        label: a.label.clone(),
        volume: a.volume.clone(),
        fs: fs_name(&root),
        token: token_summary(),
    };
    let guid = volume_guid(&root);
    out.line("start", &[("guid", s(guid.as_deref().unwrap_or("none"))), ("pid", format!("{}", unsafe { GetCurrentProcessId() }))]);

    // The journal id and a start USN: a file's own record is the head of the journal.
    let probe_file = a.ok_dir.join("probe-start.txt");
    let _ = std::fs::write(&probe_file, b"x");
    let mut start_usn = 0i64;
    let mut journal_id = 0u64;
    if let Ok(h) = open(&probe_file.to_string_lossy(), GENERIC_READ, FILE_FLAG_BACKUP_SEMANTICS) {
        let mut buf = vec![0u8; 4096];
        let input = [2u8, 0, 3, 0];
        match ioctl(h, FSCTL_READ_FILE_USN_DATA, &input, &mut buf) {
            Ok(n) => {
                let (_, recs) = parse_records_with_prefix(&buf, n as usize);
                start_usn = recs.first().map_or(0, |r| r.usn);
                out.line("file-usn", &[("ok", "true".into()), ("usn", format!("{start_usn}")), ("name", s(recs.first().map_or("", |r| &r.name)))]);
            }
            Err(e) => out.line("file-usn", &[("ok", "false".into()), ("error", error_text(e))]),
        }
        // SAFETY: handle opened above.
        unsafe { CloseHandle(h) };
    }

    for variant in VARIANTS {
        let Some(h) = open_variant(&out, variant, &a.volume, guid.as_deref(), false) else { continue };
        out.line("open", &[("handle", s(variant)), ("ok", "true".into())]);
        let mut buf = vec![0u8; 65_536];
        match ioctl(h, FSCTL_QUERY_USN_JOURNAL, &[], &mut buf) {
            Ok(_) => {
                journal_id = u64_at(&buf, 0);
                out.line("query", &[("handle", s(variant)), ("ok", "true".into()), ("journal_id", format!("{journal_id}")), ("first", format!("{}", u64_at(&buf, 8).cast_signed())), ("next", format!("{}", u64_at(&buf, 16).cast_signed()))]);
            }
            Err(e) => out.line("query", &[("handle", s(variant)), ("ok", "false".into()), ("error", error_text(e))]),
        }
        match ioctl(h, FSCTL_GET_NTFS_VOLUME_DATA, &[], &mut buf) {
            Ok(_) => out.line("ntfs-data", &[("handle", s(variant)), ("ok", "true".into())]),
            Err(e) => out.line("ntfs-data", &[("handle", s(variant)), ("ok", "false".into()), ("error", error_text(e))]),
        }
        let mut enum_in = vec![0u8; 32];
        enum_in[16..24].copy_from_slice(&i64::MAX.to_le_bytes());
        enum_in[24..26].copy_from_slice(&2u16.to_le_bytes());
        enum_in[26..28].copy_from_slice(&3u16.to_le_bytes());
        match ioctl(h, FSCTL_ENUM_USN_DATA, &enum_in, &mut buf) {
            Ok(n) => out.line("enum", &[("handle", s(variant)), ("ok", "true".into()), ("bytes", format!("{n}"))]),
            Err(e) => out.line("enum", &[("handle", s(variant)), ("ok", "false".into()), ("error", error_text(e))]),
        }
        for (cell, code) in [("read", FSCTL_READ_USN_JOURNAL), ("read-unpriv", FSCTL_READ_UNPRIVILEGED_USN_JOURNAL)] {
            let _ = std::fs::write(a.ok_dir.join(format!("touch-{cell}.txt")), b"y");
            let input = read_input(start_usn, journal_id, 0, 0);
            match ioctl(h, code, &input, &mut buf) {
                Ok(n) => {
                    let (next, recs) = parse_records_with_prefix(&buf, n as usize);
                    let named = recs.iter().filter(|r| !r.name.is_empty()).count();
                    out.line(cell, &[("handle", s(variant)), ("ok", "true".into()), ("records", format!("{}", recs.len())), ("named", format!("{named}")), ("next_usn", format!("{next}")), ("bytes", format!("{n}"))]);
                }
                Err(e) => out.line(cell, &[("handle", s(variant)), ("ok", "false".into()), ("error", error_text(e))]),
            }
        }
        // SAFETY: handle opened above.
        unsafe { CloseHandle(h) };
    }

    // Pick the first handle variant that reads unprivileged for the workloads.
    let mut chosen = None;
    for variant in VARIANTS {
        if let Some(h) = open_variant(&Out { path: std::env::temp_dir().join("usn-probe-scratch.jsonl"), label: String::new(), volume: String::new(), fs: String::new(), token: String::new() }, variant, &a.volume, guid.as_deref(), false) {
            let mut buf = vec![0u8; 65_536];
            let ok = ioctl(h, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, &read_input(start_usn, journal_id, 0, 0), &mut buf).is_ok();
            // SAFETY: handle opened above.
            unsafe { CloseHandle(h) };
            if ok {
                chosen = Some(variant);
                break;
            }
        }
    }
    let Some(variant) = chosen else {
        out.line("verdict", &[("unprivileged_read", "false".into())]);
        return;
    };
    out.line("verdict", &[("unprivileged_read", "true".into()), ("handle", s(variant))]);
    name_withholding(&out, a, variant, guid.as_deref(), journal_id);
    latency(&out, a, variant, guid.as_deref(), journal_id);
    overlapped(&out, a, variant, guid.as_deref(), journal_id);
}

fn parse_records_with_prefix(buf: &[u8], n: usize) -> (i64, Vec<Rec>) {
    // FSCTL_READ_FILE_USN_DATA has no NextUsn prefix; the others do. A prefix
    // is present when the first u32 is not a plausible record length.
    let data = buf.get(..n).unwrap_or(&[]);
    let first = u32_at(data, 0) as usize;
    let major = u16_at(data, 4);
    if first == n && (major == 2 || major == 3) {
        let mut prefixed = vec![0u8; 8];
        prefixed.extend_from_slice(data);
        parse_records(&prefixed)
    } else {
        parse_records(data)
    }
}

fn wait_for(path: &Path, timeout: Duration) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn read_all(h: HANDLE, from: i64, journal_id: u64) -> (i64, Vec<Rec>, Option<u32>) {
    let mut all = Vec::new();
    let mut next = from;
    let mut buf = vec![0u8; 65_536];
    for _ in 0..1000 {
        match ioctl(h, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, &read_input(next, journal_id, 0, 0), &mut buf) {
            Ok(n) => {
                let (n_usn, recs) = parse_records_with_prefix(&buf, n as usize);
                if recs.is_empty() || n_usn == next {
                    return (next, all, None);
                }
                all.extend(recs);
                next = n_usn;
            }
            Err(e) => return (next, all, Some(e)),
        }
    }
    (next, all, None)
}

/// Q3: names for files the caller can and cannot read.
fn name_withholding(out: &Out, a: &Args, variant: &str, guid: Option<&str>, journal_id: u64) {
    let Some(h) = open_variant(out, variant, &a.volume, guid, false) else { return };
    let (start, _, _) = read_all(h, 0, journal_id);
    let start = if start == 0 { 0 } else { start };
    // Own files: the probe creates and renames them.
    for i in 0..5 {
        let p = a.ok_dir.join(format!("granted-{i}.txt"));
        let _ = std::fs::write(&p, b"data");
        let _ = std::fs::rename(&p, a.ok_dir.join(format!("granted-{i}-renamed.txt")));
        let _ = std::fs::remove_file(a.ok_dir.join(format!("granted-{i}-renamed.txt")));
    }
    let _ = std::fs::write(a.ok_dir.join("ready.flag"), b"1");
    let admin_done = wait_for(&a.ok_dir.join("done.flag"), Duration::from_secs(60));
    let denied_frn = std::fs::read_to_string(a.ok_dir.join("denied-dir.frn")).ok().and_then(|t| u128::from_str_radix(t.trim().trim_start_matches("0x"), 16).ok());
    let (_, recs, err) = read_all(h, start, journal_id);
    let granted = recs.iter().filter(|r| r.name.starts_with("granted-")).count();
    let denied_named = recs.iter().filter(|r| r.name.starts_with("denied-")).count();
    let under_denied = denied_frn.map_or(0, |f| recs.iter().filter(|r| r.parent == f).count());
    let under_denied_named = denied_frn.map_or(0, |f| recs.iter().filter(|r| r.parent == f && !r.name.is_empty()).count());
    let empty = recs.iter().filter(|r| r.name.is_empty()).count();
    let parent_zero = recs.iter().filter(|r| r.parent == 0).count();
    let frn_zero = recs.iter().filter(|r| r.frn == 0).count();
    out.line(
        "name-withholding",
        &[
            ("handle", s(variant)),
            ("admin_done", format!("{admin_done}")),
            ("records", format!("{}", recs.len())),
            ("granted_named", format!("{granted}")),
            ("denied_named", format!("{denied_named}")),
            ("under_denied_dir", format!("{under_denied}")),
            ("under_denied_dir_named", format!("{under_denied_named}")),
            ("empty_name", format!("{empty}")),
            ("parent_frn_zero", format!("{parent_zero}")),
            ("frn_zero", format!("{frn_zero}")),
            ("read_error", err.map_or("null".into(), |e| format!("{e}"))),
        ],
    );
    // Q4: the cache-miss path of #47: open by id and resolve the final path.
    if let Some(first) = recs.iter().find(|r| r.name.starts_with("granted-") || r.name.starts_with("touch-")) {
        out.line("record-sample", &[("name", s(&first.name)), ("reason", format!("{:#x}", first.reason)), ("frn", s(&format!("{:#x}", first.frn))), ("parent", s(&format!("{:#x}", first.parent)))]);
    }
    // SAFETY: handle opened above.
    unsafe { CloseHandle(h) };
}


/// Q4: the cache-miss path of #47: `OpenFileById` plus `GetFinalPathNameByHandleW`
/// for the FRN of a record, through the volume handle.
fn resolve_frn(volume_handle: HANDLE, frn: u128, extended: bool) -> Result<String, u32> {
    // SAFETY: zero is a valid initial state for this plain-data union struct.
    let mut desc: FILE_ID_DESCRIPTOR = unsafe { std::mem::zeroed() };
    desc.dwSize = u32::try_from(std::mem::size_of::<FILE_ID_DESCRIPTOR>()).unwrap_or(0);
    if extended {
        desc.Type = 2; // ExtendedFileIdType
        // SAFETY: writing the 16-byte extended id member of the union.
        unsafe { desc.Anonymous.ExtendedFileId.Identifier = frn.to_le_bytes() };
    } else {
        desc.Type = 0; // FileIdType
        desc.Anonymous.FileId = i64::try_from(frn).unwrap_or(0);
    }
    // SAFETY: the descriptor is live for the call.
    let h = unsafe { OpenFileById(volume_handle, &raw const desc, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, null(), FILE_FLAG_BACKUP_SEMANTICS) };
    if h == INVALID_HANDLE_VALUE {
        // SAFETY: no preconditions.
        return Err(unsafe { GetLastError() });
    }
    let mut buf = [0u16; 1024];
    // SAFETY: buffer and length are valid for the call.
    let n = unsafe { GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), 1024, 0) };
    // SAFETY: handle opened above.
    unsafe { CloseHandle(h) };
    if n == 0 || n >= 1024 {
        return Err(n);
    }
    Ok(String::from_utf16_lossy(&buf[..n as usize]))
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted.get(((sorted.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0)
}

/// The FRN of `path` through `FSCTL_READ_FILE_USN_DATA` (names are stripped
/// from unprivileged journal records, so records are matched by FRN).
fn file_frn(path: &Path) -> Option<u128> {
    let h = open(&path.to_string_lossy(), GENERIC_READ, FILE_FLAG_BACKUP_SEMANTICS).ok()?;
    let mut buf = vec![0u8; 4096];
    let result = ioctl(h, FSCTL_READ_FILE_USN_DATA, &[2u8, 0, 3, 0], &mut buf);
    // SAFETY: handle opened above.
    unsafe { CloseHandle(h) };
    let n = result.ok()?;
    parse_records_with_prefix(&buf, n as usize).1.first().map(|r| r.frn)
}

/// Q5: change to readable record, 200 operations, `BytesToWaitFor = 1`
/// polling, each record matched by FRN.
fn latency(out: &Out, a: &Args, variant: &str, guid: Option<&str>, journal_id: u64) {
    let Some(h) = open_variant(out, variant, &a.volume, guid, false) else { return };
    let (mut next, _, _) = read_all(h, 0, journal_id);
    let mut micros = Vec::new();
    let mut misses = 0u32;
    let mut buf = vec![0u8; 65_536];
    for i in 0..200 {
        let path = a.ok_dir.join(format!("lat-{i}.txt"));
        let t0 = Instant::now();
        let _ = std::fs::write(&path, b"z");
        let Some(frn) = file_frn(&path) else {
            misses += 1;
            continue;
        };
        let mut seen = false;
        while t0.elapsed() < Duration::from_secs(2) && !seen {
            // A short timeout (100 ns units) keeps the call from blocking for good.
            if let Ok(n) = ioctl(h, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, &read_input(next, journal_id, 1, 10_000_000), &mut buf) {
                let (n_usn, recs) = parse_records_with_prefix(&buf, n as usize);
                if !recs.is_empty() {
                    next = n_usn;
                }
                seen = recs.iter().any(|r| r.frn == frn);
            }
        }
        if seen {
            micros.push(u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX));
        } else {
            misses += 1;
        }
    }
    micros.sort_unstable();
    out.line("latency", &[("handle", s(variant)), ("ops", "200".into()), ("misses", format!("{misses}")), ("p50_us", format!("{}", percentile(&micros, 0.5))), ("p99_us", format!("{}", percentile(&micros, 0.99))), ("max_us", format!("{}", micros.last().copied().unwrap_or(0)))]);
    // SAFETY: handle opened above.
    unsafe { CloseHandle(h) };
}

/// Q6: an overlapped unprivileged read at the head of the journal pends and
/// `CancelIoEx` aborts it.
fn overlapped(out: &Out, a: &Args, variant: &str, guid: Option<&str>, journal_id: u64) {
    let Some(h) = open_variant(out, variant, &a.volume, guid, true) else { return };
    let mut buf = vec![0u8; 65_536];
    // Find the head: a synchronous read would need the other handle, so read to the end overlapped-free by using a wait of 0.
    let mut next = 0i64;
    for _ in 0..1000 {
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        // SAFETY: event creation has no preconditions.
        ov.hEvent = unsafe { CreateEventW(null(), 1, 0, null()) };
        let input = read_input(next, journal_id, 0, 0);
        let mut returned = 0u32;
        // SAFETY: buffers and the OVERLAPPED stay live until the call and the wait below complete.
        let ok = unsafe { DeviceIoControl(h, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, input.as_ptr().cast(), 48, buf.as_mut_ptr().cast(), 65_536, &raw mut returned, &raw mut ov) };
        // SAFETY: no preconditions.
        let err = if ok == 0 { unsafe { GetLastError() } } else { 0 };
        if ok == 0 && err == ERROR_IO_PENDING {
            // SAFETY: handle and event are live.
            unsafe { WaitForSingleObject(ov.hEvent, 5000) };
        }
        // SAFETY: handle and OVERLAPPED are live.
        let done = unsafe { GetOverlappedResult(h, &raw const ov, &raw mut returned, 0) };
        // SAFETY: event created above.
        unsafe { CloseHandle(ov.hEvent) };
        if done == 0 {
            out.line("overlapped", &[("handle", s(variant)), ("ok", "false".into()), ("stage", s("head-read")), ("error", error_text(unsafe { GetLastError() }))]);
            // SAFETY: handle opened above.
            unsafe { CloseHandle(h) };
            return;
        }
        let (n_usn, recs) = parse_records_with_prefix(&buf, returned as usize);
        if recs.is_empty() || n_usn == next {
            break;
        }
        next = n_usn;
    }
    // Now at the head: wait for 1 byte with a long timeout, then cancel.
    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    // SAFETY: event creation has no preconditions.
    ov.hEvent = unsafe { CreateEventW(null(), 1, 0, null()) };
    let input = read_input(next, journal_id, 1, 10_000_000 * 30);
    let mut returned = 0u32;
    // SAFETY: as above.
    let ok = unsafe { DeviceIoControl(h, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, input.as_ptr().cast(), 48, buf.as_mut_ptr().cast(), 65_536, &raw mut returned, &raw mut ov) };
    // SAFETY: no preconditions.
    let err = if ok == 0 { unsafe { GetLastError() } } else { 0 };
    let pended = ok == 0 && err == ERROR_IO_PENDING;
    let t0 = Instant::now();
    // SAFETY: handle and event are live.
    let early = unsafe { WaitForSingleObject(ov.hEvent, 500) } == WAIT_OBJECT_0;
    // SAFETY: handle and OVERLAPPED are live.
    let cancel = unsafe { CancelIoEx(h, &raw const ov) };
    // SAFETY: no preconditions.
    let cancel_err = if cancel == 0 { unsafe { GetLastError() } } else { 0 };
    // SAFETY: handle and event are live.
    unsafe { WaitForSingleObject(ov.hEvent, INFINITE) };
    // SAFETY: handle and OVERLAPPED are live.
    let _ = unsafe { GetOverlappedResult(h, &raw const ov, &raw mut returned, 0) };
    // SAFETY: no preconditions.
    let final_err = unsafe { GetLastError() };
    out.line(
        "overlapped",
        &[
            ("handle", s(variant)),
            ("ok", "true".into()),
            ("pended", format!("{pended}")),
            ("completed_before_cancel", format!("{early}")),
            ("cancel_ok", format!("{}", cancel != 0)),
            ("cancel_error", format!("{cancel_err}")),
            ("final_error", format!("{final_err}")),
            ("cancel_to_complete_us", format!("{}", t0.elapsed().as_micros())),
        ],
    );
    // SAFETY: event and handle created above.
    unsafe {
        CloseHandle(ov.hEvent);
        CloseHandle(h);
    }
}

fn command_line(extra: &[String]) -> Vec<u16> {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut line = format!("\"{exe}\" battery");
    for arg in extra {
        line.push_str(&format!(" \"{arg}\""));
    }
    wide(&line)
}

fn run_restricted(extra: &[String]) {
    let mut token: HANDLE = null_mut();
    // SAFETY: out pointer valid.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT, &raw mut token) };
    if ok == 0 {
        println!("{{\"cell\":\"restricted\",\"ok\":false,\"error\":{}}}", unsafe { GetLastError() });
        return;
    }
    let mut sid = vec![0u8; 68];
    let mut sid_len = 68u32;
    // SAFETY: buffer and length valid.
    unsafe { CreateWellKnownSid(WinBuiltinAdministratorsSid, null_mut(), sid.as_mut_ptr().cast(), &raw mut sid_len) };
    let disable = SID_AND_ATTRIBUTES { Sid: sid.as_mut_ptr().cast(), Attributes: 0 };
    let mut restricted: HANDLE = null_mut();
    // SAFETY: token and the SID array are live.
    let ok = unsafe { CreateRestrictedToken(token, DISABLE_MAX_PRIVILEGE, 1, &disable, 0, null(), 0, null(), &raw mut restricted) };
    if ok == 0 {
        println!("{{\"cell\":\"restricted\",\"ok\":false,\"error\":{}}}", unsafe { GetLastError() });
        return;
    }
    // Medium integrity (S-1-16-8192).
    let mut label_sid = [1u8, 1, 0, 0, 0, 0, 0, 16, 0, 0x20, 0, 0];
    let label = TOKEN_MANDATORY_LABEL { Label: SID_AND_ATTRIBUTES { Sid: label_sid.as_mut_ptr().cast(), Attributes: 0x20 } };
    // SAFETY: the label and SID live across the call.
    let il = unsafe { SetTokenInformation(restricted, TokenIntegrityLevel, (&raw const label).cast(), u32::try_from(std::mem::size_of::<TOKEN_MANDATORY_LABEL>() + 12).unwrap_or(0)) };
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let mut cmd = command_line(extra);
    // SAFETY: all pointers are valid for the call; the command line is mutable and NUL-terminated.
    let created = unsafe { CreateProcessAsUserW(restricted, null(), cmd.as_mut_ptr(), null(), null(), 0, 0, null(), null(), &si, &raw mut pi) };
    if created == 0 {
        println!("{{\"cell\":\"restricted\",\"ok\":false,\"error\":{},\"integrity_set\":{}}}", unsafe { GetLastError() }, il != 0);
        return;
    }
    // SAFETY: the process handle is valid.
    unsafe {
        WaitForSingleObject(pi.hProcess, INFINITE);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
}

fn run_user(name: &str, password: &str, extra: &[String]) {
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(0);
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // The child has its own console: capture what it prints next to its JSONL.
    let log = parse_args(extra).out.with_extension("child.log");
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut line = format!("cmd.exe /c \"\"{exe}\" battery");
    for arg in extra {
        line.push_str(&format!(" \"{arg}\""));
    }
    line.push_str(&format!(" > \"{}\" 2>&1\"", log.display()));
    let mut cmd = wide(&line);
    // SAFETY: all strings are NUL-terminated and live for the call.
    let ok = unsafe { CreateProcessWithLogonW(wide(name).as_ptr(), wide(".").as_ptr(), wide(password).as_ptr(), LOGON_WITH_PROFILE, null(), cmd.as_mut_ptr(), 0, null(), null(), &si, &raw mut pi) };
    if ok == 0 {
        println!("{{\"cell\":\"user\",\"ok\":false,\"error\":{}}}", unsafe { GetLastError() });
        return;
    }
    // SAFETY: the process handle is valid.
    unsafe {
        WaitForSingleObject(pi.hProcess, INFINITE);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("battery") => battery(&parse_args(&args[1..])),
        Some("restricted") => run_restricted(&args[1..]),
        Some("user") if args.len() >= 3 => run_user(&args[1], &args[2], &args[3..]),
        _ => println!("usage: usn-probe battery|restricted|user ..."),
    }
}
