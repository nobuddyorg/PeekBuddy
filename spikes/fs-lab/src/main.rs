//! Spike #36: which RDCW (Full information class) and USN records does each
//! file-system edge case produce?
//!
//! `fs-lab run --scenario N1 --dir X:\fslab --volume X: [--reps 3]
//!  [--settle-s 3] --out fixtures.jsonl [--label L]`
//!
//! Every repetition creates a fresh directory under `--dir`, watches it with
//! `ReadDirectoryChangesExW` (Full class, subtree) and tails the USN journal of
//! the volume (administrator), runs the scenario, waits for the records to
//! settle and appends one JSON object per record. Names are kept as UTF-16
//! arrays next to a lossy string. File data is never read.

#[cfg(not(windows))]
fn main() {
    eprintln!("fs-lab runs on Windows only");
}

#[cfg(windows)]
fn main() {
    imp::main();
}

#[cfg(windows)]
mod imp {
    use std::ffi::{OsStr, OsString, c_void};
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Write as _};
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
    use std::os::windows::io::AsRawHandle as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr::{null, null_mut};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Sender, channel};
    use std::thread;
    use std::time::Duration;

    use peekbuddy_core::notify::{self, Class};
    use serde_json::{Value, json};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetShortPathNameW, OPEN_EXISTING, ReadDirectoryChangesExW,
        ReadDirectoryNotifyFullInformation,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED};
    use windows_sys::Win32::System::Ioctl::{
        DUPLICATE_EXTENTS_DATA, FSCTL_DUPLICATE_EXTENTS_TO_FILE, FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
    };
    use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OSVERSIONINFOW) -> i32;
    }

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_LIST_DIRECTORY: u32 = 0x1;
    const NOTIFY_ALL: u32 = 0x1FF;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    fn qpc() -> i64 {
        let mut t = 0i64;
        // SAFETY: valid out pointer.
        unsafe { QueryPerformanceCounter(&mut t) };
        t
    }

    fn freq() -> i64 {
        let mut f = 1i64;
        // SAFETY: valid out pointer.
        unsafe { QueryPerformanceFrequency(&mut f) };
        f
    }

    fn os_build() -> u32 {
        // SAFETY: plain data; size set as the API requires.
        let mut v: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
        v.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
        // SAFETY: valid pointer to an initialised structure.
        unsafe { RtlGetVersion(&mut v) };
        v.dwBuildNumber
    }

    fn u16le(b: &[u8], o: usize) -> u16 {
        b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32le(b: &[u8], o: usize) -> u32 {
        b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
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

    fn lossy(units: &[u16]) -> String {
        String::from_utf16_lossy(units)
    }

    // ------------------------------------------------------------ capture

    fn open_dir(path: &Path, overlapped: bool) -> io::Result<HANDLE> {
        let w = wide(path.as_os_str());
        let flags = FILE_FLAG_BACKUP_SEMANTICS | if overlapped { FILE_FLAG_OVERLAPPED } else { 0 };
        // SAFETY: NUL terminated path, no security attributes or template.
        let h = unsafe {
            CreateFileW(w.as_ptr(), FILE_LIST_DIRECTORY, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, null(), OPEN_EXISTING, flags, null_mut())
        };
        if h == INVALID_HANDLE_VALUE { Err(io::Error::last_os_error()) } else { Ok(h) }
    }

    struct SendHandle(HANDLE);
    // SAFETY: a kernel handle is usable from any thread.
    unsafe impl Send for SendHandle {}

    fn rdcw_thread(root: PathBuf, stop: Arc<AtomicBool>, tx: Sender<Value>, ready: Sender<()>) {
        let h = match open_dir(&root, true) {
            Ok(h) => SendHandle(h),
            Err(e) => {
                let _ = tx.send(json!({ "src": "error", "what": "rdcw open", "error": e.to_string() }));
                let _ = ready.send(());
                return;
            }
        };
        // SAFETY: manual-reset, unsignalled event with no name.
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        let mut buf = vec![0u64; 32 * 1024]; // 256 KiB, 8-byte aligned
        let mut first = true;
        loop {
            // SAFETY: plain data.
            let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
            ov.hEvent = event;
            // SAFETY: valid event handle.
            unsafe { ResetEvent(event) };
            let mut n = 0u32;
            // SAFETY: `buf` and `ov` stay alive until the request has completed
            // or been cancelled and waited for below.
            let ok = unsafe {
                ReadDirectoryChangesExW(h.0, buf.as_mut_ptr().cast::<c_void>(), (buf.len() * 8) as u32, 1, NOTIFY_ALL, &mut n, &mut ov, None, ReadDirectoryNotifyFullInformation)
            };
            if ok == 0 {
                let _ = tx.send(json!({ "src": "error", "what": "ReadDirectoryChangesExW", "error": io::Error::last_os_error().to_string() }));
                if first {
                    let _ = ready.send(());
                }
                break;
            }
            if first {
                first = false;
                let _ = ready.send(());
            }
            let cancelled = loop {
                // SAFETY: valid event handle.
                if unsafe { WaitForSingleObject(event, 50) } == 0 {
                    break false;
                }
                if stop.load(Ordering::Relaxed) {
                    // SAFETY: cancels this handle's pending request.
                    unsafe { CancelIoEx(h.0, &ov) };
                    // SAFETY: valid event handle; the request is completing.
                    unsafe { WaitForSingleObject(event, 5000) };
                    break true;
                }
            };
            // SAFETY: the request has completed.
            let done = unsafe { GetOverlappedResult(h.0, &ov, &mut n, 0) };
            let at = qpc();
            if cancelled || stop.load(Ordering::Relaxed) && done == 0 {
                break;
            }
            if done == 0 {
                let _ = tx.send(json!({ "src": "error", "what": "GetOverlappedResult", "error": io::Error::last_os_error().to_string() }));
                break;
            }
            if n == 0 {
                let _ = tx.send(json!({ "src": "rdcw", "t": at, "overflow": true }));
                continue;
            }
            // SAFETY: the buffer holds `n` valid bytes written by the kernel.
            let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), n as usize) };
            for rec in notify::parse(bytes, n as usize, Class::Full) {
                match rec {
                    Ok(r) => {
                        let units: Vec<u16> = r.name.units().collect();
                        let mut row = json!({ "src": "rdcw", "t": at, "action": format!("{:?}", r.action), "name_u16": units, "name": lossy(&units) });
                        if let Some(e) = r.extended {
                            row["attributes"] = json!(e.attributes);
                            row["reparse_tag"] = json!(e.reparse_tag);
                            row["file_id"] = json!(e.file_id);
                            row["parent_file_id"] = json!(e.parent_file_id);
                            row["name_flags"] = json!(e.name_flags);
                            row["file_size"] = json!(e.file_size);
                            row["allocated_length"] = json!(e.allocated_length);
                        }
                        let _ = tx.send(row);
                    }
                    Err(e) => {
                        let _ = tx.send(json!({ "src": "error", "what": "parse", "error": e.to_string() }));
                    }
                }
            }
        }
        // SAFETY: handles opened above.
        unsafe {
            CloseHandle(event);
            CloseHandle(h.0);
        }
    }

    fn usn_thread(volume: String, stop: Arc<AtomicBool>, tx: Sender<Value>, ready: Sender<()>) {
        let w = wide(OsStr::new(&format!("\\\\.\\{}", volume.trim_end_matches('\\'))));
        // SAFETY: NUL terminated path.
        let h = unsafe {
            CreateFileW(w.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, null(), OPEN_EXISTING, 0, null_mut())
        };
        if h == INVALID_HANDLE_VALUE {
            let _ = tx.send(json!({ "src": "error", "what": "usn open", "error": io::Error::last_os_error().to_string() }));
            let _ = ready.send(());
            return;
        }
        let mut q = vec![0u8; 256];
        let mut n = 0u32;
        // SAFETY: buffers valid for the lengths passed.
        let ok = unsafe { DeviceIoControl(h, FSCTL_QUERY_USN_JOURNAL, null(), 0, q.as_mut_ptr().cast(), 256, &mut n, null_mut()) };
        if ok == 0 {
            let _ = tx.send(json!({ "src": "error", "what": "usn query", "error": io::Error::last_os_error().to_string() }));
            let _ = ready.send(());
            // SAFETY: handle opened above.
            unsafe { CloseHandle(h) };
            return;
        }
        let journal_id = u64le(&q, 0);
        let mut usn = u64le(&q, 16) as i64;
        let mut buf = vec![0u8; 64 * 1024];
        let _ = ready.send(());
        while !stop.load(Ordering::Relaxed) {
            let mut input = vec![0u8; 48];
            input[0..8].copy_from_slice(&usn.to_le_bytes());
            input[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
            input[16..24].copy_from_slice(&1u64.to_le_bytes()); // timeout 1 s
            input[24..32].copy_from_slice(&1u64.to_le_bytes()); // wait for 1 byte
            input[32..40].copy_from_slice(&journal_id.to_le_bytes());
            input[40..42].copy_from_slice(&2u16.to_le_bytes());
            input[42..44].copy_from_slice(&3u16.to_le_bytes());
            let mut got = 0u32;
            // SAFETY: buffers valid for the lengths passed; synchronous call.
            let ok = unsafe { DeviceIoControl(h, FSCTL_READ_USN_JOURNAL, input.as_ptr().cast(), 48, buf.as_mut_ptr().cast(), buf.len() as u32, &mut got, null_mut()) };
            let at = qpc();
            if ok == 0 {
                let _ = tx.send(json!({ "src": "error", "what": "usn read", "error": io::Error::last_os_error().to_string() }));
                break;
            }
            let data = buf.get(..got as usize).unwrap_or(&[]);
            let next = u64le(data, 0) as i64;
            let mut off = 8usize;
            while off + 8 <= data.len() {
                let len = u32le(data, off) as usize;
                if len < 8 || off + len > data.len() {
                    break;
                }
                let major = u16le(data, off + 4);
                let (frn, parent, usn_v, reason, source, attrs, nlen, noff) = match major {
                    2 => (u64le(data, off + 8) as u128, u64le(data, off + 16) as u128, u64le(data, off + 24), u32le(data, off + 40), u32le(data, off + 44), u32le(data, off + 52), u16le(data, off + 56), u16le(data, off + 58)),
                    3 => (u128le(data, off + 8), u128le(data, off + 24), u64le(data, off + 40), u32le(data, off + 56), u32le(data, off + 60), u32le(data, off + 68), u16le(data, off + 72), u16le(data, off + 74)),
                    _ => (0, 0, 0, 0, 0, 0, 0, 0),
                };
                let start = off + noff as usize;
                let units: Vec<u16> = data
                    .get(start..start + nlen as usize)
                    .unwrap_or(&[])
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let _ = tx.send(json!({
                    "src": "usn", "t": at, "version": major, "frn": format!("{frn:x}"), "parent": format!("{parent:x}"),
                    "usn": usn_v, "reason": reason, "source_info": source, "attributes": attrs,
                    "name_u16": units, "name": lossy(&units),
                }));
                off += (len + 7) & !7;
            }
            if next > usn {
                usn = next;
            }
            if got <= 8 {
                thread::sleep(Duration::from_millis(5));
            }
        }
        // SAFETY: handle opened above.
        unsafe { CloseHandle(h) };
    }

    // ----------------------------------------------------------- scenarios

    struct Ctx {
        root: PathBuf,
        volume: String,
        tx: Sender<Value>,
    }

    impl Ctx {
        fn op(&self, label: &str) {
            let _ = self.tx.send(json!({ "src": "op", "t": qpc(), "label": label }));
        }
        fn p(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }
        fn note(&self, what: &str, value: Value) {
            let _ = self.tx.send(json!({ "src": "note", "what": what, "value": value }));
        }
        fn run(&self, program: &str, args: &[&str]) -> String {
            let out = Command::new(program).args(args).output();
            let text = match out {
                Ok(o) => format!("exit {:?}: {}{}", o.status.code(), String::from_utf8_lossy(&o.stdout).trim(), String::from_utf8_lossy(&o.stderr).trim()),
                Err(e) => format!("cannot run: {e}"),
            };
            self.note(&format!("{program} {}", args.join(" ")), json!(text));
            text
        }
    }

    fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut f = OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
        f.write_all(bytes)
    }

    fn short_path(path: &Path) -> Option<PathBuf> {
        let w = wide(path.as_os_str());
        let mut buf = [0u16; 520];
        // SAFETY: NUL terminated input and a buffer of the length passed.
        let n = unsafe { GetShortPathNameW(w.as_ptr(), buf.as_mut_ptr(), 520) } as usize;
        (n > 0 && n < 520).then(|| PathBuf::from(OsString::from_wide(buf.get(..n).unwrap_or(&[]))))
    }

    fn n1(c: &Ctx) -> io::Result<()> {
        c.run("fsutil", &["8dot3name", "query", &c.volume]);
        c.run("fsutil", &["8dot3name", "set", &c.volume, "0"]);
        let long = c.p("A long file name for the short name test.txt");
        c.op("create long name");
        write(&long, b"one")?;
        thread::sleep(Duration::from_millis(300));
        let short = short_path(&long);
        c.note("short path", json!(short.as_ref().map(|p| p.display().to_string())));
        if let Some(short) = short {
            c.op("write through the short name");
            write(&short, b"two two")?;
            thread::sleep(Duration::from_millis(300));
            c.op("rename through the short name");
            fs::rename(&short, c.p("renamed-via-short.txt"))?;
        }
        thread::sleep(Duration::from_millis(300));
        c.op("delete");
        fs::remove_file(c.p("renamed-via-short.txt")).or_else(|_| fs::remove_file(&long))
    }

    fn n2(c: &Ctx) -> io::Result<()> {
        write(&c.p("a.txt"), b"x")?;
        thread::sleep(Duration::from_millis(300));
        c.op("case-only rename a.txt -> A.txt");
        fs::rename(c.p("a.txt"), c.p("A.txt"))?;
        thread::sleep(Duration::from_millis(300));
        c.op("case-only rename of a directory");
        fs::create_dir(c.p("dir"))?;
        thread::sleep(Duration::from_millis(300));
        fs::rename(c.p("dir"), c.p("DIR"))
    }

    fn n3(c: &Ctx) -> io::Result<()> {
        let d = c.p("cs");
        fs::create_dir(&d)?;
        c.op("enable per-directory case sensitivity");
        c.run("fsutil", &["file", "setCaseSensitiveInfo", &d.display().to_string(), "enable"]);
        thread::sleep(Duration::from_millis(300));
        c.op("create two names differing only by case");
        write(&d.join("file.txt"), b"lower")?;
        let upper = write(&d.join("FILE.txt"), b"upper");
        c.note("second name created", json!(upper.is_ok()));
        Ok(())
    }

    fn n4(c: &Ctx) -> io::Result<()> {
        let lone_high: Vec<u16> = vec![0x61, 0xD800, 0x62];
        let lone_low: Vec<u16> = vec![0x61, 0xDC00, 0x62];
        let a = c.root.join(OsString::from_wide(&lone_high));
        let b = c.root.join(OsString::from_wide(&lone_low));
        c.op("create name with a lone high surrogate");
        write(&a, b"x")?;
        c.op("create name with a lone low surrogate");
        write(&b, b"y")?;
        thread::sleep(Duration::from_millis(300));
        c.op("write the first");
        write(&a, b"xx")?;
        thread::sleep(Duration::from_millis(300));
        c.op("rename the first to a normal name");
        fs::rename(&a, c.p("normal.txt"))?;
        c.op("delete both");
        fs::remove_file(&b)?;
        fs::remove_file(c.p("normal.txt"))
    }

    fn l1(c: &Ctx) -> io::Result<()> {
        write(&c.p("orig.txt"), b"data")?;
        thread::sleep(Duration::from_millis(300));
        c.op("create hard link");
        fs::hard_link(c.p("orig.txt"), c.p("link.txt"))?;
        thread::sleep(Duration::from_millis(300));
        c.op("write through the link");
        write(&c.p("link.txt"), b"data data")?;
        thread::sleep(Duration::from_millis(300));
        c.op("write through the original");
        write(&c.p("orig.txt"), b"data data data")?;
        thread::sleep(Duration::from_millis(300));
        c.op("delete the link");
        fs::remove_file(c.p("link.txt"))?;
        c.op("delete the original");
        fs::remove_file(c.p("orig.txt"))
    }

    fn l2(c: &Ctx) -> io::Result<()> {
        let outside = c.root.with_file_name(format!("{}-target", c.root.file_name().and_then(|n| n.to_str()).unwrap_or("t")));
        let outside2 = c.root.with_file_name(format!("{}-target2", c.root.file_name().and_then(|n| n.to_str()).unwrap_or("t")));
        fs::create_dir_all(&outside)?;
        fs::create_dir_all(&outside2)?;
        let link = c.p("junction");
        c.op("create junction");
        c.run("cmd", &["/c", "mklink", "/J", &link.display().to_string(), &outside.display().to_string()]);
        thread::sleep(Duration::from_millis(300));
        c.op("create a file under the target through the junction");
        write(&link.join("through.txt"), b"x")?;
        thread::sleep(Duration::from_millis(300));
        c.op("create a file directly in the target");
        write(&outside.join("direct.txt"), b"y")?;
        thread::sleep(Duration::from_millis(300));
        c.op("retarget: remove the junction");
        fs::remove_dir(&link)?;
        c.op("retarget: create it again to the other target");
        c.run("cmd", &["/c", "mklink", "/J", &link.display().to_string(), &outside2.display().to_string()]);
        thread::sleep(Duration::from_millis(300));
        c.op("delete the junction");
        fs::remove_dir(&link)?;
        let _ = fs::remove_dir_all(&outside);
        let _ = fs::remove_dir_all(&outside2);
        Ok(())
    }

    fn l3(c: &Ctx) -> io::Result<()> {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        write(&c.p("target.txt"), b"x")?;
        fs::create_dir(c.p("targetdir"))?;
        thread::sleep(Duration::from_millis(300));
        c.op("create file symlink");
        let f = symlink_file(c.p("target.txt"), c.p("filelink"));
        c.note("file symlink", json!(f.as_ref().map_err(ToString::to_string).err()));
        thread::sleep(Duration::from_millis(300));
        c.op("create directory symlink");
        let d = symlink_dir(c.p("targetdir"), c.p("dirlink"));
        c.note("dir symlink", json!(d.as_ref().map_err(ToString::to_string).err()));
        thread::sleep(Duration::from_millis(300));
        c.op("write through the file symlink");
        let _ = write(&c.p("filelink"), b"xx");
        thread::sleep(Duration::from_millis(300));
        c.op("create a file through the directory symlink");
        let _ = write(&c.p("dirlink").join("inside.txt"), b"z");
        thread::sleep(Duration::from_millis(300));
        c.op("delete the symlinks");
        let _ = fs::remove_file(c.p("filelink"));
        let _ = fs::remove_dir(c.p("dirlink"));
        Ok(())
    }

    fn l4(c: &Ctx) -> io::Result<()> {
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let apps = Path::new(&local).join("Microsoft").join("WindowsApps");
        let mut found = Vec::new();
        if let Ok(rd) = fs::read_dir(&apps) {
            for e in rd.flatten().take(20) {
                let md = fs::symlink_metadata(e.path()).ok();
                use std::os::windows::fs::MetadataExt as _;
                found.push(json!({ "name": e.file_name().to_string_lossy(), "attributes": md.map(|m| m.file_attributes()) }));
            }
        }
        c.note("WindowsApps entries (a Store app update cannot be triggered on a hosted runner)", json!(found));
        Ok(())
    }

    fn s1(c: &Ctx) -> io::Result<()> {
        write(&c.p("file.txt"), b"main")?;
        thread::sleep(Duration::from_millis(300));
        c.op("create an alternate data stream");
        write(Path::new(&format!("{}:stream", c.p("file.txt").display())), b"stream data")?;
        thread::sleep(Duration::from_millis(300));
        c.op("append to the stream");
        let mut f = OpenOptions::new().append(true).open(format!("{}:stream", c.p("file.txt").display()))?;
        f.write_all(b" more")?;
        drop(f);
        thread::sleep(Duration::from_millis(300));
        c.op("delete the stream (file stays)");
        fs::remove_file(format!("{}:stream", c.p("file.txt").display()))?;
        Ok(())
    }

    fn d1(c: &Ctx) -> io::Result<()> {
        let src = c.p("big.bin");
        let size: u64 = 16 * 1024 * 1024;
        c.op("create a 16 MiB file");
        {
            let mut f = File::create(&src)?;
            let chunk = vec![0x5Au8; 1024 * 1024];
            for _ in 0..16 {
                f.write_all(&chunk)?;
            }
        }
        thread::sleep(Duration::from_millis(500));
        c.op("fs::copy (CopyFile2, clones on ReFS where supported)");
        fs::copy(&src, c.p("copy.bin"))?;
        thread::sleep(Duration::from_millis(500));
        c.op("explicit FSCTL_DUPLICATE_EXTENTS_TO_FILE");
        let target = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(c.p("clone.bin"))?;
        target.set_len(size)?;
        let source = File::open(&src)?;
        let data = DUPLICATE_EXTENTS_DATA { FileHandle: source.as_raw_handle(), SourceFileOffset: 0, TargetFileOffset: 0, ByteCount: size as i64 };
        let mut n = 0u32;
        // SAFETY: `data` is a valid DUPLICATE_EXTENTS_DATA and both handles outlive the call.
        let ok = unsafe { DeviceIoControl(target.as_raw_handle(), FSCTL_DUPLICATE_EXTENTS_TO_FILE, (&data as *const DUPLICATE_EXTENTS_DATA).cast(), size_of::<DUPLICATE_EXTENTS_DATA>() as u32, null_mut(), 0, &mut n, null_mut()) };
        c.note("duplicate extents", json!(if ok != 0 { "ok".to_owned() } else { io::Error::last_os_error().to_string() }));
        drop(target);
        thread::sleep(Duration::from_millis(500));
        c.op("write into the copy");
        write(&c.p("copy.bin"), b"changed")?;
        Ok(())
    }

    fn b1(c: &Ctx) -> io::Result<()> {
        // Nothing happens here on purpose: the settle time is the observation.
        c.op("idle: no user action");
        c.run("powershell", &["-NoProfile", "-Command", &format!("try {{ Start-MpScan -ScanType CustomScan -ScanPath '{}' ; 'scan done' }} catch {{ \"scan unavailable: $_\" }}", c.root.display())]);
        Ok(())
    }

    fn scenario(id: &str) -> Option<fn(&Ctx) -> io::Result<()>> {
        Some(match id {
            "N1" => n1,
            "N2" => n2,
            "N3" => n3,
            "N4" => n4,
            "L1" => l1,
            "L2" => l2,
            "L3" => l3,
            "L4" => l4,
            "S1" => s1,
            "D1" => d1,
            "B1" => b1,
            _ => return None,
        })
    }

    // ----------------------------------------------------------------- main

    fn arg(args: &[String], key: &str) -> Option<String> {
        args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) != Some("run") {
            eprintln!("usage: fs-lab run --scenario N1 --dir X:\\fslab --volume X: --out file.jsonl [--reps 3] [--settle-s 3] [--label L]");
            std::process::exit(64);
        }
        let id = arg(&args, "--scenario").unwrap_or_default();
        let Some(run) = scenario(&id) else {
            eprintln!("unknown scenario {id}");
            std::process::exit(64);
        };
        let base = PathBuf::from(arg(&args, "--dir").unwrap_or_else(|| ".".into()));
        let volume = arg(&args, "--volume").unwrap_or_else(|| "C:".into());
        let out = arg(&args, "--out").unwrap_or_else(|| format!("{id}.jsonl"));
        let reps: usize = arg(&args, "--reps").and_then(|s| s.parse().ok()).unwrap_or(3);
        let settle: u64 = arg(&args, "--settle-s").and_then(|s| s.parse().ok()).unwrap_or(3);
        let label = arg(&args, "--label").unwrap_or_default();
        let mut file = OpenOptions::new().create(true).append(true).open(&out).expect("output file");
        for rep in 0..reps {
            let root = base.join(format!("{id}-{rep}"));
            let _ = fs::remove_dir_all(&root);
            if let Err(e) = fs::create_dir_all(&root) {
                eprintln!("cannot create {}: {e}", root.display());
                continue;
            }
            let (tx, rx) = channel::<Value>();
            let stop = Arc::new(AtomicBool::new(false));
            let (ready_tx, ready_rx) = channel();
            let t1 = {
                let (root, stop, tx, ready) = (root.clone(), stop.clone(), tx.clone(), ready_tx.clone());
                thread::spawn(move || rdcw_thread(root, stop, tx, ready))
            };
            let t2 = {
                let (volume, stop, tx, ready) = (volume.clone(), stop.clone(), tx.clone(), ready_tx.clone());
                thread::spawn(move || usn_thread(volume, stop, tx, ready))
            };
            let _ = ready_rx.recv_timeout(Duration::from_secs(10));
            let _ = ready_rx.recv_timeout(Duration::from_secs(10));
            thread::sleep(Duration::from_millis(1500));
            let ctx = Ctx { root: root.clone(), volume: volume.clone(), tx: tx.clone() };
            let _ = tx.send(json!({ "src": "op", "t": qpc(), "label": "scenario start" }));
            let result = run(&ctx);
            let _ = tx.send(json!({ "src": "op", "t": qpc(), "label": "scenario end", "error": result.err().map(|e| e.to_string()) }));
            thread::sleep(Duration::from_secs(settle));
            stop.store(true, Ordering::Relaxed);
            let _ = t1.join();
            let _ = t2.join();
            drop(tx);
            drop(ctx);
            let header = json!({
                "src": "header", "scenario": id, "rep": rep, "label": label, "os_build": os_build(),
                "volume": volume, "root": root.display().to_string(), "rdcw_class": "Full", "qpc_freq": freq(), "settle_s": settle,
                "arch": std::env::consts::ARCH,
            });
            writeln!(file, "{header}").expect("write");
            let mut rows: Vec<Value> = rx.try_iter().collect();
            rows.sort_by_key(|r| r["t"].as_i64().unwrap_or(0));
            for row in rows {
                writeln!(file, "{row}").expect("write");
            }
            let _ = fs::remove_dir_all(&root);
        }
    }
}
