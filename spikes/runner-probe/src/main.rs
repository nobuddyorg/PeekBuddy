//! Spike #34: probes what a GitHub-hosted Windows runner allows.
//!
//! Prints one JSON line per check: `{"check":..,"label":..,"ok":..,"detail":..}`.
//! Exits 0 even when checks fail: the probe reports, the workflow decides.

use std::time::Instant;

fn emit(check: &str, ok: bool, detail: &str) {
    let label = std::env::var("PROBE_LABEL").unwrap_or_else(|_| "local".to_owned());
    println!(
        "{{\"check\":\"{}\",\"label\":\"{}\",\"ok\":{},\"detail\":\"{}\"}}",
        escape(check),
        escape(&label),
        ok,
        escape(detail)
    );
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Question 13: noise baseline. A fixed CPU-bound loop, 30 samples.
fn noise_baseline() {
    let mut samples = Vec::with_capacity(30);
    for _ in 0..30 {
        let started = Instant::now();
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for i in 0..5_000_000u64 {
            x = x.rotate_left(5) ^ i.wrapping_mul(0x2545_F491_4F6C_DD1D);
        }
        std::hint::black_box(x);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let var = samples.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let cv = var.sqrt() / mean * 100.0;
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    emit(
        "noise.cpu_loop",
        true,
        &format!(
            "n=30 mean_ms={mean:.2} median_ms={:.2} min_ms={:.2} max_ms={:.2} cv_pct={cv:.2}",
            sorted[15], sorted[0], sorted[29]
        ),
    );
}

fn main() {
    #[cfg(windows)]
    win::run();
    #[cfg(not(windows))]
    emit("platform", false, "not windows");
    noise_baseline();
}

#[cfg(windows)]
mod win {
    use super::emit;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Security::*;
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Pipes::*;
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows_sys::Win32::System::StationsAndDesktops::*;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::UI::Shell::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn run() {
        session();
        desktop();
        let hwnd = window_class_and_hidden_window();
        tray(hwnd);
        visible_window();
        pipe();
        token();
        restricted_child();
        if !hwnd.is_null() {
            // SAFETY: hwnd was created by this thread and is destroyed once.
            unsafe { DestroyWindow(hwnd) };
        }
    }

    fn session() {
        let mut id = 0u32;
        // SAFETY: valid out pointer.
        let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut id) } != 0;
        emit("session.id", ok && id != 0, &format!("session_id={id}"));
    }

    fn desktop() {
        // SAFETY: plain call; the handle is closed below.
        let desk = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
        if desk.is_null() {
            // SAFETY: plain call.
            let err = unsafe { GetLastError() };
            emit("desktop.input_visible", false, &format!("OpenInputDesktop failed: {err}"));
            return;
        }
        let mut flags = USEROBJECTFLAGS { fInherit: 0, fReserved: 0, dwFlags: 0 };
        let mut needed = 0u32;
        // SAFETY: buffer is a USEROBJECTFLAGS of the passed size.
        let ok = unsafe {
            GetUserObjectInformationW(
                desk,
                UOI_FLAGS,
                (&raw mut flags).cast(),
                size_of::<USEROBJECTFLAGS>() as u32,
                &mut needed,
            )
        } != 0;
        let visible = flags.dwFlags & (WSF_VISIBLE as u32) != 0;
        emit(
            "desktop.input_visible",
            ok && visible,
            &format!("dwFlags={:#x}", flags.dwFlags),
        );
        // SAFETY: desk came from OpenInputDesktop.
        unsafe { CloseDesktop(desk) };
    }

    extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        // SAFETY: forwarding the arguments we were given.
        unsafe { DefWindowProcW(hwnd, msg, w, l) }
    }

    fn window_class_and_hidden_window() -> HWND {
        let class = wide("PeekBuddyRunnerProbe");
        // SAFETY: null module name means this exe.
        let hinstance = unsafe { GetModuleHandleW(null()) };
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        // SAFETY: wc points to valid data for the call.
        let atom = unsafe { RegisterClassW(&wc) };
        let title = wide("PeekBuddy probe (hidden)");
        // SAFETY: class registered above; strings are NUL-terminated.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                400,
                300,
                null_mut(),
                null_mut(),
                hinstance,
                null(),
            )
        };
        emit(
            "window.hidden_create",
            atom != 0 && !hwnd.is_null(),
            &format!("atom={atom}"),
        );
        let name = wide("TaskbarCreated");
        // SAFETY: NUL-terminated string.
        let msg = unsafe { RegisterWindowMessageW(name.as_ptr()) };
        emit("tray.taskbar_created_registered", msg != 0, &format!("msg={msg:#x}"));
        hwnd
    }

    fn tray(hwnd: HWND) {
        if hwnd.is_null() {
            emit("tray.nim_add", false, "no window");
            return;
        }
        // SAFETY: NOTIFYICONDATAW is plain data; zeroed is a valid start.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_APP + 1;
        for (slot, c) in data.szTip.iter_mut().zip("PeekBuddy probe".encode_utf16()) {
            *slot = c;
        }
        // SAFETY: data is fully initialised for NIM_ADD.
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
        // SAFETY: plain call.
        let err = if added { 0 } else { unsafe { GetLastError() } };
        emit("tray.nim_add", added, &format!("last_error={err}"));
        if added {
            // SAFETY: same icon id as added above.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
        }
    }

    fn visible_window() {
        let class = wide("PeekBuddyRunnerProbe");
        let title = wide("PeekBuddy probe (visible)");
        // SAFETY: class registered earlier in this process.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                400,
                300,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        if hwnd.is_null() {
            emit("window.visible_create", false, "CreateWindowExW failed");
            return;
        }
        // SAFETY: hwnd is a live window owned by this thread.
        unsafe { ShowWindow(hwnd, SW_SHOW) };
        // SAFETY: same window.
        let fg = unsafe { SetForegroundWindow(hwnd) } != 0;
        // SAFETY: same window.
        let visible = unsafe { IsWindowVisible(hwnd) } != 0;
        emit("window.visible_create", visible, "ShowWindow(SW_SHOW)");
        emit("window.set_foreground", fg, "");
        // SAFETY: destroyed once.
        unsafe { DestroyWindow(hwnd) };
    }

    fn pipe() {
        let name = wide(r"\\.\pipe\PeekBuddyRunnerProbe");
        // SAFETY: NUL-terminated name; default security.
        let server = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE,
                1,
                4096,
                4096,
                0,
                null(),
            )
        };
        if server == INVALID_HANDLE_VALUE {
            // SAFETY: plain call.
            let err = unsafe { GetLastError() };
            emit("pipe.first_instance", false, &format!("CreateNamedPipeW: {err}"));
            return;
        }
        // A second first-instance must fail.
        // SAFETY: as above.
        let second = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE,
                1,
                4096,
                4096,
                0,
                null(),
            )
        };
        emit(
            "pipe.first_instance",
            second == INVALID_HANDLE_VALUE,
            "second FIRST_PIPE_INSTANCE rejected",
        );
        let client_name = name.clone();
        let client = std::thread::spawn(move || {
            // SAFETY: NUL-terminated name.
            let h = unsafe {
                CreateFileW(
                    client_name.as_ptr(),
                    GENERIC_READ,
                    0,
                    null(),
                    OPEN_EXISTING,
                    0,
                    null_mut(),
                )
            };
            let ok = h != INVALID_HANDLE_VALUE;
            if ok {
                // SAFETY: valid handle.
                unsafe { CloseHandle(h) };
            }
            ok
        });
        // SAFETY: synchronous connect on our server handle.
        let connected = unsafe { ConnectNamedPipe(server, null_mut()) } != 0
            // SAFETY: plain call.
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        let client_ok = client.join().unwrap_or(false);
        emit("pipe.connect", connected && client_ok, "");
        // SAFETY: valid handle.
        unsafe { CloseHandle(server) };
    }

    fn process_token() -> HANDLE {
        let mut token: HANDLE = null_mut();
        // SAFETY: pseudo-handle of this process; valid out pointer.
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
                &mut token,
            )
        };
        token
    }

    fn token() {
        let token = process_token();
        let mut kind = 0i32;
        let mut len = 0u32;
        // SAFETY: buffer is an i32 of the passed size.
        unsafe {
            GetTokenInformation(
                token,
                TokenElevationType,
                (&raw mut kind).cast(),
                4,
                &mut len,
            )
        };
        let mut buf = [0u8; 256];
        // SAFETY: buffer of the passed size.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut len,
            )
        } != 0;
        let mut rid = 0u32;
        if ok {
            // SAFETY: on success buf holds a TOKEN_MANDATORY_LABEL.
            unsafe {
                let label = &*buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
                let count = *GetSidSubAuthorityCount(label.Label.Sid);
                rid = *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1);
            }
        }
        // 1 default, 2 full (elevated), 3 limited.
        emit(
            "token.elevation_and_integrity",
            ok,
            &format!("elevation_type={kind} integrity_rid={rid:#x}"),
        );
        // SAFETY: valid handle.
        unsafe { CloseHandle(token) };
    }

    fn restricted_child() {
        let token = process_token();
        let mut restricted: HANDLE = null_mut();
        // SAFETY: valid token; no SID lists.
        let ok = unsafe {
            CreateRestrictedToken(
                token,
                DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
                0,
                null(),
                0,
                null(),
                0,
                null(),
                &mut restricted,
            )
        } != 0;
        if !ok {
            // SAFETY: plain call.
            let err = unsafe { GetLastError() };
            emit("token.restricted_child", false, &format!("CreateRestrictedToken: {err}"));
            return;
        }
        let mut cmd = wide("cmd.exe /c exit 7");
        // SAFETY: STARTUPINFOW / PROCESS_INFORMATION are plain data.
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = size_of::<STARTUPINFOW>() as u32;
        // SAFETY: as above.
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: restricted is a primary token derived from ours; cmd is mutable and NUL-terminated.
        let started = unsafe {
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
        } != 0;
        if !started {
            // SAFETY: plain call.
            let err = unsafe { GetLastError() };
            emit("token.restricted_child", false, &format!("CreateProcessAsUserW: {err}"));
            return;
        }
        let mut code = 0u32;
        // SAFETY: valid process handle.
        unsafe {
            WaitForSingleObject(pi.hProcess, 10_000);
            GetExitCodeProcess(pi.hProcess, &mut code);
            CloseHandle(pi.hProcess);
            CloseHandle(pi.hThread);
            CloseHandle(restricted);
            CloseHandle(token);
        }
        emit("token.restricted_child", code == 7, &format!("exit_code={code}"));
    }
}
