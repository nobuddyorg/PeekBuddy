//! The real file system and clock on Windows.
//!
//! Operations use `std::fs`, which passes long paths (> 260) as `\\?\`
//! paths and keeps lone surrogates (F17) through `OsString::from_wide`.
//! What std lacks (junctions, the archive attribute, volume facts, the
//! high-resolution timer) comes from `peekbuddy-win`.

use std::ffi::OsString;
use std::io::Write as _;
use std::os::windows::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use peekbuddy_win::fs::{create_junction, file_system_name, short_names_generated, toggle_archive};
use peekbuddy_win::timer::HighResTimer;

use super::{Clock, Target, filetime};
use crate::ledger::Header;
use crate::ops::{Op, OpKind, WidePath};

/// Bytes written by write and append operations.
const PAYLOAD: &[u8] = b"PeekBuddy churn\r\n";

/// `ERROR_INVALID_PARAMETER`, for an error without an OS code (an
/// operation with missing operands).
const INVALID_PARAMETER: u32 = 87;

/// Runs operations below `root`.
#[derive(Debug)]
pub struct WinTarget {
    root: PathBuf,
    /// Seconds added to the base time of each timestamp change, so every
    /// change sets a new value.
    times_set: u64,
}

impl WinTarget {
    /// A target for the churn root `root` (the directory holding `t<n>`).
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            times_set: 0,
        }
    }

    fn path(&self, relative: &WidePath) -> PathBuf {
        self.root.join(OsString::from_wide(relative))
    }

    fn operand(&self, op: &Op, index: usize) -> std::io::Result<PathBuf> {
        op.paths
            .get(index)
            .map(|relative| self.path(relative))
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    }

    fn run(&mut self, op: &Op) -> std::io::Result<()> {
        let first = self.operand(op, 0)?;
        let open = |append: bool| {
            std::fs::OpenOptions::new()
                .write(!append)
                .append(append)
                .open(&first)
        };
        match op.kind {
            OpKind::CreateFile => std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&first)
                .map(drop),
            OpKind::CreateDir => std::fs::create_dir(&first),
            OpKind::Write => open(false)?.write_all(PAYLOAD),
            OpKind::Append => open(true)?.write_all(PAYLOAD),
            OpKind::Truncate => {
                // Truncating an empty file changes nothing and produces no
                // record, so an empty file gets one byte first.
                let mut file = open(false)?;
                if file.metadata()?.len() == 0 {
                    file.write_all(b"x")?;
                }
                file.set_len(0)
            }
            OpKind::SetSize => {
                let file = open(false)?;
                let len = file.metadata()?.len();
                file.set_len(len + 4096)
            }
            OpKind::SetAttributes => toggle_archive(&first),
            OpKind::SetTimes => {
                self.times_set += 1;
                // 2001-09-09 plus one second per change.
                let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
                open(false)?.set_modified(base + Duration::from_secs(self.times_set))
            }
            OpKind::Rename | OpKind::Move | OpKind::CaseRename => {
                std::fs::rename(&first, self.operand(op, 1)?)
            }
            OpKind::RenameChain => {
                let middle = self.operand(op, 1)?;
                std::fs::rename(&first, &middle)?;
                std::fs::rename(&middle, self.operand(op, 2)?)
            }
            OpKind::Delete | OpKind::DeleteLink => std::fs::remove_file(&first),
            OpKind::DeleteTree => std::fs::remove_dir_all(&first),
            OpKind::HardLink => std::fs::hard_link(&first, self.operand(op, 1)?),
            OpKind::Junction => create_junction(&first, &self.operand(op, 1)?),
        }
    }
}

impl Target for WinTarget {
    fn apply(&mut self, op: &Op) -> u32 {
        match self.run(op) {
            Ok(()) => 0,
            Err(error) => error
                .raw_os_error()
                .and_then(|code| u32::try_from(code).ok())
                .unwrap_or(INVALID_PARAMETER),
        }
    }
}

/// `GetSystemTimePreciseAsFileTime` (std's `SystemTime::now` on Windows)
/// and a high-resolution waitable timer, with `std::thread::sleep` as the
/// fallback where the timer is unavailable.
#[derive(Debug)]
pub struct WinClock {
    timer: Option<HighResTimer>,
}

impl WinClock {
    /// A clock for the calling thread.
    #[must_use]
    pub fn new() -> Self {
        Self {
            timer: HighResTimer::new().ok(),
        }
    }

    /// Whether sleeps use the high-resolution timer.
    #[must_use]
    pub fn high_resolution(&self) -> bool {
        self.timer.is_some()
    }
}

impl Default for WinClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for WinClock {
    fn now(&mut self) -> u64 {
        filetime(SystemTime::now())
    }

    fn sleep(&mut self, ticks: u64) {
        let slept = self.timer.as_ref().is_some_and(|t| t.sleep(ticks).is_ok());
        if !slept {
            std::thread::sleep(Duration::from_nanos(ticks.saturating_mul(100)));
        }
    }
}

/// The ledger header for a run in `root`: OS build, CPU count, file system,
/// Defender real-time state and 8.3 short-name generation. Facts that
/// cannot be read are `None` or `"unknown"`.
#[must_use]
pub fn header(profile: &str, seed: u64, threads: u32, root: &Path, runner: &str) -> Header {
    // The 8.3 probe file goes next to the root, not into it, so a watcher
    // on the root never sees it.
    let probe_dir = root.parent().unwrap_or(root);
    Header {
        version: 1,
        profile: profile.to_owned(),
        seed,
        threads,
        os_build: os_build().unwrap_or_else(|| "unknown".to_owned()),
        runner: runner.to_owned(),
        cpus: std::thread::available_parallelism()
            .ok()
            .and_then(|n| u32::try_from(n.get()).ok())
            .unwrap_or(0),
        file_system: file_system_name(root).unwrap_or_else(|_| "unknown".to_owned()),
        defender_realtime: defender_realtime(),
        short_names: short_names_generated(probe_dir).ok(),
    }
}

/// `10.0.26100.6584` from `cmd /c ver` (`Microsoft Windows [Version
/// 10.0.26100.6584]`), which includes the update revision.
fn os_build() -> Option<String> {
    let output = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "ver"])
        .output()
        .ok()?;
    parse_ver(&String::from_utf8_lossy(&output.stdout))
}

/// The version number inside the brackets of `ver` output.
#[must_use]
pub fn parse_ver(text: &str) -> Option<String> {
    let inside = text.split('[').nth(1)?.split(']').next()?;
    let version = inside.split_whitespace().last()?;
    version
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.')
        .then(|| version.to_owned())
}

/// `(Get-MpComputerStatus).RealTimeProtectionEnabled`; `None` when Defender
/// is not installed or the query fails.
fn defender_realtime() -> Option<bool> {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-MpComputerStatus).RealTimeProtectionEnabled",
        ])
        .output()
        .ok()?;
    match String::from_utf8_lossy(&output.stdout).trim() {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_ver;

    #[test]
    fn ver_output() {
        assert_eq!(
            parse_ver("\r\nMicrosoft Windows [Version 10.0.26100.6584]\r\n").as_deref(),
            Some("10.0.26100.6584")
        );
        assert_eq!(
            parse_ver("Microsoft Windows [Versión 10.0.22631.4890]").as_deref(),
            Some("10.0.22631.4890")
        );
        assert_eq!(parse_ver("no brackets"), None);
        assert_eq!(parse_ver("[Version x.y]"), None);
    }
}
