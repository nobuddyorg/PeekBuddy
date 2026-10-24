//! Spike #31: can a standard user read the NTFS/ReFS change journal?
//!
//! Subcommands (all write JSON lines to `--out FILE`, else to stdout):
//!
//! - `attempt-all --volume X: [--start start.json] [--file path]`: every
//!   handle variant against every FSCTL, one row per attempt.
//! - `watch --volume X: --handle H --cell DIR`: tails the journal while an
//!   admin `workload run` creates, writes, renames and deletes files in a
//!   directory the probe may access and one it may not; matches records to
//!   the manifest by file reference number.
//! - `overlapped --volume X: --handle H --cell DIR`: overlapped read on an
//!   I/O completion port, wake latency, `CancelIoEx`.
//! - `workload prepare|run`: the admin side of `watch`.
//! - `tree` and `frnset`: the stretch question 7 (directory FRN set).
//! - `spawn-restricted` and `spawn-as`: run a command with a restricted
//!   token or as another local user.
//!
//! Windows only; elsewhere the binary just says so.

#[cfg(not(windows))]
fn main() {
    eprintln!("usn-probe runs on Windows only");
}

#[cfg(windows)]
fn main() {
    imp::main();
}

#[cfg(windows)]
mod imp {
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::ffi::c_void;
    use std::fs::{self, File, OpenOptions};
    use std::io::Write as _;
    use std::os::windows::io::AsRawHandle as _;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_IO_PENDING, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
        WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{
        CheckTokenMembership, CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE,
        GetLengthSid, GetTokenInformation, SID_AND_ATTRIBUTES, SetTokenInformation,
        TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_ELEVATION,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenElevation, TokenElevationType,
        TokenIntegrityLevel, TokenPrivileges, WinBuiltinAdministratorsSid,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OVERLAPPED, FILE_ID_128, FILE_ID_DESCRIPTOR, FILE_ID_INFO, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo, GetFileInformationByHandleEx,
        GetFinalPathNameByHandleW, GetVolumeNameForVolumeMountPointW, OPEN_EXISTING, OpenFileById,
    };
    use windows_sys::Win32::System::IO::{
        CancelIoEx, CreateIoCompletionPort, DeviceIoControl, GetQueuedCompletionStatus, OVERLAPPED,
    };
    use windows_sys::Win32::System::Ioctl::{
        FSCTL_ENUM_USN_DATA, FSCTL_GET_NTFS_VOLUME_DATA, FSCTL_QUERY_USN_JOURNAL,
        FSCTL_READ_FILE_USN_DATA, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
    };
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateProcessAsUserW, CreateProcessWithLogonW, GetCurrentProcess,
        GetExitCodeProcess, LOGON_WITH_PROFILE, OpenProcessToken, PROCESS_INFORMATION,
        STARTUPINFOW, WaitForSingleObject,
    };

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OSVERSIONINFOW) -> i32;
    }

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_LIST_DIRECTORY: u32 = 0x1;
    const FILE_TRAVERSE: u32 = 0x20;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const USN_REASON_FILE_CREATE: u32 = 0x100;
    const USN_REASON_FILE_DELETE: u32 = 0x200;
    const ERROR_HANDLE_EOF: u32 = 38;
    const ERROR_OPERATION_ABORTED: u32 = 995;

    // ---------------------------------------------------------------- args

    struct Args {
        cmd: String,
        kv: HashMap<String, String>,
        rest: Vec<String>,
    }

    impl Args {
        fn parse() -> Args {
            let mut it = std::env::args().skip(1);
            let cmd = it.next().unwrap_or_default();
            let mut kv = HashMap::new();
            let mut rest = Vec::new();
            while let Some(a) = it.next() {
                if a == "--" {
                    rest.extend(it.by_ref());
                    break;
                }
                if let Some(k) = a.strip_prefix("--") {
                    kv.insert(k.to_string(), it.next().unwrap_or_default());
                }
            }
            Args { cmd, kv, rest }
        }
        fn get(&self, k: &str) -> Option<&str> {
            self.kv.get(k).map(String::as_str)
        }
        fn str(&self, k: &str, default: &str) -> String {
            self.get(k).unwrap_or(default).to_string()
        }
        fn num(&self, k: &str, default: u64) -> u64 {
            self.get(k).and_then(|v| v.parse().ok()).unwrap_or(default)
        }
    }

    struct Out(Option<File>);

    impl Out {
        fn open(args: &Args) -> Out {
            Out(args
                .get("out")
                .and_then(|p| OpenOptions::new().create(true).append(true).open(p).ok()))
        }
        fn row(&mut self, v: Value) {
            let line = v.to_string();
            match &mut self.0 {
                Some(f) => {
                    let _ = writeln!(f, "{line}");
                    let _ = f.flush();
                }
                None => println!("{line}"),
            }
        }
    }

    // ------------------------------------------------------------ plumbing

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn qpc() -> i64 {
        let mut t = 0i64;
        // SAFETY: `t` is a valid out pointer for the duration of the call.
        unsafe { QueryPerformanceCounter(&mut t) };
        t
    }

    fn qpc_freq() -> i64 {
        let mut f = 1i64;
        // SAFETY: `f` is a valid out pointer for the duration of the call.
        unsafe { QueryPerformanceFrequency(&mut f) };
        f.max(1)
    }

    fn ms(ticks: i64) -> f64 {
        ticks as f64 * 1000.0 / qpc_freq() as f64
    }

    fn u16le(b: &[u8], o: usize) -> u16 {
        b.get(o..o + 2)
            .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32le(b: &[u8], o: usize) -> u32 {
        b.get(o..o + 4)
            .map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64le(b: &[u8], o: usize) -> u64 {
        b.get(o..o + 8).map_or(0, |s| {
            let mut a = [0u8; 8];
            a.copy_from_slice(s);
            u64::from_le_bytes(a)
        })
    }
    fn u128le(b: &[u8], o: usize) -> u128 {
        b.get(o..o + 16).map_or(0, |s| {
            let mut a = [0u8; 16];
            a.copy_from_slice(s);
            u128::from_le_bytes(a)
        })
    }

    fn os_info() -> Value {
        let mut v = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..unsafe_zeroed_osvi()
        };
        // SAFETY: `v` is a valid OSVERSIONINFOW with its size field set.
        let status = unsafe { RtlGetVersion(&mut v) };
        json!({
            "status": status,
            "major": v.dwMajorVersion,
            "minor": v.dwMinorVersion,
            "build": v.dwBuildNumber,
            "arch": std::env::consts::ARCH,
        })
    }

    fn unsafe_zeroed_osvi() -> OSVERSIONINFOW {
        OSVERSIONINFOW {
            dwOSVersionInfoSize: 0,
            dwMajorVersion: 0,
            dwMinorVersion: 0,
            dwBuildNumber: 0,
            dwPlatformId: 0,
            szCSDVersion: [0; 128],
        }
    }

    /// Elevation, integrity level, Administrators membership, privilege count.
    fn token_info() -> Value {
        let mut tok: HANDLE = null_mut();
        // SAFETY: the pseudo handle of the current process is always valid and
        // `tok` is a valid out pointer.
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) };
        if ok == 0 {
            return json!({ "error": last_error() });
        }
        let elevated;
        let mut elev_type = 0u32;
        let mut n = 0u32;
        // SAFETY: the out buffers are valid for the sizes passed.
        unsafe {
            let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
            GetTokenInformation(
                tok,
                TokenElevation,
                (&mut e as *mut TOKEN_ELEVATION).cast(),
                4,
                &mut n,
            );
            elevated = e.TokenIsElevated;
            GetTokenInformation(
                tok,
                TokenElevationType,
                (&mut elev_type as *mut u32).cast(),
                4,
                &mut n,
            );
        }
        let integrity = token_integrity(tok);
        let privileges = token_privilege_count(tok);
        // SAFETY: `tok` was opened above and is not used afterwards.
        unsafe { CloseHandle(tok) };
        json!({
            "elevated": elevated != 0,
            "elevation_type": elev_type,
            "integrity_rid": integrity,
            "is_admin_member": is_admin_member(),
            "privilege_count": privileges,
        })
    }

    fn token_integrity(tok: HANDLE) -> Value {
        let mut buf = [0u8; 128];
        let mut n = 0u32;
        // SAFETY: `buf` is valid for the length passed.
        let ok = unsafe {
            GetTokenInformation(
                tok,
                TokenIntegrityLevel,
                buf.as_mut_ptr().cast(),
                128,
                &mut n,
            )
        };
        if ok == 0 {
            return Value::Null;
        }
        // SAFETY: on success the buffer starts with a TOKEN_MANDATORY_LABEL
        // whose SID lives in the same buffer; the sub-authority count is the
        // second byte of the SID and the last sub-authority is the RID.
        let rid = unsafe {
            let label = &*(buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
            let sid = label.Label.Sid as *const u8;
            let count = *sid.add(1) as usize;
            let rid_ptr = sid.add(8 + 4 * (count - 1)).cast::<u32>();
            rid_ptr.read_unaligned()
        };
        json!(rid)
    }

    fn token_privilege_count(tok: HANDLE) -> Value {
        let mut buf = vec![0u8; 4096];
        let mut n = 0u32;
        // SAFETY: `buf` is valid for the length passed.
        let ok = unsafe {
            GetTokenInformation(tok, TokenPrivileges, buf.as_mut_ptr().cast(), 4096, &mut n)
        };
        if ok == 0 {
            Value::Null
        } else {
            json!(u32le(&buf, 0))
        }
    }

    fn is_admin_member() -> bool {
        let mut sid = [0u8; 68];
        let mut len = 68u32;
        // SAFETY: `sid` is valid for `len` bytes.
        let made = unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                sid.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if made == 0 {
            return false;
        }
        let mut member = 0i32;
        // SAFETY: `sid` holds a valid SID; a null token means the caller.
        let ok = unsafe { CheckTokenMembership(null_mut(), sid.as_mut_ptr().cast(), &mut member) };
        ok != 0 && member != 0
    }

    fn last_error() -> u32 {
        // SAFETY: no preconditions.
        unsafe { GetLastError() }
    }

    fn base_row(args: &Args) -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        m.insert("label".into(), json!(args.str("label", "")));
        m.insert("os".into(), os_info());
        m.insert("token".into(), token_info());
        m
    }

    // ------------------------------------------------------------- handles

    fn volume_root(volume: &str) -> String {
        format!("{}\\", volume.trim_end_matches('\\'))
    }

    fn volume_guid_path(volume: &str) -> Result<String, u32> {
        let root = wide(&volume_root(volume));
        let mut buf = [0u16; 64];
        // SAFETY: `root` is NUL terminated and `buf` is valid for 64 units.
        let ok = unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), buf.as_mut_ptr(), 64) };
        if ok == 0 {
            return Err(last_error());
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Ok(String::from_utf16_lossy(buf.get(..n).unwrap_or(&[])))
    }

    fn default_access(variant: &str) -> u32 {
        match variant {
            "guid-dir" | "backup-root" | "device-read" => GENERIC_READ,
            "traverse" => FILE_TRAVERSE,
            _ => 0,
        }
    }

    fn parse_access(s: &str, variant: &str) -> u32 {
        match s {
            "" | "default" => default_access(variant),
            "generic-read" => GENERIC_READ,
            "read-attributes" => FILE_READ_ATTRIBUTES,
            "list-directory" => FILE_LIST_DIRECTORY,
            "traverse" => FILE_TRAVERSE,
            "synchronize" => SYNCHRONIZE,
            "none" => 0,
            hex => u32::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap_or(0),
        }
    }

    struct Opened {
        handle: HANDLE,
        path: String,
    }

    fn open_volume(
        variant: &str,
        volume: &str,
        access: u32,
        overlapped: bool,
    ) -> Result<Opened, (u32, String)> {
        let path = match variant {
            "guid-dir" => volume_guid_path(volume).map_err(|e| (e, "guid lookup".to_string()))?,
            "traverse" | "device-read" => format!("\\\\.\\{}", volume.trim_end_matches('\\')),
            "backup-root" => format!("\\\\?\\{}", volume_root(volume)),
            other => return Err((87, format!("unknown handle variant {other}"))),
        };
        let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
        if overlapped {
            flags |= FILE_FLAG_OVERLAPPED;
        }
        let w = wide(&path);
        // SAFETY: `w` is NUL terminated; no security attributes or template.
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                flags,
                null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err((last_error(), path));
        }
        Ok(Opened { handle: h, path })
    }

    fn close(h: HANDLE) {
        // SAFETY: callers pass handles they opened and no longer use.
        unsafe { CloseHandle(h) };
    }

    fn ioctl(h: HANDLE, code: u32, input: &[u8], out: &mut [u8]) -> Result<u32, u32> {
        let mut n = 0u32;
        let (ip, il) = if input.is_empty() {
            (null(), 0)
        } else {
            (input.as_ptr().cast::<c_void>(), input.len() as u32)
        };
        // SAFETY: the buffers are valid for the lengths passed and the call is
        // synchronous (the handle is not overlapped).
        let ok = unsafe {
            DeviceIoControl(
                h,
                code,
                ip,
                il,
                out.as_mut_ptr().cast(),
                out.len() as u32,
                &mut n,
                null_mut(),
            )
        };
        if ok == 0 { Err(last_error()) } else { Ok(n) }
    }

    // ------------------------------------------------------------- journal

    #[derive(Clone, Copy, Default)]
    struct Journal {
        id: u64,
        first: i64,
        next: i64,
    }

    fn parse_journal(b: &[u8]) -> Journal {
        Journal {
            id: u64le(b, 0),
            first: u64le(b, 8) as i64,
            next: u64le(b, 16) as i64,
        }
    }

    /// `READ_USN_JOURNAL_DATA_V1`, 48 bytes.
    fn read_input(start: i64, timeout_s: u64, bytes_to_wait: u64, journal_id: u64) -> Vec<u8> {
        let mut b = vec![0u8; 48];
        b[0..8].copy_from_slice(&start.to_le_bytes());
        b[8..12].copy_from_slice(&u32::MAX.to_le_bytes()); // every reason
        b[12..16].copy_from_slice(&0u32.to_le_bytes()); // not only on close
        b[16..24].copy_from_slice(&timeout_s.to_le_bytes());
        b[24..32].copy_from_slice(&bytes_to_wait.to_le_bytes());
        b[32..40].copy_from_slice(&journal_id.to_le_bytes());
        b[40..42].copy_from_slice(&2u16.to_le_bytes());
        b[42..44].copy_from_slice(&3u16.to_le_bytes());
        b
    }

    /// `MFT_ENUM_DATA_V1`, 32 bytes.
    fn enum_input(start_frn: u64) -> Vec<u8> {
        let mut b = vec![0u8; 32];
        b[0..8].copy_from_slice(&start_frn.to_le_bytes());
        b[8..16].copy_from_slice(&0i64.to_le_bytes());
        b[16..24].copy_from_slice(&i64::MAX.to_le_bytes());
        b[24..26].copy_from_slice(&2u16.to_le_bytes());
        b[26..28].copy_from_slice(&3u16.to_le_bytes());
        b
    }

    #[derive(Clone, Debug)]
    struct Rec {
        major: u16,
        frn: u128,
        parent: u128,
        reason: u32,
        name: String,
        name_len: u16,
    }

    fn parse_records(buf: &[u8], mut off: usize) -> Vec<Rec> {
        let mut recs = Vec::new();
        while off + 8 <= buf.len() {
            let len = u32le(buf, off) as usize;
            if len < 8 || off + len > buf.len() {
                break;
            }
            let major = u16le(buf, off + 4);
            let rec = match major {
                2 => Some(Rec {
                    major,
                    frn: u64le(buf, off + 8) as u128,
                    parent: u64le(buf, off + 16) as u128,
                    reason: u32le(buf, off + 40),
                    name_len: u16le(buf, off + 56),
                    name: name_at(buf, off, u16le(buf, off + 58), u16le(buf, off + 56)),
                }),
                3 => Some(Rec {
                    major,
                    frn: u128le(buf, off + 8),
                    parent: u128le(buf, off + 24),
                    reason: u32le(buf, off + 56),
                    name_len: u16le(buf, off + 72),
                    name: name_at(buf, off, u16le(buf, off + 74), u16le(buf, off + 72)),
                }),
                _ => None,
            };
            recs.extend(rec);
            off += (len + 7) & !7;
        }
        recs
    }

    fn name_at(buf: &[u8], rec: usize, name_off: u16, name_len: u16) -> String {
        let start = rec + name_off as usize;
        let Some(bytes) = buf.get(start..start + name_len as usize) else {
            return String::new();
        };
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    }

    fn hex128(v: u128) -> String {
        format!("{v:032x}")
    }

    fn percentile(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return f64::NAN;
        }
        let i = ((sorted.len() - 1) as f64 * p).round() as usize;
        sorted.get(i).copied().unwrap_or(f64::NAN)
    }

    fn stats(mut v: Vec<f64>) -> Value {
        v.sort_by(|a, b| a.total_cmp(b));
        json!({
            "n": v.len(),
            "p50_ms": percentile(&v, 0.5),
            "p99_ms": percentile(&v, 0.99),
            "max_ms": v.last().copied().unwrap_or(f64::NAN),
        })
    }

    // -------------------------------------------------------- attempt-all

    const HANDLES: [&str; 4] = ["guid-dir", "traverse", "backup-root", "device-read"];
    const FSCTLS: [&str; 5] = ["query", "ntfs-data", "enum", "read", "read-unpriv"];

    struct StartInfo {
        journal: Journal,
    }

    fn load_start(path: Option<&str>) -> StartInfo {
        let j = path
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|v| Journal {
                id: v["journal_id"].as_u64().unwrap_or(0),
                first: v["first_usn"].as_i64().unwrap_or(0),
                next: v["next_usn"].as_i64().unwrap_or(0),
            })
            .unwrap_or_default();
        StartInfo { journal: j }
    }

    fn run_fsctl(h: HANDLE, fsctl: &str, start: &StartInfo) -> Value {
        let mut buf = vec![0u8; 64 * 1024];
        let t0 = qpc();
        let (res, kind): (Result<u32, u32>, &str) = match fsctl {
            "query" => (ioctl(h, FSCTL_QUERY_USN_JOURNAL, &[], &mut buf), "query"),
            "ntfs-data" => (
                ioctl(h, FSCTL_GET_NTFS_VOLUME_DATA, &[], &mut buf),
                "ntfs-data",
            ),
            "enum" => (
                ioctl(h, FSCTL_ENUM_USN_DATA, &enum_input(0), &mut buf),
                "enum",
            ),
            "read" | "read-unpriv" => {
                let code = if fsctl == "read" {
                    FSCTL_READ_USN_JOURNAL
                } else {
                    FSCTL_READ_UNPRIVILEGED_USN_JOURNAL
                };
                // The journal id comes from the admin-prepared start.json, so the
                // read does not depend on the query working.
                let input = read_input(start.journal.first, 0, 0, start.journal.id);
                (ioctl(h, code, &input, &mut buf), "read")
            }
            _ => (Err(87), "unknown"),
        };
        let took = ms(qpc() - t0);
        match res {
            Err(e) => json!({ "fsctl": fsctl, "ok": false, "error": e, "ms": took }),
            Ok(n) => {
                let data = buf.get(..n as usize).unwrap_or(&[]);
                let mut row = json!({ "fsctl": fsctl, "ok": true, "bytes": n, "ms": took });
                match kind {
                    "query" => {
                        let j = parse_journal(data);
                        row["journal"] = json!({ "id": j.id, "first": j.first, "next": j.next,
                            "min_major": u16le(data, 56), "max_major": u16le(data, 58) });
                    }
                    "read" | "enum" => {
                        let recs = parse_records(data, 8);
                        row["records"] = json!(recs.len());
                        row["empty_names"] = json!(recs.iter().filter(|r| r.name_len == 0).count());
                        row["versions"] =
                            json!(recs.iter().map(|r| r.major).collect::<HashSet<_>>());
                        row["next"] = json!(u64le(data, 0));
                        row["sample"] = json!(recs.iter().take(3).map(|r| json!({
                            "frn": hex128(r.frn), "parent": hex128(r.parent), "name": r.name, "reason": r.reason
                        })).collect::<Vec<_>>());
                    }
                    "ntfs-data" => {
                        row["bytes_per_cluster"] = json!(u32le(data, 44));
                        row["mft_valid_data_length"] = json!(u64le(data, 56));
                    }
                    _ => {}
                }
                row
            }
        }
    }

    fn attempt_all(args: &Args, out: &mut Out) {
        let volume = args.str("volume", "C:");
        let start = load_start(args.get("start"));
        let access_override = args.str("access", "default");
        for variant in HANDLES {
            let access = parse_access(&access_override, variant);
            let mut row = base_row(args);
            row.insert("volume".into(), json!(volume));
            row.insert("handle".into(), json!(variant));
            row.insert("access".into(), json!(format!("{access:#x}")));
            match open_volume(variant, &volume, access, false) {
                Err((e, path)) => {
                    row.insert("kind".into(), json!("open"));
                    row.insert("ok".into(), json!(false));
                    row.insert("error".into(), json!(e));
                    row.insert("path".into(), json!(path));
                    out.row(Value::Object(row));
                }
                Ok(o) => {
                    row.insert("kind".into(), json!("open"));
                    row.insert("ok".into(), json!(true));
                    row.insert("path".into(), json!(o.path));
                    out.row(Value::Object(row.clone()));
                    row.insert("kind".into(), json!("fsctl"));
                    for fsctl in FSCTLS {
                        let mut r = row.clone();
                        if let Value::Object(m) = run_fsctl(o.handle, fsctl, &start) {
                            r.extend(m);
                        }
                        out.row(Value::Object(r));
                    }
                    open_by_id_row(args, &volume, &o, &row, out);
                    close(o.handle);
                }
            }
        }
        if let Some(file) = args.get("file") {
            file_usn_row(args, &volume, file, out);
        }
    }

    /// The cache-miss path of #47: FRN -> OpenFileById -> final path.
    fn open_by_id_row(
        args: &Args,
        volume: &str,
        vol: &Opened,
        base: &serde_json::Map<String, Value>,
        out: &mut Out,
    ) {
        let _ = (args, volume);
        let mut buf = vec![0u8; 64 * 1024];
        let Ok(n) = ioctl(vol.handle, FSCTL_ENUM_USN_DATA, &enum_input(0), &mut buf) else {
            return;
        };
        let recs = parse_records(buf.get(..n as usize).unwrap_or(&[]), 8);
        let (mut tried, mut opened, mut denied, mut not_found, mut other, mut with_path) =
            (0, 0, 0, 0, 0, 0);
        for r in recs.iter().filter(|r| r.name_len > 0).take(200) {
            tried += 1;
            let mut desc: FILE_ID_DESCRIPTOR = FILE_ID_DESCRIPTOR {
                dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
                Type: 2, // ExtendedFileIdType
                Anonymous: windows_sys::Win32::Storage::FileSystem::FILE_ID_DESCRIPTOR_0 {
                    FileId: 0,
                },
            };
            desc.Anonymous.ExtendedFileId = FILE_ID_128 {
                Identifier: r.frn.to_le_bytes(),
            };
            // SAFETY: the descriptor is fully initialised and the volume handle
            // is open.
            let h = unsafe {
                OpenFileById(
                    vol.handle,
                    &desc,
                    FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    null(),
                    FILE_FLAG_BACKUP_SEMANTICS,
                )
            };
            if h == INVALID_HANDLE_VALUE {
                match last_error() {
                    5 => denied += 1,
                    2 | 1005 | 1392 | 87 | 1 => not_found += 1,
                    _ => other += 1,
                }
                continue;
            }
            opened += 1;
            let mut path = [0u16; 1024];
            // SAFETY: `path` is valid for 1024 units.
            let len = unsafe { GetFinalPathNameByHandleW(h, path.as_mut_ptr(), 1024, 0) };
            if len > 0 && (len as usize) < 1024 {
                with_path += 1;
            }
            close(h);
        }
        let mut r = base.clone();
        r.insert("kind".into(), json!("open-by-id"));
        r.insert("tried".into(), json!(tried));
        r.insert("opened".into(), json!(opened));
        r.insert("access_denied".into(), json!(denied));
        r.insert("not_found".into(), json!(not_found));
        r.insert("other_error".into(), json!(other));
        r.insert("final_path_ok".into(), json!(with_path));
        out.row(Value::Object(r));
    }

    /// `FSCTL_READ_FILE_USN_DATA` on a file handle, no volume handle involved.
    fn file_usn_row(args: &Args, volume: &str, file: &str, out: &mut Out) {
        let mut row = base_row(args);
        row.insert("volume".into(), json!(volume));
        row.insert("kind".into(), json!("file-usn"));
        row.insert("file".into(), json!(file));
        let w = wide(file);
        // SAFETY: `w` is NUL terminated.
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            row.insert("ok".into(), json!(false));
            row.insert("open_error".into(), json!(last_error()));
            out.row(Value::Object(row));
            return;
        }
        let mut input = [0u8; 4];
        input[0..2].copy_from_slice(&2u16.to_le_bytes());
        input[2..4].copy_from_slice(&3u16.to_le_bytes());
        let mut buf = vec![0u8; 1024];
        match ioctl(h, FSCTL_READ_FILE_USN_DATA, &input, &mut buf) {
            Ok(n) => {
                let recs = parse_records(buf.get(..n as usize).unwrap_or(&[]), 0);
                row.insert("ok".into(), json!(true));
                row.insert("records".into(), json!(recs.len()));
                row.insert("name".into(), json!(recs.first().map(|r| r.name.clone())));
            }
            Err(e) => {
                row.insert("ok".into(), json!(false));
                row.insert("error".into(), json!(e));
            }
        }
        close(h);
        out.row(Value::Object(row));
    }

    // ------------------------------------------------------------ workload

    fn cell_dir(args: &Args) -> PathBuf {
        PathBuf::from(args.str("cell", "."))
    }

    fn file_frn(f: &File) -> u128 {
        let mut info = FILE_ID_INFO {
            VolumeSerialNumber: 0,
            FileId: FILE_ID_128 {
                Identifier: [0; 16],
            },
        };
        // SAFETY: `info` is a valid FILE_ID_INFO and the handle belongs to `f`.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                f.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        if ok == 0 {
            0
        } else {
            u128::from_le_bytes(info.FileId.Identifier)
        }
    }

    fn dir_frn(path: &Path) -> u128 {
        let w = wide(&path.to_string_lossy());
        // SAFETY: `w` is NUL terminated.
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return 0;
        }
        let mut info = FILE_ID_INFO {
            VolumeSerialNumber: 0,
            FileId: FILE_ID_128 {
                Identifier: [0; 16],
            },
        };
        // SAFETY: `info` is a valid FILE_ID_INFO and `h` was opened above.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                h,
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        };
        close(h);
        if ok == 0 {
            0
        } else {
            u128::from_le_bytes(info.FileId.Identifier)
        }
    }

    /// Admin side, before the probe starts: journal position and directory ids.
    fn workload_prepare(args: &Args, out: &mut Out) {
        let volume = args.str("volume", "C:");
        let cell = cell_dir(args);
        let _ = fs::create_dir_all(&cell);
        for f in ["ready.flag", "done.flag", "manifest.jsonl", "start.json"] {
            let _ = fs::remove_file(cell.join(f));
        }
        let mut row = base_row(args);
        row.insert("kind".into(), json!("prepare"));
        match open_volume("device-read", &volume, GENERIC_READ, false) {
            Err((e, _)) => {
                row.insert("ok".into(), json!(false));
                row.insert("error".into(), json!(e));
            }
            Ok(o) => {
                let mut buf = vec![0u8; 256];
                match ioctl(o.handle, FSCTL_QUERY_USN_JOURNAL, &[], &mut buf) {
                    Ok(_) => {
                        let j = parse_journal(&buf);
                        let start =
                            json!({ "journal_id": j.id, "first_usn": j.first, "next_usn": j.next });
                        let _ = fs::write(cell.join("start.json"), start.to_string());
                        row.insert("ok".into(), json!(true));
                        row.insert("start".into(), start);
                    }
                    Err(e) => {
                        row.insert("ok".into(), json!(false));
                        row.insert("error".into(), json!(e));
                    }
                }
                close(o.handle);
            }
        }
        out.row(Value::Object(row));
    }

    fn workload_run(args: &Args, out: &mut Out) {
        let cell = cell_dir(args);
        let files = args.num("files", 250) as usize;
        let pace = Duration::from_micros(args.num("pace-us", 5000));
        let allowed = PathBuf::from(args.str("allowed", ""));
        let denied = PathBuf::from(args.str("denied", ""));
        let mut manifest = String::new();
        manifest.push_str(&json!({
            "header": true, "qpc_freq": qpc_freq(),
            "allowed_dir_frn": hex128(dir_frn(&allowed)), "denied_dir_frn": hex128(dir_frn(&denied)),
        }).to_string());
        manifest.push('\n');
        let mut errors = 0u32;
        for i in 0..files {
            for (class, dir) in [("allowed", &allowed), ("denied", &denied)] {
                let p1 = dir.join(format!("f{i}.tmp"));
                let p2 = dir.join(format!("r{i}.tmp"));
                let t_create = qpc();
                let Ok(mut f) = OpenOptions::new().write(true).create_new(true).open(&p1) else {
                    errors += 1;
                    continue;
                };
                let frn = file_frn(&f);
                let t_write = qpc();
                let _ = f.write_all(&[0x5a; 128]);
                drop(f);
                let t_closed = qpc();
                let t_rename = qpc();
                if fs::rename(&p1, &p2).is_err() {
                    errors += 1;
                }
                let t_delete = qpc();
                if fs::remove_file(&p2).is_err() {
                    errors += 1;
                }
                manifest.push_str(&json!({
                    "class": class, "i": i, "frn": hex128(frn), "name": format!("f{i}.tmp"), "renamed": format!("r{i}.tmp"),
                    "t_create": t_create, "t_write": t_write, "t_closed": t_closed, "t_rename": t_rename, "t_delete": t_delete,
                }).to_string());
                manifest.push('\n');
            }
            std::thread::sleep(pace);
        }
        let _ = fs::write(cell.join("manifest.jsonl"), manifest);
        let _ = fs::write(cell.join("done.flag"), b"1");
        out.row(json!({ "kind": "workload", "files_per_dir": files, "ops": files * 8, "errors": errors }));
    }

    // --------------------------------------------------------------- watch

    struct Manifest {
        freq: i64,
        entries: Vec<Value>,
    }

    fn load_manifest(cell: &Path) -> Option<Manifest> {
        let text = fs::read_to_string(cell.join("manifest.jsonl")).ok()?;
        let mut freq = 1;
        let mut entries = Vec::new();
        for line in text.lines() {
            let v: Value = serde_json::from_str(line).ok()?;
            if v["header"].as_bool() == Some(true) {
                freq = v["qpc_freq"].as_i64().unwrap_or(1);
            } else {
                entries.push(v);
            }
        }
        Some(Manifest { freq, entries })
    }

    struct Seen {
        rec: Rec,
        at: i64,
    }

    fn watch(args: &Args, out: &mut Out) {
        let volume = args.str("volume", "C:");
        let variant = args.str("handle", "guid-dir");
        let fsctl = args.str("fsctl", "read-unpriv");
        let cell = cell_dir(args);
        let access = parse_access(&args.str("access", "default"), &variant);
        let mut row = base_row(args);
        row.insert("kind".into(), json!("watch"));
        row.insert("volume".into(), json!(volume));
        row.insert("handle".into(), json!(variant));
        row.insert("fsctl".into(), json!(fsctl));
        row.insert("access".into(), json!(format!("{access:#x}")));
        let vol = match open_volume(&variant, &volume, access, false) {
            Ok(v) => v,
            Err((e, path)) => {
                row.insert("ok".into(), json!(false));
                row.insert("open_error".into(), json!(e));
                row.insert("path".into(), json!(path));
                out.row(Value::Object(row));
                return;
            }
        };
        let code = if fsctl == "read" {
            FSCTL_READ_USN_JOURNAL
        } else {
            FSCTL_READ_UNPRIVILEGED_USN_JOURNAL
        };
        let start = load_start(Some(&cell.join("start.json").to_string_lossy()));
        let mut usn = start.journal.next;
        let mut buf = vec![0u8; 64 * 1024];
        let mut seen: Vec<Seen> = Vec::new();
        let mut calls = 0u32;
        let mut empty_calls = 0u32;
        let mut records_per_call: Vec<f64> = Vec::new();
        let mut first_error: Option<u32> = None;
        let mut ready = false;
        let mut done_at: Option<Instant> = None;
        let mut idle_since = Instant::now();
        let overall = Instant::now();
        let tail = Duration::from_secs(args.num("tail-s", 45));
        let mut manifest: Option<Manifest> = None;
        loop {
            let input = read_input(usn, 1, 1, start.journal.id);
            let t_call = Instant::now();
            match ioctl(vol.handle, code, &input, &mut buf) {
                Ok(n) => {
                    let at = qpc();
                    let data = buf.get(..n as usize).unwrap_or(&[]);
                    let recs = parse_records(data, 8);
                    calls += 1;
                    if recs.is_empty() {
                        empty_calls += 1;
                    } else {
                        idle_since = Instant::now();
                        records_per_call.push(recs.len() as f64);
                    }
                    let next = u64le(data, 0) as i64;
                    if next > usn {
                        usn = next;
                    }
                    seen.extend(recs.into_iter().map(|rec| Seen { rec, at }));
                }
                Err(e) if e == ERROR_HANDLE_EOF => {}
                Err(e) => {
                    first_error = Some(e);
                    break;
                }
            }
            if !ready {
                let _ = fs::write(cell.join("ready.flag"), b"1");
                ready = true;
            }
            if t_call.elapsed() < Duration::from_millis(50) {
                std::thread::sleep(Duration::from_millis(2)); // the call returned at once: no wait support
            }
            if overall.elapsed() > Duration::from_secs(300) {
                break;
            }
            if cell.join("done.flag").exists() {
                let d = *done_at.get_or_insert_with(Instant::now);
                if manifest.is_none() {
                    manifest = load_manifest(&cell);
                }
                let complete = manifest.as_ref().is_some_and(|m| all_seen(m, &seen));
                if (complete && idle_since.elapsed() > Duration::from_secs(1)) || d.elapsed() > tail
                {
                    break;
                }
            }
        }
        close(vol.handle);
        row.insert("ok".into(), json!(first_error.is_none()));
        if let Some(e) = first_error {
            row.insert("read_error".into(), json!(e));
        }
        row.insert("calls".into(), json!(calls));
        row.insert("empty_calls".into(), json!(empty_calls));
        row.insert("records_total".into(), json!(seen.len()));
        row.insert("records_per_call".into(), stats(records_per_call));
        row.insert("last_usn".into(), json!(usn));
        if manifest.is_none() {
            manifest = load_manifest(&cell);
        }
        if let Some(m) = manifest {
            summarize(&mut row, &m, &seen);
        } else {
            row.insert("manifest".into(), json!("missing"));
        }
        out.row(Value::Object(row));
    }

    fn by_frn(seen: &[Seen]) -> HashMap<u128, Vec<&Seen>> {
        let mut map: HashMap<u128, Vec<&Seen>> = HashMap::new();
        for s in seen {
            map.entry(s.rec.frn).or_default().push(s);
        }
        map
    }

    fn manifest_frn(e: &Value) -> u128 {
        u128::from_str_radix(e["frn"].as_str().unwrap_or("0"), 16).unwrap_or(0)
    }

    fn all_seen(m: &Manifest, seen: &[Seen]) -> bool {
        let map = by_frn(seen);
        m.entries
            .iter()
            .filter(|e| e["class"] == "allowed")
            .all(|e| {
                map.get(&manifest_frn(e))
                    .is_some_and(|v| v.iter().any(|s| s.rec.reason & USN_REASON_FILE_DELETE != 0))
            })
    }

    fn summarize(row: &mut serde_json::Map<String, Value>, m: &Manifest, seen: &[Seen]) {
        let map = by_frn(seen);
        let mut per_class: BTreeMap<String, Value> = BTreeMap::new();
        let mut lat_create: Vec<f64> = Vec::new();
        let mut lat_delete: Vec<f64> = Vec::new();
        let mut sample_empty: Vec<Value> = Vec::new();
        for class in ["allowed", "denied"] {
            let (mut files, mut files_seen, mut recs, mut named, mut empty, mut wrong_name) =
                (0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
            let mut c_lat = Vec::new();
            let mut d_lat = Vec::new();
            for e in m.entries.iter().filter(|e| e["class"] == class) {
                files += 1;
                let Some(v) = map.get(&manifest_frn(e)) else {
                    continue;
                };
                files_seen += 1;
                let expect = [
                    e["name"].as_str().unwrap_or(""),
                    e["renamed"].as_str().unwrap_or(""),
                ];
                for s in v {
                    recs += 1;
                    if s.rec.name_len == 0 {
                        empty += 1;
                        if sample_empty.len() < 3 {
                            sample_empty.push(json!({ "class": class, "reason": s.rec.reason, "frn": hex128(s.rec.frn), "parent": hex128(s.rec.parent) }));
                        }
                    } else {
                        named += 1;
                        if !expect.contains(&s.rec.name.as_str()) {
                            wrong_name += 1;
                        }
                    }
                    if s.rec.reason & USN_REASON_FILE_CREATE != 0 {
                        let t = e["t_create"].as_i64().unwrap_or(0);
                        c_lat.push((s.at - t) as f64 * 1000.0 / m.freq as f64);
                    }
                    if s.rec.reason & USN_REASON_FILE_DELETE != 0 {
                        let t = e["t_delete"].as_i64().unwrap_or(0);
                        d_lat.push((s.at - t) as f64 * 1000.0 / m.freq as f64);
                    }
                }
            }
            if class == "allowed" {
                lat_create = c_lat.clone();
                lat_delete = d_lat.clone();
            }
            per_class.insert(
                class.to_string(),
                json!({ "files": files, "files_with_records": files_seen, "records": recs,
                        "records_with_name": named, "records_empty_name": empty, "records_wrong_name": wrong_name,
                        "latency_create": stats(c_lat), "latency_delete": stats(d_lat) }),
            );
        }
        let matched: HashSet<u128> = m.entries.iter().map(manifest_frn).collect();
        let unmatched = seen
            .iter()
            .filter(|s| !matched.contains(&s.rec.frn))
            .count();
        let reasons: HashSet<u32> = seen.iter().map(|s| s.rec.reason).collect();
        row.insert("classes".into(), json!(per_class));
        row.insert("unmatched_records".into(), json!(unmatched));
        row.insert("distinct_reason_masks".into(), json!(reasons.len()));
        row.insert(
            "versions".into(),
            json!(seen.iter().map(|s| s.rec.major).collect::<HashSet<_>>()),
        );
        row.insert("sample_empty".into(), json!(sample_empty));
        row.insert("latency_create_allowed".into(), stats(lat_create));
        row.insert("latency_delete_allowed".into(), stats(lat_delete));
    }

    // ---------------------------------------------------------- overlapped

    /// An overlapped unprivileged read on an IOCP: pending, wakes on a change,
    /// and `CancelIoEx` completes it with `ERROR_OPERATION_ABORTED`.
    fn overlapped(args: &Args, out: &mut Out) {
        let volume = args.str("volume", "C:");
        let variant = args.str("handle", "guid-dir");
        let fsctl = args.str("fsctl", "read-unpriv");
        let cell = cell_dir(args);
        let access = parse_access(&args.str("access", "default"), &variant);
        let mut row = base_row(args);
        row.insert("kind".into(), json!("overlapped"));
        row.insert("volume".into(), json!(volume));
        row.insert("handle".into(), json!(variant));
        row.insert("fsctl".into(), json!(fsctl));
        let vol = match open_volume(&variant, &volume, access, true) {
            Ok(v) => v,
            Err((e, _)) => {
                row.insert("ok".into(), json!(false));
                row.insert("open_error".into(), json!(e));
                out.row(Value::Object(row));
                return;
            }
        };
        let code = if fsctl == "read" {
            FSCTL_READ_USN_JOURNAL
        } else {
            FSCTL_READ_UNPRIVILEGED_USN_JOURNAL
        };
        let start = load_start(Some(&cell.join("start.json").to_string_lossy()));
        // SAFETY: the volume handle is valid; a null existing port creates one.
        let port = unsafe { CreateIoCompletionPort(vol.handle, null_mut(), 1, 1) };
        row.insert("iocp_associate".into(), json!(!port.is_null()));
        let mut buf = vec![0u8; 64 * 1024];
        let mut ov: Box<OVERLAPPED> = Box::new(unsafe_zeroed_overlapped());
        // The read starts at the journal tail so that only a new change completes it.
        let input = read_input(start.journal.next, 0, 1, start.journal.id);
        let mut n = 0u32;
        // SAFETY: `input`, `buf` and `ov` stay alive until the completion has
        // been dequeued below.
        let ok = unsafe {
            DeviceIoControl(
                vol.handle,
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut n,
                &mut *ov,
            )
        };
        let err = if ok == 0 { last_error() } else { 0 };
        row.insert("issue_ok".into(), json!(ok != 0));
        row.insert("issue_error".into(), json!(err));
        let pending = ok == 0 && err == ERROR_IO_PENDING;
        row.insert("pending".into(), json!(pending));
        if pending {
            // Change something the probe may touch, then time the wake.
            let probe_file = cell.join("overlapped.tmp");
            let t0 = qpc();
            let _ = fs::write(&probe_file, b"x");
            let (st, bytes, dequeued) = wait_completion(port, 5000);
            let t1 = qpc();
            let _ = fs::remove_file(&probe_file);
            row.insert("wake_dequeued".into(), json!(dequeued));
            row.insert("wake_status".into(), json!(st));
            row.insert("wake_bytes".into(), json!(bytes));
            row.insert(
                "wake_latency_ms".into(),
                json!(if dequeued { ms(t1 - t0) } else { f64::NAN }),
            );
            // Second read, cancelled.
            let mut ov2: Box<OVERLAPPED> = Box::new(unsafe_zeroed_overlapped());
            let start2 = if dequeued && st == 0 {
                u64le(&buf, 0) as i64
            } else {
                start.journal.next
            };
            let input2 = read_input(start2, 0, 1, start.journal.id);
            let mut buf2 = vec![0u8; 64 * 1024];
            // SAFETY: as above; the buffers outlive the dequeue below.
            let ok2 = unsafe {
                DeviceIoControl(
                    vol.handle,
                    code,
                    input2.as_ptr().cast(),
                    input2.len() as u32,
                    buf2.as_mut_ptr().cast(),
                    buf2.len() as u32,
                    &mut n,
                    &mut *ov2,
                )
            };
            let err2 = if ok2 == 0 { last_error() } else { 0 };
            row.insert("second_issue_error".into(), json!(err2));
            if ok2 == 0 && err2 == ERROR_IO_PENDING {
                let t2 = qpc();
                // SAFETY: cancels the pending request of this handle only.
                let cancelled = unsafe { CancelIoEx(vol.handle, &*ov2) };
                let cancel_err = if cancelled == 0 { last_error() } else { 0 };
                let (st2, _, dq2) = wait_completion(port, 5000);
                row.insert("cancel_call_ok".into(), json!(cancelled != 0));
                row.insert("cancel_call_error".into(), json!(cancel_err));
                row.insert("cancel_dequeued".into(), json!(dq2));
                row.insert("cancel_status".into(), json!(st2));
                row.insert(
                    "cancel_is_aborted".into(),
                    json!(st2 == ERROR_OPERATION_ABORTED),
                );
                row.insert("cancel_latency_ms".into(), json!(ms(qpc() - t2)));
            }
        }
        close(vol.handle);
        if !port.is_null() {
            close(port);
        }
        row.insert("ok".into(), json!(true));
        out.row(Value::Object(row));
    }

    fn unsafe_zeroed_overlapped() -> OVERLAPPED {
        // SAFETY: OVERLAPPED is plain data for which all-zero is the documented
        // initial state.
        unsafe { std::mem::zeroed() }
    }

    /// Returns (completion status as Win32 error, bytes, dequeued).
    fn wait_completion(port: HANDLE, timeout_ms: u32) -> (u32, u32, bool) {
        let mut bytes = 0u32;
        let mut key = 0usize;
        let mut ovp: *mut OVERLAPPED = null_mut();
        // SAFETY: all out pointers are valid for the call.
        let ok =
            unsafe { GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut ovp, timeout_ms) };
        if !ovp.is_null() {
            let st = if ok != 0 { 0 } else { last_error() };
            return (st, bytes, true);
        }
        (last_error(), 0, false)
    }

    // ------------------------------------------------- spawn helpers

    fn command_line(rest: &[String]) -> String {
        rest.iter()
            .map(|a| {
                if a.contains(' ') || a.is_empty() {
                    format!("\"{a}\"")
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn wait_child(pi: &PROCESS_INFORMATION, timeout_s: u64) -> i32 {
        // SAFETY: the process handle is valid until closed below.
        let w = unsafe { WaitForSingleObject(pi.hProcess, (timeout_s * 1000) as u32) };
        let mut code = 0u32;
        let rc = if w == WAIT_OBJECT_0 {
            // SAFETY: valid process handle and out pointer.
            unsafe { GetExitCodeProcess(pi.hProcess, &mut code) };
            code as i32
        } else {
            -1
        };
        close(pi.hProcess);
        close(pi.hThread);
        rc
    }

    fn startup() -> STARTUPINFOW {
        // SAFETY: STARTUPINFOW is plain data; zero plus `cb` is the documented
        // minimal initialisation.
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si
    }

    /// Runs the command with a restricted token: all privileges removed
    /// except the bypass-traverse one Windows keeps, Administrators
    /// deny-only, medium integrity.
    fn spawn_restricted(args: &Args) -> i32 {
        let mut tok: HANDLE = null_mut();
        // SAFETY: valid pseudo handle and out pointer.
        let ok = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
                &mut tok,
            )
        };
        if ok == 0 {
            eprintln!("OpenProcessToken failed: {}", last_error());
            return 2;
        }
        let mut sid = [0u8; 68];
        let mut len = 68u32;
        // SAFETY: `sid` is valid for `len` bytes.
        unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                sid.as_mut_ptr().cast(),
                &mut len,
            )
        };
        let deny = SID_AND_ATTRIBUTES {
            Sid: sid.as_mut_ptr().cast(),
            Attributes: 0,
        };
        let mut restricted: HANDLE = null_mut();
        // SAFETY: `deny` points at one valid SID; no restricting SIDs.
        let ok = unsafe {
            CreateRestrictedToken(
                tok,
                DISABLE_MAX_PRIVILEGE,
                1,
                &deny,
                0,
                null(),
                0,
                null(),
                &mut restricted,
            )
        };
        if ok == 0 {
            eprintln!("CreateRestrictedToken failed: {}", last_error());
            return 3;
        }
        let medium = wide("S-1-16-8192");
        let mut msid: *mut c_void = null_mut();
        // SAFETY: `medium` is NUL terminated; `msid` is freed with LocalFree below.
        if unsafe { ConvertStringSidToSidW(medium.as_ptr(), &mut msid) } != 0 {
            let label = TOKEN_MANDATORY_LABEL {
                Label: SID_AND_ATTRIBUTES {
                    Sid: msid,
                    Attributes: 0x20,
                },
            };
            // SAFETY: `label` and the SID it points to are valid for the call.
            let set = unsafe {
                SetTokenInformation(
                    restricted,
                    TokenIntegrityLevel,
                    (&label as *const TOKEN_MANDATORY_LABEL).cast(),
                    std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(msid),
                )
            };
            if set == 0 {
                eprintln!("SetTokenInformation(integrity) failed: {}", last_error());
            }
            // SAFETY: allocated by ConvertStringSidToSidW.
            unsafe { LocalFree(msid) };
        }
        let mut cmd = wide(&command_line(&args.rest));
        let si = startup();
        // SAFETY: plain data, filled by the call.
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `cmd` is a writable NUL terminated buffer; the token is a
        // restricted copy of this process's own primary token.
        let ok = unsafe {
            CreateProcessAsUserW(
                restricted,
                null(),
                cmd.as_mut_ptr(),
                null(),
                null(),
                0,
                CREATE_NO_WINDOW,
                null(),
                null(),
                &si,
                &mut pi,
            )
        };
        close(restricted);
        close(tok);
        if ok == 0 {
            eprintln!("CreateProcessAsUserW failed: {}", last_error());
            return 4;
        }
        wait_child(&pi, args.num("timeout-s", 300))
    }

    /// Runs the command as a local user via the secondary logon service.
    fn spawn_as(args: &Args) -> i32 {
        let user = wide(&args.str("user", ""));
        let pass = wide(&args.str("password", ""));
        let domain = wide(".");
        let mut cmd = wide(&command_line(&args.rest));
        let si = startup();
        // SAFETY: plain data, filled by the call.
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: all strings are NUL terminated and `cmd` is writable.
        let ok = unsafe {
            CreateProcessWithLogonW(
                user.as_ptr(),
                domain.as_ptr(),
                pass.as_ptr(),
                LOGON_WITH_PROFILE,
                null(),
                cmd.as_mut_ptr(),
                CREATE_NO_WINDOW,
                null(),
                null(),
                &si,
                &mut pi,
            )
        };
        if ok == 0 {
            eprintln!("CreateProcessWithLogonW failed: {}", last_error());
            return 4;
        }
        wait_child(&pi, args.num("timeout-s", 300))
    }

    // ---------------------------------------------- stretch: FRN set (Q7)

    /// Creates `dirs` directories, `fanout` per parent, two levels deep.
    fn tree(args: &Args, out: &mut Out) {
        let root = PathBuf::from(args.str("dir", "."));
        let dirs = args.num("dirs", 1000) as usize;
        let fanout = args.num("fanout", 100) as usize;
        let t = Instant::now();
        let mut made = 0usize;
        let mut top = 0usize;
        while made < dirs {
            let a = root.join(format!("d{top}"));
            if fs::create_dir_all(&a).is_ok() {
                made += 1;
            }
            for j in 0..fanout.min(dirs - made.min(dirs)) {
                if fs::create_dir(a.join(format!("s{j}"))).is_ok() {
                    made += 1;
                }
            }
            top += 1;
        }
        out.row(json!({ "kind": "tree", "dirs": made, "seconds": t.elapsed().as_secs_f64() }));
    }

    fn private_bytes() -> u64 {
        // SAFETY: plain data, size set below.
        let mut c: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        // SAFETY: valid pseudo handle; `c` has the size the API expects.
        let ok = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                c.cb,
            )
        };
        if ok == 0 { 0 } else { c.PrivateUsage as u64 }
    }

    /// Builds the directory FRN set by a tree walk and checks that the parent
    /// of every record for files created in random directories is in it.
    fn frnset(args: &Args, out: &mut Out) {
        let root = PathBuf::from(args.str("dir", "."));
        let volume = args.str("volume", "C:");
        let variant = args.str("handle", "guid-dir");
        let fsctl = args.str("fsctl", "read-unpriv");
        let before = private_bytes();
        let t = Instant::now();
        let mut set: HashSet<u128> = HashSet::new();
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            set.insert(dir_frn(&d));
            dirs.push(d.clone());
            if let Ok(rd) = fs::read_dir(&d) {
                for e in rd.flatten() {
                    if e.file_type().is_ok_and(|t| t.is_dir()) {
                        stack.push(e.path());
                    }
                }
            }
        }
        let build_s = t.elapsed().as_secs_f64();
        let after = private_bytes();
        let mut row = base_row(args);
        row.insert("kind".into(), json!("frnset"));
        row.insert("dirs".into(), json!(dirs.len()));
        row.insert("build_seconds".into(), json!(build_s));
        row.insert(
            "private_bytes_delta".into(),
            json!(after.saturating_sub(before)),
        );
        row.insert("set_bytes_estimate".into(), json!(set.capacity() * 17));
        let access = parse_access("default", &variant);
        let code = if fsctl == "read" {
            FSCTL_READ_USN_JOURNAL
        } else {
            FSCTL_READ_UNPRIVILEGED_USN_JOURNAL
        };
        let start = load_start(args.get("start"));
        match open_volume(&variant, &volume, access, false) {
            Err((e, _)) => {
                row.insert("open_error".into(), json!(e));
            }
            Ok(vol) => {
                // Journal position now, then create files in 1,000 spread directories.
                let step = (dirs.len() / 1000).max(1);
                let mut made = 0usize;
                let tag = format!("q7-{}", std::process::id());
                for d in dirs.iter().step_by(step) {
                    if fs::write(d.join(&tag), b"x").is_ok() {
                        made += 1;
                    }
                }
                let mut usn = start.journal.next;
                let mut buf = vec![0u8; 64 * 1024];
                let (mut recs_total, mut in_set, mut tagged, mut tagged_in_set) =
                    (0u64, 0u64, 0u64, 0u64);
                let mut err = None;
                for _ in 0..2000 {
                    let input = read_input(usn, 0, 0, start.journal.id);
                    match ioctl(vol.handle, code, &input, &mut buf) {
                        Ok(n) => {
                            let data = buf.get(..n as usize).unwrap_or(&[]);
                            let recs = parse_records(data, 8);
                            for r in &recs {
                                recs_total += 1;
                                if set.contains(&r.parent) {
                                    in_set += 1;
                                }
                                if r.name == tag {
                                    tagged += 1;
                                    if set.contains(&r.parent) {
                                        tagged_in_set += 1;
                                    }
                                }
                            }
                            let next = u64le(data, 0) as i64;
                            if recs.is_empty() || next <= usn {
                                break;
                            }
                            usn = next;
                        }
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    }
                }
                for d in dirs.iter().step_by(step) {
                    let _ = fs::remove_file(d.join(&tag));
                }
                close(vol.handle);
                row.insert("files_created".into(), json!(made));
                row.insert("records".into(), json!(recs_total));
                row.insert("parent_in_set".into(), json!(in_set));
                row.insert("tagged_records".into(), json!(tagged));
                row.insert("tagged_parent_in_set".into(), json!(tagged_in_set));
                row.insert("read_error".into(), json!(err));
            }
        }
        out.row(Value::Object(row));
    }

    pub fn main() {
        let args = Args::parse();
        let mut out = Out::open(&args);
        match args.cmd.as_str() {
            "attempt-all" => attempt_all(&args, &mut out),
            "watch" => watch(&args, &mut out),
            "overlapped" => overlapped(&args, &mut out),
            "workload" => match args.get("mode").unwrap_or("run") {
                "prepare" => workload_prepare(&args, &mut out),
                _ => workload_run(&args, &mut out),
            },
            "tree" => tree(&args, &mut out),
            "frnset" => frnset(&args, &mut out),
            "spawn-restricted" => std::process::exit(spawn_restricted(&args)),
            "spawn-as" => std::process::exit(spawn_as(&args)),
            "info" => {
                let mut r = base_row(&args);
                r.insert("kind".into(), json!("info"));
                out.row(Value::Object(r));
            }
            other => {
                eprintln!("unknown command {other:?}");
                std::process::exit(64);
            }
        }
        let _ = (GENERIC_WRITE, CREATE_NEW, FILE_ATTRIBUTE_NORMAL);
    }
}
